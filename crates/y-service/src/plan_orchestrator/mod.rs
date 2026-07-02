//! Plan orchestrator: handles the `Plan` tool call by delegating to
//! the `plan-writer` sub-agent, resolving the configured review policy, and
//! executing approved phases in child sessions.
//!
//! Follows the same pattern as `TaskDelegationOrchestrator` and
//! `ToolSearchOrchestrator` -- the `tool_dispatch` layer intercepts
//! `Plan` tool calls and routes them here.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_agent::orchestrator::dag::{DagError, TaskDag, TaskNode, TaskPriority};
use y_agent::AgentDefinition;
use y_core::agent::InheritedConstraints;
use y_core::provider::ResponseFormat;
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::{ToolError, ToolOutput};
use y_core::trust::TrustTier;
use y_core::types::{Message, SessionId};
use y_diagnostics::DiagnosticsEvent;
use y_guardrails::PlanReviewMode;

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError, AgentService};
use crate::chat::{TurnEvent, TurnEventSender};
use crate::chat_types::{OperationMode, PendingPlanReviews, PlanReviewDecision};
use crate::container::ServiceContainer;

const PLAN_CANCELLED_MESSAGE: &str = "Cancelled";
const PHASE_EXECUTOR_AGENT_ID: &str = "plan-phase-executor";
/// Default maximum number of phases to execute concurrently.
const DEFAULT_MAX_PARALLEL_PHASES: usize = 4;
/// Hard upper bound to protect against runaway concurrency from caller input.
const MAX_PARALLEL_PHASES_CEILING: usize = 16;
const MAX_PLAN_REVISIONS: usize = 5;
const PHASE_EXECUTOR_FALLBACK_ALLOWED_TOOLS: &[&str] = &[
    "ToolSearch",
    "FileRead",
    "FileWrite",
    "ShellExec",
    "WebFetch",
    "Browser",
    "Glob",
    "Grep",
];

/// Ambient marker that the current async task is executing the phases of an
/// already-approved plan. Scoped around phase execution so that any nested
/// `Plan` tool call auto-approves instead of requesting a second human review:
/// the top-level plan is the sole human approval gate.
///
/// Phases run inline within the same async task (no `tokio::spawn`), so the
/// task-local propagates to nested `Plan` dispatch and through `Task`
/// delegation. Mirrors `DELEGATION_INTERACTION_CTX` in `agent_service`.
mod plan_execution_ctx {
    tokio::task_local! {
        static IN_PLAN_EXECUTION: ();
    }

    /// Returns true when running inside an approved plan's phase execution.
    pub(super) fn is_active() -> bool {
        IN_PLAN_EXECUTION.try_with(|()| ()).is_ok()
    }

    /// Run `fut` with the plan-execution marker set.
    pub(super) async fn scoped<F: std::future::Future>(fut: F) -> F::Output {
        IN_PLAN_EXECUTION.scope((), fut).await
    }
}

// ---------------------------------------------------------------------------
// Plan data structures
// ---------------------------------------------------------------------------

/// Status of a single task in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    #[serde(alias = "blocked")]
    Pending,
    InProgress,
    Completed,
    Failed,
}

fn default_estimated_iterations() -> usize {
    15
}

/// A structured task extracted from the plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanTask {
    #[serde(alias = "task_id")]
    pub id: String,
    #[serde(default)]
    pub phase: usize,
    #[serde(alias = "label")]
    pub title: String,
    #[serde(default, alias = "objective")]
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default = "default_estimated_iterations")]
    #[serde(alias = "iterations")]
    pub estimated_iterations: usize,
    #[serde(default)]
    pub key_files: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// Structured plan output from the plan-writer agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredPlan {
    pub plan_title: String,
    #[serde(default)]
    pub plan_file: String,
    #[serde(default)]
    pub estimated_effort: String,
    #[serde(default)]
    pub overview: String,
    #[serde(default)]
    pub scope_in: Vec<String>,
    #[serde(default)]
    pub scope_out: Vec<String>,
    #[serde(default)]
    pub guardrails: Vec<String>,
    #[serde(default)]
    execution_contract: PlanExecutionContract,
    pub tasks: Vec<PlanTask>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct PlanExecutionContract {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    additional_read_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inherited_constraints: Option<InheritedConstraints>,
}

// ---------------------------------------------------------------------------
// Restored plan review (for cross-restart persistence)
// ---------------------------------------------------------------------------

pub struct RestoredPlanReview {
    pub review_id: String,
    pub plan_run_id: String,
    pub plan_path: String,
    pub plan: StructuredPlan,
    pub decision_rx: tokio::sync::oneshot::Receiver<PlanReviewDecision>,
}

impl RestoredPlanReview {
    pub fn build_review_payload(&self) -> serde_json::Value {
        let plan_content = structured_plan_to_markdown(&self.plan);
        let tasks = serde_json::to_value(&self.plan.tasks).unwrap_or_default();
        serde_json::json!({
            "plan_title": self.plan.plan_title,
            "plan_file": if self.plan.plan_file.is_empty() {
                &self.plan_path
            } else {
                &self.plan.plan_file
            },
            "estimated_effort": self.plan.estimated_effort,
            "overview": self.plan.overview,
            "scope_in": self.plan.scope_in,
            "scope_out": self.plan.scope_out,
            "guardrails": self.plan.guardrails,
            "plan_content": plan_content,
            "tasks": tasks,
        })
    }
}

// ---------------------------------------------------------------------------
// Plan orchestrator
// ---------------------------------------------------------------------------

/// Orchestrates the plan-mode workflow triggered by the `Plan` tool.
pub struct PlanOrchestrator;

#[derive(Debug, Clone)]
struct ResolvedAgentConfig {
    system_prompt: String,
    max_iterations: usize,
    max_tool_calls: usize,
    preferred_models: Vec<String>,
    provider_tags: Vec<String>,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    trust_tier: Option<TrustTier>,
    allowed_tools: Vec<String>,
    prune_tool_history: bool,
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Clone)]
struct PlanReviewOutcome {
    approved: bool,
    status: String,
    feedback: String,
    plan_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetainedPhaseContext {
    task_id: String,
    phase: usize,
    title: String,
    summary: String,
}

impl PlanOrchestrator {
    /// Handle a `Plan` tool call.
    ///
    /// Workflow:
    /// 1. Create a child session for the `plan-writer` sub-agent
    /// 2. Execute plan-writer (structured JSON plan with tasks)
    /// 3. Resolve the effective plan review mode from operation mode + Guardrails
    /// 4. Execute phases automatically or pause for structured user approval
    /// 5. Return consolidated results or a short rejection for the root agent
    pub async fn handle(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        progress: Option<&TurnEventSender>,
        cancel: Option<CancellationToken>,
    ) -> Result<ToolOutput, ToolError> {
        if is_cancelled(cancel.as_ref()) {
            return Err(cancelled_tool_error());
        }

        let request = arguments
            .get("request")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'request' is required".into(),
            })?;

        let context = arguments
            .get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let resume_requested = arguments
            .get("resume")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // Auto-resume: if the session has a recent plan run that was cancelled
        // or partially failed (i.e. interrupted mid-execution with uncompleted
        // tasks), resume it instead of generating a new plan. This lets the
        // LLM naturally continue an interrupted plan — e.g. the user sends
        // "continue" and the LLM calls Plan again, which picks up where it
        // left off without losing completed phase results.
        //
        // Explicit `resume: true` in the arguments forces a resume attempt
        // even if the latest run completed (useful for retrying a plan the
        // LLM considers unsatisfactory). When `resume` is false (default),
        // only interrupted runs (cancelled / partial_failure) are resumed.
        if !is_cancelled(cancel.as_ref()) {
            if let Some(resumed) = Self::try_resume_interrupted_plan(
                container,
                parent_session_id,
                resume_requested,
                progress,
                cancel.as_ref(),
            )
            .await?
            {
                return Ok(resumed);
            }
        }

        // Generate a plan file path.
        let plan_slug = slug_from_request(request);
        let plan_dir = container.data_dir.join("plan");
        if let Err(e) = tokio::fs::create_dir_all(&plan_dir).await {
            tracing::warn!(error = %e, "failed to create plan directory");
        }
        let plan_path = plan_dir.join(format!("{plan_slug}.md"));

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: serde_json::json!({
                    "request": request,
                    "context": context,
                })
                .to_string(),
                result_preview: "Starting plan generation".into(),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: Some(build_plan_start_metadata(&plan_path)),
            });
        }

        // A plan spawned while executing an already-approved plan's phases
        // auto-approves: the top-level plan is the sole human approval gate.
        // This prevents concurrent sub-plan reviews that the user cannot see
        // and that do not pause the main agent.
        let review_mode = resolve_review_mode_for_handle(container, parent_session_id).await;

        let max_parallel = resolve_max_parallel_phases(arguments);

        // Plan-write -> review loop. On `Revise`, the user's feedback is fed
        // back into plan-writer for another iteration. `Approve` exits the
        // loop into execution; `Reject` / timeout / cancel returns immediately.
        let mut revision_feedback: Option<String> = None;
        let mut revision_count: usize = 0;
        let (structured_plan, review) = loop {
            tracing::info!(
                request = %request,
                revision = revision_count,
                "plan orchestrator: starting plan-writer"
            );
            let structured_plan = Self::run_plan_writer(
                container,
                parent_session_id,
                request,
                context,
                &plan_path,
                review_mode,
                revision_feedback.as_deref(),
                progress,
                cancel.as_ref(),
            )
            .await?;

            let review = Self::resolve_plan_review(
                &structured_plan,
                &plan_path,
                review_mode,
                parent_session_id,
                progress,
                &container.session_state.pending_plan_reviews,
                container,
            )
            .await;

            if review.status == "revise" {
                revision_count += 1;
                if revision_count > MAX_PLAN_REVISIONS {
                    tracing::warn!(
                        revision_count,
                        "plan orchestrator: exceeded MAX_PLAN_REVISIONS, aborting"
                    );
                    let exhausted = PlanReviewOutcome {
                        approved: false,
                        status: "max_revisions_exceeded".to_string(),
                        feedback: review.feedback,
                        plan_run_id: review.plan_run_id,
                    };
                    break (structured_plan, exhausted);
                }
                revision_feedback = Some(review.feedback);
                continue;
            }

            break (structured_plan, review);
        };

        let total_tasks = structured_plan.tasks.len();

        if !review.approved {
            return Ok(build_plan_rejected_tool_output(
                &plan_path,
                &structured_plan,
                &review,
            ));
        }

        // Dependency-aware parallel execution after explicit human approval.
        tracing::info!(
            total_tasks,
            max_parallel,
            "plan orchestrator: starting phase execution"
        );

        let plan_run_id = if let Some(id) = review.plan_run_id.clone() {
            id
        } else {
            let id = Uuid::new_v4().to_string();
            let plan_json = serde_json::to_string(&structured_plan).unwrap_or_default();
            let _ = container
                .plan_run_store
                .create_run(
                    &id,
                    parent_session_id.as_str(),
                    &plan_json,
                    &plan_path.display().to_string(),
                )
                .await;
            id
        };

        let phase_results = Box::pin(plan_execution_ctx::scoped(Self::execute_phases(
            container,
            parent_session_id,
            &structured_plan,
            &plan_path,
            &plan_run_id,
            max_parallel,
            progress,
            cancel.as_ref(),
            None,
        )))
        .await;

        // Cancellation is not an error from the caller's perspective: the
        // user asked to stop, so we return a partial ToolOutput with whatever
        // phases completed before the cancel. The plan run is marked
        // "cancelled" in the DB so it can be detected and resumed later.
        let phase_results = match phase_results {
            Ok(results) => results,
            Err(e) if is_cancelled_tool_error(&e) => {
                let _ = container
                    .plan_run_store
                    .update_run_status(&plan_run_id, "cancelled")
                    .await;
                // Load whatever step results were persisted before the cancel
                // so the ToolOutput reflects actual progress, not an empty vec.
                let persisted = container
                    .plan_run_store
                    .load_step_results(&plan_run_id)
                    .await
                    .unwrap_or_default();
                let phase_results: Vec<serde_json::Value> = persisted
                    .iter()
                    .map(|step| {
                        let mut entry = serde_json::json!({
                            "task_id": step.task_id,
                            "phase": step.phase,
                            "title": step.title,
                            "status": step.status,
                        });
                        if let Some(output) = &step.output_json {
                            if step.status == "completed" {
                                entry["summary"] = serde_json::Value::String(output.clone());
                            } else {
                                entry["error"] = serde_json::Value::String(output.clone());
                            }
                        }
                        entry
                    })
                    .collect();
                let completed = phase_results
                    .iter()
                    .filter(|r| r["status"] == "completed")
                    .count();
                let failed = phase_results
                    .iter()
                    .filter(|r| r["status"] == "failed")
                    .count();
                let metadata = build_plan_execution_metadata(
                    &plan_path,
                    &structured_plan,
                    &plan_run_id,
                    completed,
                    failed,
                    &phase_results,
                );
                return Ok(ToolOutput {
                    success: false,
                    content: build_plan_execution_tool_content(
                        &plan_path,
                        &structured_plan,
                        &plan_run_id,
                        completed,
                        failed,
                        &phase_results,
                        Some(&review),
                        None,
                    ),
                    warnings: vec!["Plan execution was cancelled by the user. \
                        Completed phases are preserved; the plan can be resumed."
                        .into()],
                    metadata,
                });
            }
            Err(e) => return Err(e),
        };

        let run_status = if phase_results.iter().any(|r| r["status"] == "failed") {
            "partial_failure"
        } else {
            "completed"
        };
        let _ = container
            .plan_run_store
            .update_run_status(&plan_run_id, run_status)
            .await;

        let completed = phase_results
            .iter()
            .filter(|r| r["status"] == "completed")
            .count();
        let failed = phase_results
            .iter()
            .filter(|r| r["status"] == "failed")
            .count();
        let metadata = build_plan_execution_metadata(
            &plan_path,
            &structured_plan,
            &plan_run_id,
            completed,
            failed,
            &phase_results,
        );

        Ok(ToolOutput {
            success: failed == 0,
            content: build_plan_execution_tool_content(
                &plan_path,
                &structured_plan,
                &plan_run_id,
                completed,
                failed,
                &phase_results,
                Some(&review),
                None,
            ),
            warnings: vec![],
            metadata,
        })
    }

    /// Create a child session under the parent and run the plan-writer agent.
    ///
    /// The plan-writer returns a structured JSON plan directly, which is parsed
    /// into a [`StructuredPlan`]. A markdown representation is persisted to disk
    /// for human review.
    async fn run_plan_writer(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        context: &str,
        plan_path: &std::path::Path,
        review_mode: PlanReviewMode,
        revision_feedback: Option<&str>,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<StructuredPlan, ToolError> {
        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string("plan-writer")),
                title: Some("Plan Writer".to_string()),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to create plan-writer session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            "plan-writer",
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 1,
                max_tool_calls: 0,
                preferred_models: vec![],
                provider_tags: vec!["general".to_string()],
                temperature: Some(0.3),
                max_tokens: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                allowed_tools: vec![],
                prune_tool_history: false,
                response_format: None,
            },
        )
        .await;

        // Build the user message for the plan-writer as structured JSON.
        let user_msg = build_plan_writer_input(request, context, revision_feedback);

        let messages = build_subagent_messages(&settings.system_prompt, user_msg);
        let tool_defs =
            Self::load_tool_schemas_for_allowed_tools(container, &settings.allowed_tools).await;

        let exec_config = AgentExecutionConfig {
            agent_name: "plan-writer".to_string(),
            system_prompt: settings.system_prompt.clone(),
            max_iterations: settings.max_iterations,
            max_tool_calls: settings.max_tool_calls,
            tool_definitions: tool_defs,
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages,
            provider_id: None,
            preferred_models: settings.preferred_models.clone(),
            provider_tags: settings.provider_tags.clone(),
            fallback_provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: settings.temperature,
            max_tokens: settings.max_tokens,
            thinking: None,
            session_id: Some(child_session.id.clone()),
            session_uuid: child_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: request.to_string(),
            external_trace_id: None,
            trust_tier: settings.trust_tier,
            agent_allowed_tools: settings.allowed_tools.clone(),
            prune_tool_history: settings.prune_tool_history,
            response_format: settings.response_format.clone(),
            image_generation_options: None,
            inherited_constraints: None,
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error("plan-writer", e))?;

        emit_subagent_completed(container, child_uuid, "plan-writer", true);

        // Persist the plan-writer's transcript to its child session for drill-in.
        crate::chat::ChatService::persist_subagent_turn(
            container,
            &child_session.id,
            &exec_config.user_query,
            &result,
        )
        .await;

        // Parse the JSON output from the plan-writer response.
        let json_text = extract_json_from_response(&result.content);
        let json_text = repair_json(&json_text);
        let mut plan: StructuredPlan = parse_structured_plan(&json_text).map_err(|msg| {
            tracing::error!(
                raw_output = %result.content,
                error = %msg,
                "failed to parse plan-writer output"
            );
            ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to parse plan-writer output: {msg}"),
            }
        })?;

        normalize_plan_task_phases(&mut plan.tasks);

        if plan.plan_file.is_empty() {
            plan.plan_file = plan_path.display().to_string();
        }
        plan.execution_contract = build_plan_execution_contract(container, plan_path, &plan).await;

        // Persist a markdown representation for human review.
        let plan_content = structured_plan_to_markdown(&plan);
        if let Err(error) = persist_plan_content(plan_path, &plan_content).await {
            tracing::warn!(path = %plan_path.display(), %error, "failed to persist generated plan");
        }

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: "plan-writer completed".into(),
                result_preview: format!(
                    "{} tasks extracted, plan written to {}",
                    plan.tasks.len(),
                    plan_path.display()
                ),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: Some(build_plan_writer_stage_metadata(
                    plan_path,
                    &plan,
                    review_status_for_mode(review_mode),
                )),
            });
        }

        Ok(plan)
    }

    /// Resolve whether this plan should execute automatically or pause for
    /// structured user approval via the GUI / API dialog.
    ///
    /// The LLM is never involved in this decision -- the orchestrator pauses
    /// on a `oneshot::Receiver` until the presentation layer posts back via
    /// `deliver_review_decision`. The model only ever sees the final outcome
    /// (full execution results on approve, a short rejection `ToolOutput` on
    /// reject).
    async fn resolve_plan_review(
        plan: &StructuredPlan,
        plan_path: &Path,
        review_mode: PlanReviewMode,
        session_id: &SessionId,
        progress: Option<&TurnEventSender>,
        pending_plan_reviews: &PendingPlanReviews,
        container: &ServiceContainer,
    ) -> PlanReviewOutcome {
        match review_mode {
            PlanReviewMode::Auto => {
                if let Some(tx) = progress {
                    emit_plan_review_progress(tx, plan_path, plan, "auto_approved", "", None);
                }
                PlanReviewOutcome {
                    approved: true,
                    status: "auto_approved".to_string(),
                    feedback: String::new(),
                    plan_run_id: None,
                }
            }
            PlanReviewMode::Manual => {
                let Some(tx) = progress else {
                    tracing::warn!(
                        plan_title = %plan.plan_title,
                        "manual plan review requested without an event channel; auto-approving"
                    );
                    return PlanReviewOutcome {
                        approved: true,
                        status: "auto_approved_no_review_surface".to_string(),
                        feedback: String::new(),
                        plan_run_id: None,
                    };
                };

                let review_id = Uuid::new_v4().to_string();
                let plan_run_id = review_id.clone();

                let plan_json = serde_json::to_string(plan).unwrap_or_default();
                if let Err(e) = container
                    .plan_run_store
                    .create_run_with_status(
                        &plan_run_id,
                        session_id.as_str(),
                        &plan_json,
                        &plan_path.display().to_string(),
                        "awaiting_approval",
                    )
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        "failed to persist plan run as awaiting_approval"
                    );
                }

                let (decision_tx, decision_rx) = tokio::sync::oneshot::channel();

                {
                    let mut map = pending_plan_reviews.lock().await;
                    map.insert(
                        review_id.clone(),
                        crate::chat_types::PendingPlanReview::new(session_id.clone(), decision_tx),
                    );
                }

                emit_plan_review_progress(
                    tx,
                    plan_path,
                    plan,
                    "awaiting_user",
                    "",
                    Some(&review_id),
                );

                let plan_content = structured_plan_to_markdown(plan);
                let tasks_json = serde_json::to_value(&plan.tasks).unwrap_or_else(|err| {
                    tracing::warn!(error = %err, "failed to serialize plan tasks for review request");
                    serde_json::Value::Array(vec![])
                });
                let plan_file = if plan.plan_file.is_empty() {
                    plan_path.display().to_string()
                } else {
                    plan.plan_file.clone()
                };
                let _ = tx.send(TurnEvent::PlanReviewRequest {
                    review_id: review_id.clone(),
                    plan_title: plan.plan_title.clone(),
                    plan_file,
                    estimated_effort: plan.estimated_effort.clone(),
                    overview: plan.overview.clone(),
                    scope_in: plan.scope_in.clone(),
                    scope_out: plan.scope_out.clone(),
                    guardrails: plan.guardrails.clone(),
                    plan_content,
                    tasks: tasks_json,
                });

                let outcome = match decision_rx.await {
                    Ok(PlanReviewDecision::Approve) => {
                        emit_plan_review_progress(tx, plan_path, plan, "approved", "", None);
                        PlanReviewOutcome {
                            approved: true,
                            status: "approved".to_string(),
                            feedback: String::new(),
                            plan_run_id: Some(plan_run_id.clone()),
                        }
                    }
                    Ok(PlanReviewDecision::Revise { feedback }) => {
                        emit_plan_review_progress(
                            tx,
                            plan_path,
                            plan,
                            "feedback_received",
                            &feedback,
                            None,
                        );
                        PlanReviewOutcome {
                            approved: false,
                            status: "revise".to_string(),
                            feedback,
                            plan_run_id: Some(plan_run_id.clone()),
                        }
                    }
                    Ok(PlanReviewDecision::Reject { feedback }) => {
                        emit_plan_review_progress(tx, plan_path, plan, "rejected", &feedback, None);
                        PlanReviewOutcome {
                            approved: false,
                            status: "rejected".to_string(),
                            feedback,
                            plan_run_id: Some(plan_run_id.clone()),
                        }
                    }
                    Err(_) => {
                        Self::remove_pending_review(&review_id, pending_plan_reviews).await;
                        emit_plan_review_progress(
                            tx,
                            plan_path,
                            plan,
                            "review_cancelled",
                            "",
                            None,
                        );
                        PlanReviewOutcome {
                            approved: false,
                            status: "review_cancelled".to_string(),
                            feedback: String::new(),
                            plan_run_id: Some(plan_run_id.clone()),
                        }
                    }
                };

                let db_status = match outcome.status.as_str() {
                    "approved" => "running",
                    "revise" => "awaiting_approval",
                    "rejected" => "rejected",
                    _ => "cancelled",
                };
                let _ = container
                    .plan_run_store
                    .update_run_status(&plan_run_id, db_status)
                    .await;

                outcome
            }
        }
    }

    /// Deliver a user's plan review decision back to the awaiting
    /// orchestrator. Called by the presentation layer
    /// (`chat_answer_plan_review` Tauri command or the equivalent HTTP route).
    ///
    /// Returns `true` if the decision was delivered successfully.
    pub async fn deliver_review_decision(
        review_id: &str,
        decision: PlanReviewDecision,
        pending_plan_reviews: &PendingPlanReviews,
    ) -> bool {
        let sender = {
            let mut map = pending_plan_reviews.lock().await;
            map.remove(review_id)
        };

        if let Some(pending) = sender {
            pending.send(decision).is_ok()
        } else {
            tracing::warn!(
                review_id = %review_id,
                "deliver_review_decision: no pending review found (may have timed out)"
            );
            false
        }
    }

    /// Reconstruct a display summary for every persisted plan run in a session,
    /// oldest first, so the UI can present the full plan history independent of
    /// the loaded message window (surviving session switches and restarts).
    ///
    /// Each entry matches the `{ "display": { ... } }` shape the frontend
    /// already parses for live plan tool results, plus an explicit
    /// `plan_run_status` carrying the authoritative DB status so terminal states
    /// (`completed` / `partial_failure` / `rejected` / `cancelled` / `awaiting`)
    /// render correctly.
    pub async fn list_session_plans(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Vec<serde_json::Value> {
        let runs = match container
            .plan_run_store
            .list_runs_for_session(session_id.as_str())
            .await
        {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to list plan runs for session");
                return Vec::new();
            }
        };

        let mut out = Vec::with_capacity(runs.len());
        for run in runs {
            let plan: StructuredPlan = match serde_json::from_str(&run.plan_json) {
                Ok(plan) => plan,
                Err(e) => {
                    tracing::warn!(
                        plan_run_id = %run.id,
                        error = %e,
                        "skipping plan run with undeserializable plan_json"
                    );
                    continue;
                }
            };

            let steps = container
                .plan_run_store
                .load_step_results(&run.id)
                .await
                .unwrap_or_default();

            let phase_results: Vec<serde_json::Value> = steps
                .iter()
                .map(|step| {
                    serde_json::json!({
                        "task_id": step.task_id,
                        "phase": step.phase,
                        "title": step.title,
                        "status": step.status,
                    })
                })
                .collect();

            let completed = steps.iter().filter(|s| s.status == "completed").count();
            let failed = steps.iter().filter(|s| s.status == "failed").count();

            let mut meta = build_plan_execution_metadata(
                Path::new(&run.plan_path),
                &plan,
                &run.id,
                completed,
                failed,
                &phase_results,
            );
            if let Some(display) = meta.get_mut("display").and_then(|d| d.as_object_mut()) {
                display.insert(
                    "plan_run_status".to_string(),
                    serde_json::Value::String(run.status.clone()),
                );
            }
            out.push(meta);
        }
        out
    }

    async fn remove_pending_review(review_id: &str, pending: &PendingPlanReviews) {
        let mut map = pending.lock().await;
        map.remove(review_id);
    }

    /// Execute plan phases with dependency-aware parallelism.
    ///
    /// Builds a DAG from `PlanTask.depends_on`, then executes in waves:
    /// each wave runs all tasks whose dependencies are satisfied, up to
    /// `max_parallel` concurrently. Falls back to sequential execution if
    /// the DAG is invalid (cycles, missing deps).
    async fn execute_phases(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        plan: &StructuredPlan,
        plan_path: &Path,
        plan_run_id: &str,
        max_parallel: usize,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
        resume: Option<(HashSet<String>, Vec<serde_json::Value>)>,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let total_tasks = plan.tasks.len();
        let is_resumed = resume.is_some();

        // Build a lookup from task id to task.
        let task_map: std::collections::HashMap<&str, &PlanTask> =
            plan.tasks.iter().map(|t| (t.id.as_str(), t)).collect();

        // Build DAG. Fall back to sequential on error.
        let dag = match build_task_dag(&plan.tasks) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to build task DAG, falling back to sequential execution"
                );
                return Self::execute_phases_sequential(
                    container,
                    parent_session_id,
                    plan,
                    plan_path,
                    plan_run_id,
                    progress,
                    cancel,
                    resume,
                )
                .await;
            }
        };

        let (mut completed, mut phase_results) = resume.unwrap_or_default();
        let mut failed: HashSet<String> = HashSet::new();

        // Emit initial state showing retained results (resume only).
        if is_resumed {
            if let Some(tx) = progress {
                emit_plan_execution_progress(
                    tx,
                    plan_path,
                    plan,
                    plan_run_id,
                    &phase_results,
                    "Resuming plan execution".to_string(),
                );
            }
        }

        let heartbeat_cancel = CancellationToken::new();
        let heartbeat_handle = if is_resumed {
            None
        } else {
            progress.map(|tx| {
                let tx = tx.clone();
                let token = heartbeat_cancel.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
                    interval.tick().await;
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {
                                let _ = tx.send(TurnEvent::Heartbeat {
                                    agent_name: "plan-orchestrator".into(),
                                });
                            }
                            () = token.cancelled() => break,
                        }
                    }
                })
            })
        };

        loop {
            if is_cancelled(cancel) {
                return Err(cancelled_tool_error());
            }

            // Find ready tasks (deps satisfied, not completed, not failed).
            let ready: Vec<&TaskNode> = dag
                .ready_tasks(&completed)
                .into_iter()
                .filter(|n| !failed.contains(&n.id))
                .collect();

            if ready.is_empty() {
                break;
            }

            if !is_resumed {
                tracing::info!(
                    wave_size = ready.len(),
                    ready_ids = ?ready.iter().map(|n| &n.id).collect::<Vec<_>>(),
                    "plan orchestrator: starting parallel wave"
                );
            }

            // Check if any ready task has a failed dependency (transitive).
            // Skip those tasks entirely.
            let mut runnable = Vec::new();
            for node in &ready {
                let Some(task) = task_map.get(node.id.as_str()) else {
                    continue;
                };
                let has_failed_dep = task
                    .depends_on
                    .iter()
                    .any(|dep| failed.contains(dep.as_str()));
                if has_failed_dep {
                    tracing::warn!(
                        task_id = %task.id,
                        "skipping task due to failed dependency"
                    );
                    failed.insert(task.id.clone());
                    phase_results.push(serde_json::json!({
                        "task_id": task.id,
                        "phase": task.phase,
                        "title": task.title,
                        "status": "skipped",
                        "error": "dependency failed",
                    }));
                    let _ = container
                        .plan_run_store
                        .record_step_result(
                            plan_run_id,
                            &task.id,
                            task.phase,
                            &task.title,
                            "skipped",
                            Some("dependency failed"),
                        )
                        .await;
                } else {
                    runnable.push(*task);
                }
            }

            if runnable.is_empty() {
                break;
            }

            // Emit progress for all tasks starting in this wave.
            for task in &runnable {
                if let Some(tx) = progress {
                    let mut snapshot = phase_results.clone();
                    snapshot.push(serde_json::json!({
                        "task_id": task.id,
                        "phase": task.phase,
                        "title": task.title,
                        "status": "in_progress",
                    }));
                    emit_plan_execution_progress(
                        tx,
                        plan_path,
                        plan,
                        plan_run_id,
                        &snapshot,
                        format!("Executing phase {}: {}", task.phase, task.title),
                    );
                }
            }

            // Execute runnable tasks concurrently, in chunks of
            // `max_parallel` to bound resource usage.

            for chunk in runnable.chunks(max_parallel) {
                let retained = if is_resumed {
                    retained_phase_context_from_results(&phase_results)
                } else {
                    Vec::new()
                };

                let chunk_futures: Vec<_> = chunk
                    .iter()
                    .map(|task| {
                        let retained = &retained;
                        async move {
                            let result = Self::run_phase(
                                container,
                                parent_session_id,
                                task,
                                &plan.plan_title,
                                plan_path,
                                task.phase,
                                total_tasks,
                                &plan.execution_contract,
                                retained,
                                progress,
                                cancel,
                            )
                            .await;
                            (*task, result)
                        }
                    })
                    .collect();

                let chunk_results = futures::future::join_all(chunk_futures).await;

                for (task, result) in chunk_results {
                    match result {
                        Ok(summary) => {
                            completed.insert(task.id.clone());
                            phase_results.push(serde_json::json!({
                                "task_id": task.id,
                                "phase": task.phase,
                                "title": task.title,
                                "status": "completed",
                                "summary": summary,
                            }));
                            let _ = container
                                .plan_run_store
                                .record_step_result(
                                    plan_run_id,
                                    &task.id,
                                    task.phase,
                                    &task.title,
                                    "completed",
                                    Some(&summary),
                                )
                                .await;
                            if let Some(tx) = progress {
                                emit_plan_execution_progress(
                                    tx,
                                    plan_path,
                                    plan,
                                    plan_run_id,
                                    &phase_results,
                                    format!("Completed phase {}: {}", task.phase, task.title),
                                );
                            }
                        }
                        Err(e) => {
                            if is_cancelled_tool_error(&e) {
                                return Err(e);
                            }
                            if !is_resumed {
                                tracing::error!(
                                    task_id = %task.id,
                                    error = %e,
                                    "plan orchestrator: phase failed"
                                );
                            }
                            failed.insert(task.id.clone());
                            let error_str = e.to_string();
                            phase_results.push(serde_json::json!({
                                "task_id": task.id,
                                "phase": task.phase,
                                "title": task.title,
                                "status": "failed",
                                "error": error_str,
                            }));
                            let _ = container
                                .plan_run_store
                                .record_step_result(
                                    plan_run_id,
                                    &task.id,
                                    task.phase,
                                    &task.title,
                                    "failed",
                                    Some(&error_str),
                                )
                                .await;
                            if let Some(tx) = progress {
                                emit_plan_execution_progress(
                                    tx,
                                    plan_path,
                                    plan,
                                    plan_run_id,
                                    &phase_results,
                                    format!("Failed phase {}: {}", task.phase, task.title),
                                );
                            }
                        }
                    }
                }
            }
        }

        heartbeat_cancel.cancel();
        if let Some(handle) = heartbeat_handle {
            let _ = handle.await;
        }

        Ok(phase_results)
    }

    /// Sequential fallback when DAG construction fails.
    async fn execute_phases_sequential(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        plan: &StructuredPlan,
        plan_path: &Path,
        plan_run_id: &str,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
        resume: Option<(HashSet<String>, Vec<serde_json::Value>)>,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let total_tasks = plan.tasks.len();
        let is_resumed = resume.is_some();
        let (pre_completed, mut phase_results) = match resume {
            Some((c, r)) => (c, r),
            None => (HashSet::new(), Vec::with_capacity(total_tasks)),
        };

        let heartbeat_cancel = CancellationToken::new();
        let heartbeat_handle = if is_resumed {
            None
        } else {
            progress.map(|tx| {
                let tx = tx.clone();
                let token = heartbeat_cancel.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
                    interval.tick().await;
                    loop {
                        tokio::select! {
                            _ = interval.tick() => {
                                let _ = tx.send(TurnEvent::Heartbeat {
                                    agent_name: "plan-orchestrator".into(),
                                });
                            }
                            () = token.cancelled() => break,
                        }
                    }
                })
            })
        };

        for (idx, task) in plan.tasks.iter().enumerate() {
            if is_cancelled(cancel) {
                return Err(cancelled_tool_error());
            }

            // Skip already-completed tasks when resuming.
            if is_resumed && pre_completed.contains(&task.id) {
                continue;
            }

            if let Some(tx) = progress {
                let mut snapshot = phase_results.clone();
                snapshot.push(serde_json::json!({
                    "task_id": task.id,
                    "phase": task.phase,
                    "title": task.title,
                    "status": "in_progress",
                }));
                emit_plan_execution_progress(
                    tx,
                    plan_path,
                    plan,
                    plan_run_id,
                    &snapshot,
                    format!("Executing phase {}: {}", task.phase, task.title),
                );
            }

            let retained = if is_resumed {
                retained_phase_context_from_results(&phase_results)
            } else {
                Vec::new()
            };

            match Self::run_phase(
                container,
                parent_session_id,
                task,
                &plan.plan_title,
                plan_path,
                idx + 1,
                total_tasks,
                &plan.execution_contract,
                &retained,
                progress,
                cancel,
            )
            .await
            {
                Ok(summary) => {
                    phase_results.push(serde_json::json!({
                        "task_id": task.id,
                        "phase": task.phase,
                        "title": task.title,
                        "status": "completed",
                        "summary": summary,
                    }));
                    let _ = container
                        .plan_run_store
                        .record_step_result(
                            plan_run_id,
                            &task.id,
                            task.phase,
                            &task.title,
                            "completed",
                            Some(&summary),
                        )
                        .await;
                    if let Some(tx) = progress {
                        emit_plan_execution_progress(
                            tx,
                            plan_path,
                            plan,
                            plan_run_id,
                            &phase_results,
                            format!("Completed phase {}: {}", task.phase, task.title),
                        );
                    }
                }
                Err(e) => {
                    if is_cancelled_tool_error(&e) {
                        return Err(e);
                    }
                    if !is_resumed {
                        tracing::error!(
                            phase = idx + 1,
                            error = %e,
                            "plan orchestrator: phase failed"
                        );
                    }
                    let error_str = e.to_string();
                    phase_results.push(serde_json::json!({
                        "task_id": task.id,
                        "phase": task.phase,
                        "title": task.title,
                        "status": "failed",
                        "error": error_str,
                    }));
                    let _ = container
                        .plan_run_store
                        .record_step_result(
                            plan_run_id,
                            &task.id,
                            task.phase,
                            &task.title,
                            "failed",
                            Some(&error_str),
                        )
                        .await;
                    if let Some(tx) = progress {
                        emit_plan_execution_progress(
                            tx,
                            plan_path,
                            plan,
                            plan_run_id,
                            &phase_results,
                            format!("Failed phase {}: {}", task.phase, task.title),
                        );
                    }
                }
            }
        }

        heartbeat_cancel.cancel();
        if let Some(handle) = heartbeat_handle {
            let _ = handle.await;
        }

        Ok(phase_results)
    }

    /// Resume a plan execution from a specific task.
    ///
    /// Loads the persisted plan run, invalidates the target task and all its
    /// transitive downstream dependents, then re-enters `execute_phases` with
    /// the pre-seeded completed set. Works for both failed tasks (retry after
    /// error) and completed tasks (retry after dissatisfaction).
    pub async fn resume_plan(
        container: &ServiceContainer,
        session_id: &SessionId,
        plan_run_id: &str,
        from_task_id: &str,
        working_directory: Option<String>,
        progress: Option<&TurnEventSender>,
        cancel: Option<CancellationToken>,
    ) -> Result<ToolOutput, ToolError> {
        let run = container
            .plan_run_store
            .load_run(plan_run_id)
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to load plan run: {e}"),
            })?
            .ok_or_else(|| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("plan run '{plan_run_id}' not found"),
            })?;

        let mut structured_plan: StructuredPlan =
            serde_json::from_str(&run.plan_json).map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to parse stored plan: {e}"),
            })?;
        hydrate_plan_execution_contract(
            &mut structured_plan,
            std::path::Path::new(&run.plan_path),
            working_directory,
        );

        let step_results = container
            .plan_run_store
            .load_step_results(plan_run_id)
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to load step results: {e}"),
            })?;

        let plan_path = std::path::PathBuf::from(&run.plan_path);

        // Compute the set of tasks to invalidate: from_task_id + all
        // transitive downstream dependents. When from_task_id is empty,
        // no tasks are invalidated — we simply resume from the first
        // uncompleted task, preserving all existing step results.
        let invalidated = if from_task_id.is_empty() {
            HashSet::new()
        } else {
            compute_downstream_tasks(&structured_plan.tasks, from_task_id)
        };

        // Delete invalidated step results from store.
        if !invalidated.is_empty() {
            let invalidated_refs: Vec<&str> = invalidated
                .iter()
                .map(std::string::String::as_str)
                .collect();
            let _ = container
                .plan_run_store
                .delete_step_results(plan_run_id, &invalidated_refs)
                .await;
        }

        // Mark the run as running again.
        let _ = container
            .plan_run_store
            .update_run_status(plan_run_id, "running")
            .await;

        // Pre-seed completed set and phase_results from retained steps.
        let mut pre_completed: HashSet<String> = HashSet::new();
        let mut pre_phase_results: Vec<serde_json::Value> = Vec::new();
        for step in &step_results {
            if invalidated.contains(&step.task_id) {
                continue;
            }
            if step.status == "completed" {
                pre_completed.insert(step.task_id.clone());
            }
            let mut entry = serde_json::json!({
                "task_id": step.task_id,
                "phase": step.phase,
                "title": step.title,
                "status": step.status,
            });
            if let Some(ref output) = step.output_json {
                if step.status == "completed" {
                    entry["summary"] = serde_json::Value::String(output.clone());
                } else {
                    entry["error"] = serde_json::Value::String(output.clone());
                }
            }
            pre_phase_results.push(entry);
        }

        // Re-execute with pre-seeded state.
        let phase_results = Self::execute_phases(
            container,
            session_id,
            &structured_plan,
            &plan_path,
            plan_run_id,
            DEFAULT_MAX_PARALLEL_PHASES,
            progress,
            cancel.as_ref(),
            Some((pre_completed, pre_phase_results)),
        )
        .await?;

        let completed = phase_results
            .iter()
            .filter(|r| r["status"] == "completed")
            .count();
        let failed = phase_results
            .iter()
            .filter(|r| r["status"] == "failed")
            .count();

        let run_status = if failed > 0 {
            "partial_failure"
        } else {
            "completed"
        };
        let _ = container
            .plan_run_store
            .update_run_status(plan_run_id, run_status)
            .await;

        let metadata = build_plan_execution_metadata(
            &plan_path,
            &structured_plan,
            plan_run_id,
            completed,
            failed,
            &phase_results,
        );

        Ok(ToolOutput {
            success: failed == 0,
            content: build_plan_execution_tool_content(
                &plan_path,
                &structured_plan,
                plan_run_id,
                completed,
                failed,
                &phase_results,
                None,
                Some(from_task_id),
            ),
            warnings: vec![],
            metadata,
        })
    }

    /// Check whether the session has an interrupted plan run that can be
    /// resumed, and if so, resume it.
    ///
    /// An "interrupted" run is one whose status is `cancelled` or
    /// `partial_failure` and that has at least one task not yet completed.
    /// When `force` is true, any non-`completed` run is considered
    /// resumable (the LLM explicitly asked to resume).
    ///
    /// Returns `Ok(Some(tool_output))` when a resume was performed,
    /// `Ok(None)` when no resumable run was found (proceed to new plan).
    async fn try_resume_interrupted_plan(
        container: &ServiceContainer,
        session_id: &SessionId,
        force: bool,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Option<ToolOutput>, ToolError> {
        let latest_run = container
            .plan_run_store
            .find_latest_run(session_id.as_str())
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to find latest plan run: {e}"),
            })?;

        let Some(run) = latest_run else {
            return Ok(None);
        };

        let is_interrupted = matches!(run.status.as_str(), "cancelled" | "partial_failure");
        let is_resumable = if force {
            run.status != "completed"
        } else {
            is_interrupted
        };
        if !is_resumable {
            return Ok(None);
        }

        // Parse the stored plan to check for uncompleted tasks.
        let plan: StructuredPlan =
            serde_json::from_str(&run.plan_json).map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to parse stored plan for resume: {e}"),
            })?;

        let step_results = container
            .plan_run_store
            .load_step_results(&run.id)
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to load step results for resume: {e}"),
            })?;

        let completed_ids: HashSet<&str> = step_results
            .iter()
            .filter(|s| s.status == "completed")
            .map(|s| s.task_id.as_str())
            .collect();

        // If all tasks are already completed, nothing to resume.
        let has_uncompleted = plan
            .tasks
            .iter()
            .any(|t| !completed_ids.contains(t.id.as_str()));
        if !has_uncompleted {
            return Ok(None);
        }

        tracing::info!(
            plan_run_id = %run.id,
            status = %run.status,
            completed = completed_ids.len(),
            total = plan.tasks.len(),
            "plan orchestrator: resuming interrupted plan"
        );

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: serde_json::json!({
                    "resume": run.id,
                })
                .to_string(),
                result_preview: format!(
                    "Resuming interrupted plan ({} of {} phases completed)",
                    completed_ids.len(),
                    plan.tasks.len()
                ),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: Some(build_plan_start_metadata(std::path::Path::new(
                    &run.plan_path,
                ))),
            });
        }

        // Resume from the first uncompleted task. The resume_plan method
        // invalidates that task and all downstream dependents, then re-enters
        // execute_phases with the pre-seeded completed set.
        let first_uncompleted = plan
            .tasks
            .iter()
            .find(|t| !completed_ids.contains(t.id.as_str()))
            .map_or("", |t| t.id.as_str());

        let result = Self::resume_plan(
            container,
            session_id,
            &run.id,
            first_uncompleted,
            None,
            progress,
            cancel.cloned(),
        )
        .await?;

        Ok(Some(result))
    }

    /// Execute a single phase in its own child session.
    async fn run_phase(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        task: &PlanTask,
        plan_title: &str,
        plan_path: &Path,
        phase_num: usize,
        total_phases: usize,
        execution_contract: &PlanExecutionContract,
        retained_phase_context: &[RetainedPhaseContext],
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, ToolError> {
        let phase_title = format!("Phase {phase_num}: {}", task.title);

        // Archive any existing active child sessions with the same title for
        // this parent. This happens when retrying/resuming a phase: the old
        // failed/interrupted session is archived so it disappears from the
        // info panel, leaving only the fresh retry visible. Without this,
        // each retry leaves a stale duplicate (e.g. an empty session from a
        // crashed run) that clutters the sub-agent list.
        Self::archive_stale_phase_sessions(container, parent_session_id, &phase_title).await;

        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string(PHASE_EXECUTOR_AGENT_ID)),
                title: Some(phase_title),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to create phase session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            PHASE_EXECUTOR_AGENT_ID,
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: task.estimated_iterations.max(10),
                max_tool_calls: task.estimated_iterations.max(10) * 2,
                preferred_models: vec![],
                provider_tags: vec![],
                temperature: Some(0.7),
                max_tokens: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                allowed_tools: PHASE_EXECUTOR_FALLBACK_ALLOWED_TOOLS
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect(),
                prune_tool_history: false,
                response_format: None,
            },
        )
        .await;

        let tool_defs =
            Self::load_tool_schemas_for_allowed_tools(container, &settings.allowed_tools).await;

        let exec_config = build_phase_execution_config(
            &settings,
            &child_session.id,
            child_uuid,
            task,
            plan_title,
            plan_path,
            phase_num,
            total_phases,
            tool_defs,
            execution_contract,
            retained_phase_context,
        );

        let phase_name = format!("phase-{phase_num}");

        // Create a per-phase snapshot under the parent session's file history
        // so that rewind can restore to individual phase boundaries.
        if let Err(e) = crate::rewind::RewindService::ensure_manager(
            &container.file_history_managers,
            parent_session_id,
            &container.data_dir,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to ensure file history manager for plan phase");
        }
        let snapshot_id = format!("plan-phase-{}-{}", phase_num, child_session.id.as_str());
        crate::rewind::RewindService::make_snapshot(
            &container.file_history_managers,
            parent_session_id,
            &snapshot_id,
        )
        .await;

        // Execute the phase with automatic retry for transient LLM errors.
        //
        // The provider pool retries connection-level failures, but mid-stream
        // interruptions (HTTP 200 received, then EOF) are not retried there
        // because partial content was already emitted to the consumer. At this
        // layer we retry the full agent execution so a single dropped
        // connection does not waste all progress in a long-running phase.
        //
        // Reuses the pool's `RetryConfig` (same `max_retries`, backoff
        // strategy, and delay caps) and `StandardError::should_auto_retry`
        // classification so the retry semantics are consistent with the
        // provider layer.
        let retry_config = container.provider_pool().await.retry_config().clone();
        let mut attempt: u32 = 0;
        let result = loop {
            match AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
            {
                Ok(result) => break result,
                Err(e) if matches!(e, AgentExecutionError::Cancelled { .. }) => {
                    // Cancellation: persist partial transcript, emit
                    // completed(false), then propagate as a cancelled error.
                    persist_partial_subagent_turn(
                        container,
                        &child_session.id,
                        &exec_config.user_query,
                        &e,
                    )
                    .await;
                    emit_subagent_completed(container, child_uuid, PHASE_EXECUTOR_AGENT_ID, false);
                    return Err(map_plan_agent_error(&phase_name, e));
                }
                Err(e) => {
                    if is_cancelled(cancel) {
                        persist_partial_subagent_turn(
                            container,
                            &child_session.id,
                            &exec_config.user_query,
                            &e,
                        )
                        .await;
                        emit_subagent_completed(
                            container,
                            child_uuid,
                            PHASE_EXECUTOR_AGENT_ID,
                            false,
                        );
                        return Err(map_plan_agent_error(&phase_name, e));
                    }
                    if retry_config.enabled
                        && attempt < retry_config.max_retries
                        && is_transient_llm_error(&e)
                    {
                        attempt += 1;
                        let delay = retry_config.delay_for(attempt);
                        tracing::warn!(
                            task_id = %task.id,
                            phase = phase_num,
                            error = %e,
                            attempt,
                            max_retries = retry_config.max_retries,
                            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                            "transient LLM error in plan phase; retrying after backoff"
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    // Non-retryable error or retries exhausted: persist the
                    // partial transcript so the drill-in view shows what was
                    // accomplished before the failure, emit the completion
                    // event to unstick the UI, then propagate.
                    persist_partial_subagent_turn(
                        container,
                        &child_session.id,
                        &exec_config.user_query,
                        &e,
                    )
                    .await;
                    emit_subagent_completed(container, child_uuid, PHASE_EXECUTOR_AGENT_ID, false);
                    return Err(map_plan_agent_error(&phase_name, e));
                }
            }
        };

        // Persist the phase's own transcript to its child session so it can be
        // opened as a drill-in sub-chat, rendered by the same pipeline as the
        // main chat.
        crate::chat::ChatService::persist_subagent_turn(
            container,
            &child_session.id,
            &exec_config.user_query,
            &result,
        )
        .await;

        emit_subagent_completed(container, child_uuid, PHASE_EXECUTOR_AGENT_ID, true);

        Ok(result.content)
    }

    /// Archive existing active child sessions with a matching title.
    ///
    /// When retrying or resuming a phase, the previous run's child session
    /// (which may be empty after a crash, or contain a failed transcript)
    /// must be archived so it does not appear as a stale duplicate in the
    /// info panel's sub-agent list.
    async fn archive_stale_phase_sessions(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        phase_title: &str,
    ) {
        let children = match container.session_manager.children(parent_session_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to list child sessions for archival");
                return;
            }
        };
        for child in children {
            if child.state == y_core::session::SessionState::Active
                && child.title.as_deref() == Some(phase_title)
            {
                if let Err(e) = container
                    .session_manager
                    .transition_state(&child.id, y_core::session::SessionState::Archived)
                    .await
                {
                    tracing::warn!(
                        session_id = %child.id,
                        error = %e,
                        "failed to archive stale phase session"
                    );
                }
            }
        }
    }

    async fn resolve_agent_config(
        container: &ServiceContainer,
        agent_name: &str,
        fallback: ResolvedAgentConfig,
    ) -> ResolvedAgentConfig {
        let registry = container.agent_registry.lock().await;
        let Some(def) = registry.get(agent_name) else {
            return fallback;
        };

        Self::config_from_definition(def)
    }

    fn config_from_definition(def: &AgentDefinition) -> ResolvedAgentConfig {
        ResolvedAgentConfig {
            system_prompt: def.system_prompt.clone(),
            max_iterations: def.max_iterations,
            max_tool_calls: def.max_tool_calls,
            preferred_models: def.preferred_models.clone(),
            provider_tags: def.provider_tags.clone(),
            temperature: def.temperature,
            max_tokens: def
                .max_completion_tokens
                .and_then(|value| u32::try_from(value).ok()),
            trust_tier: Some(def.trust_tier),
            allowed_tools: def.allowed_tools.clone(),
            prune_tool_history: def.prune_tool_history,
            response_format: def.resolved_response_format().ok().flatten(),
        }
    }

    async fn load_tool_schemas_for_allowed_tools(
        container: &ServiceContainer,
        allowed_tools: &[String],
    ) -> Vec<serde_json::Value> {
        if allowed_tools.is_empty() {
            return vec![];
        }

        let mut defs = Vec::new();
        for tool_name in allowed_tools {
            let tn = y_core::types::ToolName::from_string(tool_name);
            if let Some(def) = container.tool_registry.get_definition(&tn).await {
                defs.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                }));
            }
        }
        defs
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a URL-safe slug from the request text.
fn slug_from_request(request: &str) -> String {
    let slug: String = request
        .chars()
        .take(50)
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        format!("plan-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))
    } else {
        slug
    }
}

fn emit_subagent_completed(
    container: &ServiceContainer,
    session_uuid: Uuid,
    agent_name: &str,
    success: bool,
) {
    let _ = container
        .diagnostics_broadcast
        .send(DiagnosticsEvent::SubagentCompleted {
            trace_id: Uuid::new_v4(),
            session_id: Some(session_uuid),
            agent_name: agent_name.to_string(),
            success,
        });
}

/// Persist a partial transcript to a child session when the agent execution
/// failed, so the drill-in view shows what was accomplished before the error.
///
/// Writes the user prompt, then any partial messages accumulated from
/// successful iterations before the failure. Falls back to an error message
/// when no partial messages are available (e.g. the first LLM call failed).
async fn persist_partial_subagent_turn(
    container: &ServiceContainer,
    session_id: &SessionId,
    user_input: &str,
    error: &AgentExecutionError,
) {
    let user_msg = Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: user_input.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::json!({}),
    };
    if let Err(e) = container
        .session_manager
        .append_message(session_id, &user_msg)
        .await
    {
        tracing::warn!(error = %e, session_id = %session_id, "failed to persist sub-agent prompt on error");
    }

    let empty: Vec<Message> = Vec::new();
    let partial_messages: &[Message] = match error {
        AgentExecutionError::LlmError {
            partial_messages, ..
        }
        | AgentExecutionError::Cancelled {
            partial_messages, ..
        } => partial_messages,
        _ => &empty,
    };

    if partial_messages.is_empty() {
        // No partial content: write a synthetic assistant message explaining
        // the failure so the drill-in view is not blank.
        let error_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::Assistant,
            content: format!("[Phase execution failed before any output was produced: {error}]"),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({
                "error": format!("{error}"),
                "partial": true,
            }),
        };
        if let Err(e) = container
            .session_manager
            .append_message(session_id, &error_msg)
            .await
        {
            tracing::warn!(error = %e, session_id = %session_id, "failed to persist sub-agent error message");
        }
        return;
    }

    for msg in partial_messages {
        if let Err(e) = container
            .session_manager
            .append_message(session_id, msg)
            .await
        {
            tracing::warn!(error = %e, session_id = %session_id, "failed to persist partial sub-agent message");
        }
    }
}

/// Best-effort repair of malformed JSON from LLM output.
///
/// Handles common issues: trailing commas before `]`/`}`, single-line
/// `// ...` comments, unescaped control characters in strings, invalid
/// escape sequences inside strings (e.g. a regex `\d` or a Windows path
/// `C:\Users`), and truncated output (unclosed brackets/braces).
fn repair_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut prev_char = '\0';
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i < len {
        let ch = chars[i];

        if in_string {
            if ch == '\\' {
                // A backslash begins an escape sequence. Consume it atomically:
                // preserve valid escapes, and repair invalid ones (e.g. a regex
                // `\d` or a Windows path `C:\Users`) by escaping the lone
                // backslash. This keeps the output lexically valid JSON and
                // ensures escaped quotes below are never seen as terminators.
                match chars.get(i + 1).copied() {
                    Some(esc @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                        out.push('\\');
                        out.push(esc);
                        prev_char = '\0';
                        i += 2;
                        continue;
                    }
                    Some('u')
                        if i + 6 <= len
                            && chars[i + 2..i + 6].iter().all(char::is_ascii_hexdigit) =>
                    {
                        out.extend(&chars[i..i + 6]);
                        prev_char = '\0';
                        i += 6;
                        continue;
                    }
                    _ => {
                        out.push('\\');
                        out.push('\\');
                        prev_char = '\0';
                        i += 1;
                        continue;
                    }
                }
            }
            if ch == '"' && prev_char != '\\' {
                // Heuristic: look ahead past whitespace. If the next
                // non-whitespace character is a JSON structural token
                // (or another quote, or end-of-input), this is a real
                // string terminator. Otherwise the LLM forgot to escape
                // an interior quote -- escape it for them.
                let mut j = i + 1;
                while j < len && chars[j].is_ascii_whitespace() {
                    j += 1;
                }
                let next_is_structural =
                    j >= len || matches!(chars[j], ',' | ']' | '}' | ':' | '"');
                if next_is_structural {
                    in_string = false;
                } else {
                    out.push('\\');
                    out.push('"');
                    prev_char = '"';
                    i += 1;
                    continue;
                }
            } else if ch == '\n' || ch == '\r' || ch == '\t' {
                // Escape bare control characters inside strings.
                match ch {
                    '\n' => {
                        out.push_str("\\n");
                        prev_char = 'n';
                        i += 1;
                        continue;
                    }
                    '\r' => {
                        out.push_str("\\r");
                        prev_char = 'r';
                        i += 1;
                        continue;
                    }
                    '\t' => {
                        out.push_str("\\t");
                        prev_char = 't';
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            out.push(ch);
            prev_char = ch;
            i += 1;
            continue;
        }

        // Outside a string.
        if ch == '"' {
            in_string = true;
            out.push(ch);
            prev_char = ch;
            i += 1;
            continue;
        }

        // Strip single-line comments.
        if ch == '/' && i + 1 < len && chars[i + 1] == '/' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Remove trailing commas: `,` followed (ignoring whitespace) by `]` or `}`.
        if ch == ',' {
            let mut j = i + 1;
            while j < len && chars[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == ']' || chars[j] == '}') {
                // Skip the comma.
                i += 1;
                continue;
            }
        }

        out.push(ch);
        prev_char = ch;
        i += 1;
    }

    // Close unclosed brackets/braces for truncated output. Strings are now
    // fully escaped, so skip escape sequences atomically to avoid mistaking an
    // escaped quote (`\"`) or backslash (`\\`) for a string boundary.
    let mut open_braces: i32 = 0;
    let mut open_brackets: i32 = 0;
    let mut scan_in_string = false;
    let scan_chars: Vec<char> = out.chars().collect();
    let mut k = 0;
    while k < scan_chars.len() {
        let c = scan_chars[k];
        if scan_in_string {
            if c == '\\' {
                k += 2;
                continue;
            }
            if c == '"' {
                scan_in_string = false;
            }
        } else {
            match c {
                '"' => scan_in_string = true,
                '{' => open_braces += 1,
                '}' => open_braces -= 1,
                '[' => open_brackets += 1,
                ']' => open_brackets -= 1,
                _ => {}
            }
        }
        k += 1;
    }
    for _ in 0..open_brackets {
        out.push(']');
    }
    for _ in 0..open_braces {
        out.push('}');
    }

    out
}

/// Extract JSON from a response that may be wrapped in markdown code blocks.
fn extract_json_from_response(text: &str) -> String {
    let trimmed = text.trim();
    // Check for ```json ... ``` wrapper.
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }
    // Check for generic ``` ... ``` wrapper.
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        // Skip the language identifier line if present.
        let content_start = after_marker.find('\n').map_or(0, |i| i + 1);
        if let Some(end) = after_marker[content_start..].find("```") {
            return after_marker[content_start..content_start + end]
                .trim()
                .to_string();
        }
    }
    trimmed.to_string()
}

/// Renumber phases as a contiguous 1..N sequence in task order.
///
/// The plan-writer LLM occasionally emits arbitrary phase numbers (e.g.
/// continuing a prior session's numbering, or skipping ahead). Phase numbers
/// are purely a display ordinal; parallelism is driven by `depends_on`. We
/// therefore renumber unconditionally to keep the GUI label ("Phase N")
/// consistent with task count.
fn normalize_plan_task_phases(tasks: &mut [PlanTask]) {
    for (i, task) in tasks.iter_mut().enumerate() {
        task.phase = i + 1;
    }
}

/// Leniently parse `StructuredPlan` from JSON text.
///
/// Tries strict deserialization first. On failure, falls back to
/// `serde_json::Value`-based extraction so that minor schema deviations
/// from the LLM (e.g. `plan_title` being an object, missing fields,
/// extra wrapper layers) do not crash the plan pipeline.
fn parse_structured_plan(json_text: &str) -> Result<StructuredPlan, String> {
    if let Ok(plan) = serde_json::from_str::<StructuredPlan>(json_text) {
        return Ok(plan);
    }

    let val: serde_json::Value = serde_json::from_str(json_text).map_err(|e| e.to_string())?;

    // Handle bare arrays: wrap into the expected object shape.
    if let Some(arr) = val.as_array() {
        return Ok(parse_structured_plan_from_tasks(arr));
    }

    let obj = val.as_object().ok_or("expected JSON object")?;

    let plan_title = obj
        .get("plan_title")
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(m) => m
                .get("title")
                .or_else(|| m.get("name"))
                .and_then(|v2| v2.as_str())
                .map(String::from),
            _ => v.to_string().into(),
        })
        .unwrap_or_else(|| "Untitled Plan".to_string());

    let plan_file = obj
        .get("plan_file")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let estimated_effort = obj
        .get("estimated_effort")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let overview = obj
        .get("overview")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let scope_in = parse_string_list_field(obj, "scope_in");
    let scope_out = parse_string_list_field(obj, "scope_out");
    let guardrails = parse_string_list_field(obj, "guardrails");

    let tasks_val = obj
        .get("tasks")
        .ok_or("missing 'tasks' array in plan-writer output")?;

    let tasks_arr = tasks_val.as_array().ok_or("'tasks' is not an array")?;

    let tasks = parse_plan_tasks(tasks_arr);

    Ok(StructuredPlan {
        plan_title,
        plan_file,
        estimated_effort,
        overview,
        scope_in,
        scope_out,
        guardrails,
        tasks,
        execution_contract: PlanExecutionContract::default(),
    })
}

/// Parse a bare JSON array as a `StructuredPlan` with a default title.
fn parse_structured_plan_from_tasks(arr: &[serde_json::Value]) -> StructuredPlan {
    let tasks = parse_plan_tasks(arr);
    StructuredPlan {
        plan_title: "Untitled Plan".to_string(),
        plan_file: String::new(),
        estimated_effort: String::new(),
        overview: String::new(),
        scope_in: vec![],
        scope_out: vec![],
        guardrails: vec![],
        execution_contract: PlanExecutionContract::default(),
        tasks,
    }
}

fn parse_string_list_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Vec<String> {
    obj.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Leniently parse an array of JSON values into `PlanTask` items.
///
/// Tries strict deserialization first per item, then falls back to
/// manual field extraction with common LLM field-name variations
/// (`task_id` for `id`, `objective` for `description`).
fn parse_plan_tasks(arr: &[serde_json::Value]) -> Vec<PlanTask> {
    let mut tasks = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let task: PlanTask = serde_json::from_value(item.clone()).unwrap_or_else(|_| {
            let o = item.as_object();
            PlanTask {
                id: o
                    .and_then(|m| m.get("id").or_else(|| m.get("task_id")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                phase: o
                    .and_then(|m| m.get("phase"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize,
                title: o
                    .and_then(|m| m.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Untitled Task")
                    .to_string(),
                description: o
                    .and_then(|m| m.get("description").or_else(|| m.get("objective")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                depends_on: o
                    .and_then(|m| m.get("depends_on"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                status: TaskStatus::Pending,
                estimated_iterations: o
                    .and_then(|m| m.get("estimated_iterations"))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(15) as usize,
                key_files: o
                    .and_then(|m| m.get("key_files"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                acceptance_criteria: o
                    .and_then(|m| m.get("acceptance_criteria"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        });
        let task = if task.id.is_empty() {
            PlanTask {
                id: format!("task-{}", i + 1),
                ..task
            }
        } else {
            task
        };
        tasks.push(task);
    }
    tasks
}

fn build_subagent_messages(system_prompt: &str, user_content: String) -> Vec<Message> {
    let mut messages = Vec::with_capacity(if system_prompt.is_empty() { 1 } else { 2 });
    if !system_prompt.is_empty() {
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::System,
            content: system_prompt.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });
    }
    messages.push(Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: user_content,
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    });
    messages
}

async fn persist_plan_content(plan_path: &Path, plan_content: &str) -> std::io::Result<()> {
    tokio::fs::write(plan_path, plan_content).await
}

async fn build_plan_execution_contract(
    container: &ServiceContainer,
    plan_path: &Path,
    plan: &StructuredPlan,
) -> PlanExecutionContract {
    let working_directory = {
        let pctx = container.prompt_context.read().await;
        pctx.working_directory.clone()
    };
    let additional_read_dirs = vec![plan_path.display().to_string()];
    let inherited_constraints = inherited_constraints_from_plan(plan);

    PlanExecutionContract {
        working_directory,
        additional_read_dirs,
        inherited_constraints,
    }
}

fn plan_execution_contract_for_phase(
    contract: &PlanExecutionContract,
    plan_path: &Path,
    scope_out: &[String],
    guardrails: &[String],
) -> PlanExecutionContract {
    let mut effective = contract.clone();
    if effective.additional_read_dirs.is_empty() {
        effective
            .additional_read_dirs
            .push(plan_path.display().to_string());
    }
    if effective.inherited_constraints.is_none() {
        effective.inherited_constraints = inherited_constraints_from_parts(scope_out, guardrails);
    }
    effective
}

fn hydrate_plan_execution_contract(
    plan: &mut StructuredPlan,
    plan_path: &Path,
    working_directory: Option<String>,
) {
    if plan.execution_contract.working_directory.is_none() {
        plan.execution_contract.working_directory = working_directory;
    }
    if plan.execution_contract.additional_read_dirs.is_empty() {
        plan.execution_contract
            .additional_read_dirs
            .push(plan_path.display().to_string());
    }
    if plan.execution_contract.inherited_constraints.is_none() {
        plan.execution_contract.inherited_constraints = inherited_constraints_from_plan(plan);
    }
}

fn inherited_constraints_from_plan(plan: &StructuredPlan) -> Option<InheritedConstraints> {
    inherited_constraints_from_parts(&plan.scope_out, &plan.guardrails)
}

fn inherited_constraints_from_parts(
    scope_out: &[String],
    guardrails: &[String],
) -> Option<InheritedConstraints> {
    let constraints = InheritedConstraints {
        scope_boundaries: scope_out.to_vec(),
        guardrails: guardrails.to_vec(),
        output_format: None,
    };
    (!constraints.is_empty()).then_some(constraints)
}

fn retained_phase_context_from_results(
    phase_results: &[serde_json::Value],
) -> Vec<RetainedPhaseContext> {
    phase_results
        .iter()
        .filter(|result| {
            result.get("status").and_then(serde_json::Value::as_str) == Some("completed")
        })
        .filter_map(|result| {
            let summary = result
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim();
            if summary.is_empty() {
                return None;
            }
            Some(RetainedPhaseContext {
                task_id: result
                    .get("task_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                phase: result
                    .get("phase")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|phase| usize::try_from(phase).ok())
                    .unwrap_or_default(),
                title: result
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                summary: summary.to_string(),
            })
        })
        .collect()
}

fn build_phase_user_message(
    task: &PlanTask,
    plan_title: &str,
    phase_num: usize,
    total_phases: usize,
    inherited_constraints: Option<&InheritedConstraints>,
    retained_phase_context: &[RetainedPhaseContext],
) -> String {
    let mut msg = format!(
        "You are executing phase {phase_num} of {total_phases} of the plan \"{plan_title}\".\n\n"
    );

    if let Some(constraints) = inherited_constraints.filter(|constraints| !constraints.is_empty()) {
        msg.push_str("## Constraints\n\n");
        if !constraints.scope_boundaries.is_empty() {
            msg.push_str("### Out of Scope (Do NOT touch)\n");
            for item in &constraints.scope_boundaries {
                let _ = writeln!(msg, "- {item}");
            }
            msg.push('\n');
        }
        if !constraints.guardrails.is_empty() {
            msg.push_str("### Guardrails\n");
            for item in &constraints.guardrails {
                let _ = writeln!(msg, "- {item}");
            }
            msg.push('\n');
        }
        if let Some(format) = &constraints.output_format {
            let _ = writeln!(msg, "### Output Format\n{format}\n");
        }
    }

    if !retained_phase_context.is_empty() {
        msg.push_str("## Retained Completed Phase Context\n\n");
        msg.push_str(
            "Use these retained results as already-established context. Do not rediscover them unless verification is required.\n\n",
        );
        for retained in retained_phase_context {
            let _ = writeln!(
                msg,
                "### Phase {}: {} ({})\n{}\n",
                retained.phase, retained.title, retained.task_id, retained.summary
            );
        }
    }

    let _ = write!(
        msg,
        "## Phase {phase_num}: {}\n\n\
         {}\n\n\
         Key files: {}\n\n\
         Acceptance criteria:\n{}\n\n\
         Execute this phase completely. Make all necessary code changes.",
        task.title,
        task.description,
        task.key_files.join(", "),
        task.acceptance_criteria
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    msg
}

fn build_phase_execution_config(
    settings: &ResolvedAgentConfig,
    session_id: &SessionId,
    session_uuid: Uuid,
    task: &PlanTask,
    plan_title: &str,
    plan_path: &Path,
    phase_num: usize,
    total_phases: usize,
    tool_definitions: Vec<serde_json::Value>,
    execution_contract: &PlanExecutionContract,
    retained_phase_context: &[RetainedPhaseContext],
) -> AgentExecutionConfig {
    let execution_contract = plan_execution_contract_for_phase(
        execution_contract,
        plan_path,
        &execution_contract
            .inherited_constraints
            .as_ref()
            .map_or_else(Vec::new, |constraints| constraints.scope_boundaries.clone()),
        &execution_contract
            .inherited_constraints
            .as_ref()
            .map_or_else(Vec::new, |constraints| constraints.guardrails.clone()),
    );
    let user_msg = build_phase_user_message(
        task,
        plan_title,
        phase_num,
        total_phases,
        execution_contract.inherited_constraints.as_ref(),
        retained_phase_context,
    );
    let messages = build_subagent_messages(&settings.system_prompt, user_msg);

    AgentExecutionConfig {
        agent_name: format!("{PHASE_EXECUTOR_AGENT_ID}:phase-{phase_num}"),
        system_prompt: settings.system_prompt.clone(),
        max_iterations: settings.max_iterations,
        max_tool_calls: settings.max_tool_calls,
        tool_definitions,
        tool_calling_mode: y_core::provider::ToolCallingMode::Native,
        tool_dialect: y_core::provider::ToolDialect::default(),
        messages,
        provider_id: None,
        preferred_models: settings.preferred_models.clone(),
        provider_tags: settings.provider_tags.clone(),
        fallback_provider_tags: vec![],
        request_mode: y_core::provider::RequestMode::TextChat,
        working_directory: execution_contract.working_directory.clone(),
        additional_read_dirs: execution_contract.additional_read_dirs.clone(),
        temperature: settings.temperature,
        max_tokens: settings.max_tokens,
        thinking: None,
        session_id: Some(session_id.clone()),
        session_uuid,
        knowledge_collections: vec![],
        use_context_pipeline: true,
        user_query: format!("Phase {phase_num}: {}", task.title),
        external_trace_id: None,
        trust_tier: settings.trust_tier,
        agent_allowed_tools: settings.allowed_tools.clone(),
        // Phase executors rely on the full tool/result history for multi-step
        // implementation work; pruning old tool pairs would discard context.
        prune_tool_history: settings.prune_tool_history,
        response_format: None,
        image_generation_options: None,
        inherited_constraints: execution_contract.inherited_constraints.clone(),
    }
}

/// Convert a [`StructuredPlan`] into a human-readable markdown document.
pub(crate) fn structured_plan_to_markdown(plan: &StructuredPlan) -> String {
    use std::fmt::Write;
    let mut md = String::new();
    let _ = writeln!(md, "---");
    let _ = writeln!(md, "title: {}", plan.plan_title);
    let _ = writeln!(md, "status: pending");
    let _ = writeln!(md, "total_phases: {}", plan.tasks.len());
    let _ = writeln!(md, "---");
    let _ = writeln!(md);
    if !plan.estimated_effort.is_empty() {
        let _ = writeln!(md, "Estimated effort: {}", plan.estimated_effort);
        let _ = writeln!(md);
    }
    if !plan.overview.is_empty() {
        let _ = writeln!(md, "## Overview");
        let _ = writeln!(md);
        let _ = writeln!(md, "{}", plan.overview);
        let _ = writeln!(md);
    }
    if !plan.scope_in.is_empty() {
        let _ = writeln!(md, "## Scope In");
        for item in &plan.scope_in {
            let _ = writeln!(md, "- {item}");
        }
        let _ = writeln!(md);
    }
    if !plan.scope_out.is_empty() {
        let _ = writeln!(md, "## Scope Out");
        for item in &plan.scope_out {
            let _ = writeln!(md, "- {item}");
        }
        let _ = writeln!(md);
    }
    if !plan.guardrails.is_empty() {
        let _ = writeln!(md, "## Guardrails");
        for item in &plan.guardrails {
            let _ = writeln!(md, "- {item}");
        }
        let _ = writeln!(md);
    }
    for task in &plan.tasks {
        let _ = writeln!(md, "## Phase {}: {}", task.phase, task.title);
        if !task.description.is_empty() {
            let _ = writeln!(md, "\n{}", task.description);
        }
        if !task.key_files.is_empty() {
            let _ = writeln!(md, "\n### Key Files");
            for f in &task.key_files {
                let _ = writeln!(md, "- {f}");
            }
        }
        if !task.acceptance_criteria.is_empty() {
            let _ = writeln!(md, "\n### Acceptance Criteria");
            for c in &task.acceptance_criteria {
                let _ = writeln!(md, "- {c}");
            }
        }
        if !task.depends_on.is_empty() {
            let _ = writeln!(md, "\nDepends on: {}", task.depends_on.join(", "));
        }
        let _ = writeln!(md);
    }
    md
}

fn resolve_plan_file_for_display(plan_path: &Path, plan: &StructuredPlan) -> String {
    if plan.plan_file.is_empty() {
        plan_path.display().to_string()
    } else {
        plan.plan_file.clone()
    }
}

/// Map a `review_status` into the `stage_status` shown by the GUI for a
/// `plan_stage` card. The "stage" here is plan-writing; once a plan exists,
/// the stage itself is done, *except* when we are still waiting on the user
/// (or on revision feedback) — those states must surface as running so the
/// renderer does not flip the card to "Done" prematurely.
fn stage_status_for_review_status(review_status: &str) -> &'static str {
    match review_status {
        "awaiting_user" | "feedback_received" => "running",
        _ => "completed",
    }
}

fn build_plan_writer_stage_metadata(
    plan_path: &std::path::Path,
    plan: &StructuredPlan,
    review_status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": stage_status_for_review_status(review_status),
            "plan_title": plan.plan_title,
            "plan_file": resolve_plan_file_for_display(plan_path, plan),
            "estimated_effort": plan.estimated_effort,
            "overview": plan.overview,
            "scope_in": plan.scope_in,
            "scope_out": plan.scope_out,
            "guardrails": plan.guardrails,
            "review_status": review_status,
            "review_feedback": "",
            "plan_content": structured_plan_to_markdown(plan),
            "tasks": plan.tasks,
        }
    })
}

fn review_status_for_mode(mode: PlanReviewMode) -> &'static str {
    match mode {
        PlanReviewMode::Auto => "auto_approved",
        PlanReviewMode::Manual => "awaiting_user",
    }
}

/// Resolve the review mode for a `Plan` tool call inside `handle`.
///
/// A plan spawned while executing an already-approved plan's phases
/// auto-approves: the top-level plan is the sole human approval gate. This
/// prevents concurrent sub-plan reviews that the user cannot see and that do
/// not pause the main agent. Otherwise the configured mode applies.
async fn resolve_review_mode_for_handle(
    container: &ServiceContainer,
    parent_session_id: &SessionId,
) -> PlanReviewMode {
    if plan_execution_ctx::is_active() {
        return PlanReviewMode::Auto;
    }
    resolve_effective_plan_review_mode(container, parent_session_id).await
}

async fn resolve_effective_plan_review_mode(
    container: &ServiceContainer,
    parent_session_id: &SessionId,
) -> PlanReviewMode {
    let operation_mode = {
        let modes = container.session_state.session_operation_modes.read().await;
        modes
            .get(parent_session_id)
            .copied()
            .unwrap_or(OperationMode::Default)
    };

    match operation_mode {
        OperationMode::AutoReview | OperationMode::FullAccess => PlanReviewMode::Auto,
        OperationMode::Default => container.guardrail_manager.config().plan_review.mode,
    }
}

fn build_plan_review_metadata(
    plan_path: &Path,
    plan: &StructuredPlan,
    review_status: &str,
    review_feedback: &str,
    review_id: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": stage_status_for_review_status(review_status),
            "plan_title": plan.plan_title,
            "plan_file": resolve_plan_file_for_display(plan_path, plan),
            "estimated_effort": plan.estimated_effort,
            "overview": plan.overview,
            "scope_in": plan.scope_in,
            "scope_out": plan.scope_out,
            "guardrails": plan.guardrails,
            "review_status": review_status,
            "review_feedback": review_feedback,
            "review_id": review_id.unwrap_or(""),
            "plan_content": structured_plan_to_markdown(plan),
            "tasks": plan.tasks,
        }
    })
}

fn emit_plan_review_progress(
    tx: &TurnEventSender,
    plan_path: &Path,
    plan: &StructuredPlan,
    review_status: &str,
    review_feedback: &str,
    review_id: Option<&str>,
) {
    let result_preview = match review_status {
        "awaiting_user" => "Plan ready for human review",
        "auto_approved" => "Plan auto-approved; starting execution",
        "auto_approved_no_review_surface" => {
            "Plan auto-approved (no review surface available); starting execution"
        }
        "approved" => "Plan approved; starting execution",
        "rejected" => "Plan rejected; halting execution",
        "review_cancelled" => "Plan review cancelled; halting execution",
        "review_timeout" => "Plan review timed out; halting execution",
        "feedback_received" => "Plan review feedback received",
        "declined" => "Plan review dismissed",
        _ => "Plan review updated",
    };

    let _ = tx.send(TurnEvent::ToolResult {
        name: "Plan".into(),
        success: true,
        duration_ms: 0,
        input_preview: "plan review".into(),
        result_preview: result_preview.into(),
        agent_name: "plan-orchestrator".into(),
        url_meta: None,
        metadata: Some(build_plan_review_metadata(
            plan_path,
            plan,
            review_status,
            review_feedback,
            review_id,
        )),
    });
}

fn build_plan_rejected_tool_output(
    plan_path: &Path,
    plan: &StructuredPlan,
    review: &PlanReviewOutcome,
) -> ToolOutput {
    ToolOutput {
        success: true,
        content: serde_json::json!({
            "plan_title": plan.plan_title,
            "plan_file": plan_path.display().to_string(),
            "total_phases": plan.tasks.len(),
            "tasks": plan.tasks,
            "review": {
                "status": review.status,
                "approved": review.approved,
                "feedback": review.feedback,
            },
        }),
        warnings: vec![],
        metadata: build_plan_review_metadata(
            plan_path,
            plan,
            &review.status,
            &review.feedback,
            None,
        ),
    }
}

fn build_plan_start_metadata(plan_path: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": "running",
            "plan_title": "",
            "plan_file": plan_path.display().to_string(),
            "plan_content": "",
        }
    })
}

fn build_plan_execution_metadata(
    plan_path: &Path,
    plan: &StructuredPlan,
    plan_run_id: &str,
    completed: usize,
    failed: usize,
    phase_results: &[serde_json::Value],
) -> serde_json::Value {
    let tasks = build_execution_tasks(plan, phase_results);
    let phases = compact_plan_phase_results(phase_results);

    serde_json::json!({
        "action": "plan_executed",
        "display": {
            "kind": "plan_execution",
            "plan_title": plan.plan_title,
            "plan_file": resolve_plan_file_for_display(plan_path, plan),
            "plan_run_id": plan_run_id,
            "total_phases": plan.tasks.len(),
            "completed": completed,
            "failed": failed,
            "tasks": tasks,
            "phases": phases,
        }
    })
}

fn build_plan_execution_tool_content(
    plan_path: &Path,
    plan: &StructuredPlan,
    plan_run_id: &str,
    completed: usize,
    failed: usize,
    phase_results: &[serde_json::Value],
    review: Option<&PlanReviewOutcome>,
    resumed_from: Option<&str>,
) -> serde_json::Value {
    let mut content = serde_json::json!({
        "plan_title": plan.plan_title,
        "plan_file": plan_path.display().to_string(),
        "plan_run_id": plan_run_id,
        "total_phases": plan.tasks.len(),
        "completed": completed,
        "failed": failed,
        "phases": compact_plan_phase_results(phase_results),
    });

    if let Some(review) = review {
        content["review"] = serde_json::json!({
            "status": review.status,
            "approved": review.approved,
            "feedback": review.feedback,
        });
    }
    if let Some(from_task_id) = resumed_from {
        content["resumed_from"] = serde_json::Value::String(from_task_id.to_string());
    }
    if phase_results
        .iter()
        .any(|phase| phase.get("summary").is_some())
    {
        content["phase_summaries_omitted"] = serde_json::Value::Bool(true);
    }

    content
}

fn compact_plan_phase_results(phase_results: &[serde_json::Value]) -> Vec<serde_json::Value> {
    phase_results
        .iter()
        .map(compact_plan_phase_result)
        .collect()
}

fn compact_plan_phase_result(phase: &serde_json::Value) -> serde_json::Value {
    let mut compact = serde_json::Map::new();
    for key in ["task_id", "phase", "title", "status", "error"] {
        if let Some(value) = phase.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(compact)
}

fn build_execution_tasks(
    plan: &StructuredPlan,
    phase_results: &[serde_json::Value],
) -> Vec<PlanTask> {
    plan.tasks
        .iter()
        .cloned()
        .map(|mut task| {
            task.status = resolve_task_status(&task, phase_results);
            task
        })
        .collect()
}

fn resolve_task_status(task: &PlanTask, phase_results: &[serde_json::Value]) -> TaskStatus {
    let mut status = task.status;

    for phase in phase_results {
        let task_id = phase.get("task_id").and_then(|value| value.as_str());
        let phase_num = phase.get("phase").and_then(serde_json::Value::as_u64);
        let title = phase.get("title").and_then(|value| value.as_str());

        let matches_task = task_id == Some(task.id.as_str())
            || phase_num == Some(task.phase as u64)
            || title == Some(task.title.as_str());
        if !matches_task {
            continue;
        }

        status = match phase.get("status").and_then(|value| value.as_str()) {
            Some("completed") => TaskStatus::Completed,
            Some("failed") => TaskStatus::Failed,
            Some("in_progress") => TaskStatus::InProgress,
            Some("pending") => TaskStatus::Pending,
            _ => status,
        };
    }

    status
}

/// Build the structured JSON user message passed to `plan-writer`.
///
/// Includes `task`, `context`, and `plan_path`.
fn build_plan_writer_input(
    request: &str,
    context: &str,
    revision_feedback: Option<&str>,
) -> String {
    let mut obj = serde_json::json!({
        "task": request,
        "context": context,
    });
    if let Some(fb) = revision_feedback {
        obj["revision_feedback"] = serde_json::Value::String(fb.to_string());
    }
    obj.to_string()
}

/// Resolve the max-parallel-phases setting from Plan tool arguments.
///
/// Reads the optional `max_parallel_phases` field from the arguments JSON
/// (accepting u64 or i64). Clamps to `[1, MAX_PARALLEL_PHASES_CEILING]` and
/// falls back to `DEFAULT_MAX_PARALLEL_PHASES` when the field is missing,
/// zero, or not a number.
fn resolve_max_parallel_phases(arguments: &serde_json::Value) -> usize {
    let raw = arguments.get("max_parallel_phases").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
    });

    match raw {
        Some(0) | None => DEFAULT_MAX_PARALLEL_PHASES,
        Some(n) => {
            let n = usize::try_from(n).unwrap_or(DEFAULT_MAX_PARALLEL_PHASES);
            n.min(MAX_PARALLEL_PHASES_CEILING)
        }
    }
}

/// Convert structured plan tasks into a validated [`TaskDag`].
///
/// Each [`PlanTask`] becomes a [`TaskNode`] with its `depends_on` mapped to
/// DAG dependencies. The DAG is validated (cycles, missing deps) before
/// returning; callers should fall back to sequential execution on error.
fn build_task_dag(tasks: &[PlanTask]) -> Result<TaskDag, DagError> {
    let mut dag = TaskDag::new();
    for task in tasks {
        dag.add_task(TaskNode {
            id: task.id.clone(),
            name: task.title.clone(),
            priority: TaskPriority::Normal,
            dependencies: task.depends_on.clone(),
            ..TaskNode::default()
        })?;
    }
    dag.validate()?;
    Ok(dag)
}

fn count_phase_results(phase_results: &[serde_json::Value], status: &str) -> usize {
    phase_results
        .iter()
        .filter(|phase| phase.get("status").and_then(|value| value.as_str()) == Some(status))
        .count()
}

/// Compute the transitive downstream closure for a given `task_id`.
///
/// Returns a set containing `from_task_id` itself plus every task that
/// transitively depends on it (directly or indirectly).
fn compute_downstream_tasks(tasks: &[PlanTask], from_task_id: &str) -> HashSet<String> {
    // Build reverse dependency map: task_id -> set of tasks that depend on it.
    let mut dependents: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for task in tasks {
        for dep in &task.depends_on {
            dependents.entry(dep.as_str()).or_default().push(&task.id);
        }
    }

    let mut invalidated = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(from_task_id.to_string());
    invalidated.insert(from_task_id.to_string());

    while let Some(current) = queue.pop_front() {
        if let Some(children) = dependents.get(current.as_str()) {
            for &child in children {
                if invalidated.insert(child.to_string()) {
                    queue.push_back(child.to_string());
                }
            }
        }
    }

    invalidated
}

fn emit_plan_execution_progress(
    tx: &TurnEventSender,
    plan_path: &Path,
    plan: &StructuredPlan,
    plan_run_id: &str,
    phase_results: &[serde_json::Value],
    result_preview: String,
) {
    let completed = count_phase_results(phase_results, "completed");
    let failed = count_phase_results(phase_results, "failed");

    let _ = tx.send(TurnEvent::ToolResult {
        name: "Plan".into(),
        success: failed == 0,
        duration_ms: 0,
        input_preview: "plan execution progress".into(),
        result_preview,
        agent_name: "plan-orchestrator".into(),
        url_meta: None,
        metadata: Some(build_plan_execution_metadata(
            plan_path,
            plan,
            plan_run_id,
            completed,
            failed,
            phase_results,
        )),
    });
}

fn cancelled_tool_error() -> ToolError {
    ToolError::RuntimeError {
        name: "Plan".into(),
        message: PLAN_CANCELLED_MESSAGE.into(),
    }
}

fn is_cancelled(cancel: Option<&CancellationToken>) -> bool {
    cancel.is_some_and(CancellationToken::is_cancelled)
}

fn is_cancelled_tool_error(error: &ToolError) -> bool {
    matches!(
        error,
        ToolError::RuntimeError { message, .. } if message == PLAN_CANCELLED_MESSAGE
    )
}

fn map_plan_agent_error(agent_name: &str, error: AgentExecutionError) -> ToolError {
    match error {
        AgentExecutionError::Cancelled { .. } => cancelled_tool_error(),
        other => ToolError::RuntimeError {
            name: "Plan".into(),
            message: format!("{agent_name} execution failed: {other}"),
        },
    }
}

/// Whether an [`AgentExecutionError`] represents a transient LLM failure that
/// is safe to retry at the phase level.
///
/// Delegates to the same [`StandardError::should_auto_retry`] classification
/// the provider pool uses, so the retry decision is consistent across layers.
/// Non-`LlmError` variants (context, loop limits, cancelled) and `LlmError`
/// without a preserved typed error are never retried.
fn is_transient_llm_error(error: &AgentExecutionError) -> bool {
    let AgentExecutionError::LlmError {
        provider_error: Some(pe),
        ..
    } = error
    else {
        return false;
    };
    y_provider::classify_provider_error(pe).should_auto_retry()
}

// ---------------------------------------------------------------------------
// Complexity assessment (auto mode) -- preserved from old orchestrator
// ---------------------------------------------------------------------------

/// Agent ID for the complexity classifier (matches
/// `config/agents/complexity-classifier.toml`).
const CLASSIFIER_AGENT_ID: &str = "complexity-classifier";

/// Fallback system prompt used when the agent definition is not found.
const CLASSIFIER_FALLBACK_PROMPT: &str = "\
You are a task complexity classifier. Given the user's request, respond with \
exactly one word: \"plan\" if the task requires multi-file changes, architectural \
design, refactoring, or multi-step coordination. Respond \"fast\" if the task is \
a single-file fix, formatting, direct question, or simple tweak. \
Respond with ONLY \"plan\" or \"fast\", nothing else.";

/// Assess whether the user's request is complex enough to warrant plan mode.
///
/// Loads the `complexity-classifier` agent definition from the registry and
/// executes a single-turn, zero-tool LLM call. Falls back to built-in
/// defaults if the definition is missing.
///
/// Returns `true` if the classifier outputs "plan". On any error, defaults
/// to `false` (no plan) to avoid blocking the user.
pub async fn assess_complexity(
    container: &ServiceContainer,
    user_input: &str,
    provider_id: Option<&str>,
) -> String {
    use y_core::types::{Message, Role};

    let registry = container.agent_registry.lock().await;
    let agent_def = registry.get(CLASSIFIER_AGENT_ID);

    let system_prompt = agent_def.map_or_else(
        || CLASSIFIER_FALLBACK_PROMPT.to_string(),
        |d| d.system_prompt.clone(),
    );
    let temperature = agent_def.and_then(|d| d.temperature).unwrap_or(0.0);
    let max_iterations = agent_def.map_or(1, |d| d.max_iterations);
    let max_tokens = agent_def.and_then(|d| d.max_completion_tokens).unwrap_or(5) as u32;
    let provider_tags: Vec<String> = agent_def
        .map(|d| d.provider_tags.clone())
        .unwrap_or_default();
    let prune_tool_history = agent_def.is_some_and(|d| d.prune_tool_history);

    drop(registry);

    let messages = vec![
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: system_prompt,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        },
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_input.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        },
    ];

    let exec_config = AgentExecutionConfig {
        agent_name: CLASSIFIER_AGENT_ID.to_string(),
        system_prompt: String::new(),
        max_iterations,
        max_tool_calls: usize::MAX,
        tool_definitions: vec![],
        tool_calling_mode: y_core::provider::ToolCallingMode::Native,
        tool_dialect: y_core::provider::ToolDialect::default(),
        messages,
        provider_id: provider_id.map(String::from),
        preferred_models: vec![],
        provider_tags,
        fallback_provider_tags: vec![],
        request_mode: y_core::provider::RequestMode::TextChat,
        working_directory: None,
        additional_read_dirs: vec![],
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
        thinking: None,
        session_id: None,
        session_uuid: Uuid::new_v4(),
        knowledge_collections: vec![],
        use_context_pipeline: false,
        user_query: user_input.to_string(),
        external_trace_id: None,
        trust_tier: None,
        agent_allowed_tools: vec![],
        prune_tool_history,
        response_format: None,
        image_generation_options: None,
        inherited_constraints: None,
    };

    match AgentService::execute(container, &exec_config, None, None).await {
        Ok(result) => {
            let response = result.content.trim().to_lowercase();
            tracing::debug!(
                classifier_response = %response,
                "plan mode complexity assessment"
            );
            if response.contains("loop") {
                "loop".to_string()
            } else if response.contains("plan") {
                "plan".to_string()
            } else {
                "fast".to_string()
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "complexity assessment failed, defaulting to fast mode"
            );
            "fast".to_string()
        }
    }
}

#[cfg(test)]
mod tests;
