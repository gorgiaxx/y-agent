//! Plan orchestrator: handles the `Plan` tool call by delegating to
//! the `plan-writer` sub-agent, resolving the configured review policy, and
//! executing approved phases in child sessions.
//!
//! Follows the same pattern as `TaskDelegationOrchestrator` and
//! `ToolSearchOrchestrator` -- the `tool_dispatch` layer intercepts
//! `Plan` tool calls and routes them here.

use std::collections::HashSet;
use std::path::Path;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_agent::orchestrator::dag::{DagError, TaskDag, TaskNode, TaskPriority};
use y_agent::AgentDefinition;
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
/// Maximum time to wait for the user to approve or reject a plan before
/// giving up and returning a timeout outcome.
const PLAN_REVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
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
    pub tasks: Vec<PlanTask>,
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

        let review_mode = resolve_effective_plan_review_mode(container, parent_session_id).await;

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
                &container.pending_plan_reviews,
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
        let phase_results = Self::execute_phases(
            container,
            parent_session_id,
            &structured_plan,
            &plan_path,
            max_parallel,
            progress,
            cancel.as_ref(),
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
        let metadata = build_plan_execution_metadata(
            &plan_path,
            &structured_plan,
            completed,
            failed,
            &phase_results,
        );

        Ok(ToolOutput {
            success: failed == 0,
            content: serde_json::json!({
                "plan_title": structured_plan.plan_title,
                "plan_file": plan_path.display().to_string(),
                "total_phases": total_tasks,
                "completed": completed,
                "failed": failed,
                "review": {
                    "status": review.status,
                    "approved": review.approved,
                    "feedback": review.feedback,
                },
                "phases": phase_results,
            }),
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
            messages,
            provider_id: None,
            preferred_models: settings.preferred_models.clone(),
            provider_tags: settings.provider_tags.clone(),
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
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error("plan-writer", e))?;

        emit_subagent_completed(container, child_uuid, "plan-writer", true);

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

        for (i, task) in plan.tasks.iter_mut().enumerate() {
            if task.phase == 0 {
                task.phase = i + 1;
            }
        }

        if plan.plan_file.is_empty() {
            plan.plan_file = plan_path.display().to_string();
        }

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
    /// reject / timeout).
    async fn resolve_plan_review(
        plan: &StructuredPlan,
        plan_path: &Path,
        review_mode: PlanReviewMode,
        session_id: &SessionId,
        progress: Option<&TurnEventSender>,
        pending_plan_reviews: &PendingPlanReviews,
    ) -> PlanReviewOutcome {
        match review_mode {
            PlanReviewMode::Auto => {
                if let Some(tx) = progress {
                    emit_plan_review_progress(tx, plan_path, plan, "auto_approved", "");
                }
                PlanReviewOutcome {
                    approved: true,
                    status: "auto_approved".to_string(),
                    feedback: String::new(),
                }
            }
            PlanReviewMode::Manual => {
                // Non-interactive context (no progress channel): we cannot
                // surface a dialog, so auto-approve and log a warning rather
                // than hang forever. Matches the spirit of
                // `UserInteractionOrchestrator::format_as_plain_text`.
                let Some(tx) = progress else {
                    tracing::warn!(
                        plan_title = %plan.plan_title,
                        "manual plan review requested without an event channel; auto-approving"
                    );
                    return PlanReviewOutcome {
                        approved: true,
                        status: "auto_approved_no_review_surface".to_string(),
                        feedback: String::new(),
                    };
                };

                let review_id = Uuid::new_v4().to_string();
                let (decision_tx, decision_rx) = tokio::sync::oneshot::channel();

                {
                    let mut map = pending_plan_reviews.lock().await;
                    map.insert(
                        review_id.clone(),
                        crate::chat_types::PendingPlanReview::new(session_id.clone(), decision_tx),
                    );
                }

                emit_plan_review_progress(tx, plan_path, plan, "awaiting_user", "");

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

                match tokio::time::timeout(PLAN_REVIEW_TIMEOUT, decision_rx).await {
                    Ok(Ok(PlanReviewDecision::Approve)) => {
                        emit_plan_review_progress(tx, plan_path, plan, "approved", "");
                        PlanReviewOutcome {
                            approved: true,
                            status: "approved".to_string(),
                            feedback: String::new(),
                        }
                    }
                    Ok(Ok(PlanReviewDecision::Revise { feedback })) => {
                        emit_plan_review_progress(
                            tx,
                            plan_path,
                            plan,
                            "feedback_received",
                            &feedback,
                        );
                        PlanReviewOutcome {
                            approved: false,
                            status: "revise".to_string(),
                            feedback,
                        }
                    }
                    Ok(Ok(PlanReviewDecision::Reject { feedback })) => {
                        emit_plan_review_progress(tx, plan_path, plan, "rejected", &feedback);
                        PlanReviewOutcome {
                            approved: false,
                            status: "rejected".to_string(),
                            feedback,
                        }
                    }
                    Ok(Err(_)) => {
                        Self::remove_pending_review(&review_id, pending_plan_reviews).await;
                        emit_plan_review_progress(tx, plan_path, plan, "review_cancelled", "");
                        PlanReviewOutcome {
                            approved: false,
                            status: "review_cancelled".to_string(),
                            feedback: String::new(),
                        }
                    }
                    Err(_) => {
                        Self::remove_pending_review(&review_id, pending_plan_reviews).await;
                        emit_plan_review_progress(tx, plan_path, plan, "review_timeout", "");
                        PlanReviewOutcome {
                            approved: false,
                            status: "review_timeout".to_string(),
                            feedback: String::new(),
                        }
                    }
                }
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
        max_parallel: usize,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let total_tasks = plan.tasks.len();

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
                    progress,
                    cancel,
                )
                .await;
            }
        };

        let mut completed: HashSet<String> = HashSet::new();
        let mut failed: HashSet<String> = HashSet::new();
        let mut phase_results: Vec<serde_json::Value> = Vec::with_capacity(total_tasks);

        let heartbeat_cancel = CancellationToken::new();
        let heartbeat_handle = progress.map(|tx| {
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
        });

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

            tracing::info!(
                wave_size = ready.len(),
                ready_ids = ?ready.iter().map(|n| &n.id).collect::<Vec<_>>(),
                "plan orchestrator: starting parallel wave"
            );

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
                        &snapshot,
                        format!("Executing phase {}: {}", task.phase, task.title),
                    );
                }
            }

            // Execute runnable tasks concurrently, in chunks of
            // `max_parallel` to bound resource usage.
            for chunk in runnable.chunks(max_parallel) {
                let chunk_futures: Vec<_> = chunk
                    .iter()
                    .map(|task| async move {
                        let result = Self::run_phase(
                            container,
                            parent_session_id,
                            task,
                            &plan.plan_title,
                            plan_path,
                            task.phase,
                            total_tasks,
                            progress,
                            cancel,
                        )
                        .await;
                        (*task, result)
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
                            if let Some(tx) = progress {
                                emit_plan_execution_progress(
                                    tx,
                                    plan_path,
                                    plan,
                                    &phase_results,
                                    format!("Completed phase {}: {}", task.phase, task.title),
                                );
                            }
                        }
                        Err(e) => {
                            if is_cancelled_tool_error(&e) {
                                return Err(e);
                            }
                            tracing::error!(
                                task_id = %task.id,
                                error = %e,
                                "plan orchestrator: phase failed"
                            );
                            failed.insert(task.id.clone());
                            phase_results.push(serde_json::json!({
                                "task_id": task.id,
                                "phase": task.phase,
                                "title": task.title,
                                "status": "failed",
                                "error": e.to_string(),
                            }));
                            if let Some(tx) = progress {
                                emit_plan_execution_progress(
                                    tx,
                                    plan_path,
                                    plan,
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
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let total_tasks = plan.tasks.len();
        let mut phase_results = Vec::with_capacity(total_tasks);

        let heartbeat_cancel = CancellationToken::new();
        let heartbeat_handle = progress.map(|tx| {
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
        });

        for (idx, task) in plan.tasks.iter().enumerate() {
            if is_cancelled(cancel) {
                return Err(cancelled_tool_error());
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
                    &snapshot,
                    format!("Executing phase {}: {}", task.phase, task.title),
                );
            }

            match Self::run_phase(
                container,
                parent_session_id,
                task,
                &plan.plan_title,
                plan_path,
                idx + 1,
                total_tasks,
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
                    if let Some(tx) = progress {
                        emit_plan_execution_progress(
                            tx,
                            plan_path,
                            plan,
                            &phase_results,
                            format!("Completed phase {}: {}", task.phase, task.title),
                        );
                    }
                }
                Err(e) => {
                    if is_cancelled_tool_error(&e) {
                        return Err(e);
                    }
                    tracing::error!(
                        phase = idx + 1,
                        error = %e,
                        "plan orchestrator: phase failed"
                    );
                    phase_results.push(serde_json::json!({
                        "task_id": task.id,
                        "phase": task.phase,
                        "title": task.title,
                        "status": "failed",
                        "error": e.to_string(),
                    }));
                    if let Some(tx) = progress {
                        emit_plan_execution_progress(
                            tx,
                            plan_path,
                            plan,
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

    /// Execute a single phase in its own child session.
    async fn run_phase(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        task: &PlanTask,
        plan_title: &str,
        plan_path: &Path,
        phase_num: usize,
        total_phases: usize,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, ToolError> {
        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string(PHASE_EXECUTOR_AGENT_ID)),
                title: Some(format!("Phase {phase_num}: {}", task.title)),
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
        );

        let phase_name = format!("phase-{phase_num}");
        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error(&phase_name, e))?;

        emit_subagent_completed(container, child_uuid, PHASE_EXECUTOR_AGENT_ID, true);

        Ok(result.content)
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

/// Best-effort repair of malformed JSON from LLM output.
///
/// Handles common issues: trailing commas before `]`/`}`, single-line
/// `// ...` comments, unescaped control characters in strings, and
/// truncated output (unclosed brackets/braces).
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

    // Close unclosed brackets/braces for truncated output.
    let mut open_braces: i32 = 0;
    let mut open_brackets: i32 = 0;
    let mut scan_in_string = false;
    let mut scan_prev = '\0';
    for c in out.chars() {
        if scan_in_string {
            if c == '"' && scan_prev != '\\' {
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
        scan_prev = c;
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

fn build_phase_user_message(
    task: &PlanTask,
    plan_title: &str,
    phase_num: usize,
    total_phases: usize,
) -> String {
    format!(
        "You are executing phase {phase_num} of {total_phases} of the plan \"{plan_title}\".\n\n\
         ## Phase {phase_num}: {}\n\n\
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
    )
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
) -> AgentExecutionConfig {
    let user_msg = build_phase_user_message(task, plan_title, phase_num, total_phases);
    let messages = build_subagent_messages(&settings.system_prompt, user_msg);

    AgentExecutionConfig {
        agent_name: PHASE_EXECUTOR_AGENT_ID.to_string(),
        system_prompt: settings.system_prompt.clone(),
        max_iterations: settings.max_iterations,
        max_tool_calls: settings.max_tool_calls,
        tool_definitions,
        tool_calling_mode: y_core::provider::ToolCallingMode::Native,
        messages,
        provider_id: None,
        preferred_models: settings.preferred_models.clone(),
        provider_tags: settings.provider_tags.clone(),
        request_mode: y_core::provider::RequestMode::TextChat,
        working_directory: None,
        additional_read_dirs: vec![plan_path.display().to_string()],
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
    }
}

/// Convert a [`StructuredPlan`] into a human-readable markdown document.
fn structured_plan_to_markdown(plan: &StructuredPlan) -> String {
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

fn build_plan_writer_stage_metadata(
    plan_path: &std::path::Path,
    plan: &StructuredPlan,
    review_status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": "completed",
            "plan_title": plan.plan_title,
            "plan_file": if plan.plan_file.is_empty() {
                plan_path.display().to_string()
            } else {
                plan.plan_file.clone()
            },
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

async fn resolve_effective_plan_review_mode(
    container: &ServiceContainer,
    parent_session_id: &SessionId,
) -> PlanReviewMode {
    let operation_mode = {
        let modes = container.session_operation_modes.read().await;
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
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": "completed",
            "plan_title": plan.plan_title,
            "plan_file": if plan.plan_file.is_empty() {
                plan_path.display().to_string()
            } else {
                plan.plan_file.clone()
            },
            "estimated_effort": plan.estimated_effort,
            "overview": plan.overview,
            "scope_in": plan.scope_in,
            "scope_out": plan.scope_out,
            "guardrails": plan.guardrails,
            "review_status": review_status,
            "review_feedback": review_feedback,
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
        metadata: build_plan_review_metadata(plan_path, plan, &review.status, &review.feedback),
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
    completed: usize,
    failed: usize,
    phase_results: &[serde_json::Value],
) -> serde_json::Value {
    let tasks = build_execution_tasks(plan, phase_results);

    serde_json::json!({
        "action": "plan_executed",
        "display": {
            "kind": "plan_execution",
            "plan_title": plan.plan_title,
            "plan_file": plan_path.display().to_string(),
            "total_phases": plan.tasks.len(),
            "completed": completed,
            "failed": failed,
            "tasks": tasks,
            "phases": phase_results,
        }
    })
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

fn emit_plan_execution_progress(
    tx: &TurnEventSender,
    plan_path: &Path,
    plan: &StructuredPlan,
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
        messages,
        provider_id: provider_id.map(String::from),
        preferred_models: vec![],
        provider_tags,
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn make_test_container() -> (crate::container::ServiceContainer, TempDir) {
        let tmpdir = tempfile::TempDir::new().unwrap();
        let config = crate::config::ServiceConfig {
            storage: y_storage::StorageConfig {
                db_path: ":memory:".to_string(),
                pool_size: 1,
                wal_enabled: false,
                transcript_dir: tmpdir.path().join("transcripts"),
                ..y_storage::StorageConfig::default()
            },
            ..crate::config::ServiceConfig::default()
        };
        let container = crate::container::ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    #[test]
    fn test_slug_from_request() {
        assert_eq!(
            slug_from_request("Refactor the plan mode"),
            "refactor-the-plan-mode"
        );
        assert_eq!(slug_from_request("Hello, World! 123"), "hello--world--123");
    }

    #[test]
    fn test_extract_json_from_response_plain() {
        let input = r#"{"plan_title": "test", "tasks": []}"#;
        assert_eq!(extract_json_from_response(input), input);
    }

    #[test]
    fn test_extract_json_from_response_code_block() {
        let input = "```json\n{\"plan_title\": \"test\"}\n```";
        assert_eq!(
            extract_json_from_response(input),
            "{\"plan_title\": \"test\"}"
        );
    }

    #[test]
    fn test_extract_json_from_response_generic_block() {
        let input = "```\n{\"a\": 1}\n```";
        assert_eq!(extract_json_from_response(input), "{\"a\": 1}");
    }

    #[test]
    fn test_build_subagent_messages_prepends_system_prompt() {
        let messages = build_subagent_messages("system rules", "user task".into());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, y_core::types::Role::System);
        assert_eq!(messages[0].content, "system rules");
        assert_eq!(messages[1].role, y_core::types::Role::User);
        assert_eq!(messages[1].content, "user task");
    }

    #[tokio::test]
    async fn test_persist_plan_content_writes_plan_file() {
        let tmpdir = tempfile::TempDir::new().unwrap();
        let plan_path = tmpdir.path().join("gui-plan.md");

        persist_plan_content(&plan_path, "# GUI Plan Stream Fix")
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&plan_path).await.unwrap();
        assert_eq!(content, "# GUI Plan Stream Fix");
    }

    #[test]
    fn test_build_plan_start_metadata_marks_plan_as_running() {
        let meta = build_plan_start_metadata(std::path::Path::new("/tmp/gui-plan.md"));

        assert_eq!(meta["display"]["kind"], "plan_stage");
        assert_eq!(meta["display"]["stage"], "plan_writer");
        assert_eq!(meta["display"]["stage_status"], "running");
        assert_eq!(meta["display"]["plan_file"], "/tmp/gui-plan.md");
    }

    #[test]
    fn test_structured_plan_to_markdown_includes_phases() {
        let plan = StructuredPlan {
            plan_title: "Test Plan".into(),
            plan_file: "/tmp/plan.md".into(),
            estimated_effort: "Short(1-4h)".into(),
            overview: "Create the initial structure".into(),
            scope_in: vec!["Initial structure".into()],
            scope_out: vec!["Deployment".into()],
            guardrails: vec!["Stay within src/".into()],
            tasks: vec![PlanTask {
                id: "phase-1".into(),
                phase: 1,
                title: "Setup".into(),
                description: "Create initial structure".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec!["src/main.rs".into()],
                acceptance_criteria: vec!["Structure exists".into()],
            }],
        };
        let md = structured_plan_to_markdown(&plan);
        assert!(md.contains("title: Test Plan"));
        assert!(md.contains("Estimated effort: Short(1-4h)"));
        assert!(md.contains("Create the initial structure"));
        assert!(md.contains("- Initial structure"));
        assert!(md.contains("- Deployment"));
        assert!(md.contains("- Stay within src/"));
        assert!(md.contains("## Phase 1: Setup"));
        assert!(md.contains("Create initial structure"));
        assert!(md.contains("- src/main.rs"));
        assert!(md.contains("- Structure exists"));
    }

    #[test]
    fn test_review_status_for_mode_distinguishes_auto_and_manual() {
        assert_eq!(
            review_status_for_mode(PlanReviewMode::Manual),
            "awaiting_user"
        );
        assert_eq!(
            review_status_for_mode(PlanReviewMode::Auto),
            "auto_approved"
        );
    }

    #[tokio::test]
    async fn test_operation_mode_overrides_guardrail_plan_review_mode() {
        let (container, _tmpdir) = make_test_container().await;
        let session_id = SessionId("session-1".into());

        assert_eq!(
            resolve_effective_plan_review_mode(&container, &session_id).await,
            PlanReviewMode::Manual
        );

        {
            let mut modes = container.session_operation_modes.write().await;
            modes.insert(session_id.clone(), OperationMode::AutoReview);
        }

        assert_eq!(
            resolve_effective_plan_review_mode(&container, &session_id).await,
            PlanReviewMode::Auto
        );
    }

    #[test]
    fn test_build_plan_writer_stage_metadata_includes_tasks() {
        let plan = StructuredPlan {
            plan_title: "GUI Plan Stream Fix".into(),
            plan_file: "/tmp/gui-plan.md".into(),
            estimated_effort: "Short(1-4h)".into(),
            overview: "Render structured plan output for review.".into(),
            scope_in: vec!["Plan renderer".into()],
            scope_out: vec!["Execution policy".into()],
            guardrails: vec!["Avoid raw JSON in the GUI".into()],
            tasks: vec![PlanTask {
                id: "task-1".into(),
                phase: 1,
                title: "Render structured plan output".into(),
                description: "Use structured metadata instead of raw JSON".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 12,
                key_files: vec![
                    "crates/y-gui/src/components/chat-panel/chat-box/ToolCallCard.tsx".into(),
                ],
                acceptance_criteria: vec!["Task list is rendered as a dedicated component".into()],
            }],
        };

        let meta = build_plan_writer_stage_metadata(
            std::path::Path::new("/tmp/gui-plan.md"),
            &plan,
            "awaiting_user",
        );

        assert_eq!(meta["display"]["kind"], "plan_stage");
        assert_eq!(meta["display"]["stage"], "plan_writer");
        assert_eq!(meta["display"]["plan_title"], "GUI Plan Stream Fix");
        assert_eq!(
            meta["display"]["tasks"][0]["title"],
            "Render structured plan output"
        );
        assert_eq!(meta["display"]["estimated_effort"], "Short(1-4h)");
        assert_eq!(
            meta["display"]["overview"],
            "Render structured plan output for review."
        );
        assert_eq!(meta["display"]["review_status"], "awaiting_user");
    }

    #[test]
    fn test_build_plan_execution_metadata_updates_task_statuses() {
        let plan = StructuredPlan {
            plan_title: "GUI Plan Stream Fix".into(),
            plan_file: "/tmp/gui-plan.md".into(),
            estimated_effort: String::new(),
            overview: String::new(),
            scope_in: vec![],
            scope_out: vec![],
            guardrails: vec![],
            tasks: vec![
                PlanTask {
                    id: "task-1".into(),
                    phase: 1,
                    title: "Render markdown output".into(),
                    description: "Use markdown rendering for plan output.".into(),
                    depends_on: vec![],
                    status: TaskStatus::Pending,
                    estimated_iterations: 8,
                    key_files: vec![
                        "crates/y-gui/src/components/chat-panel/chat-box/tool-renderers/PlanRenderer.tsx"
                            .into(),
                    ],
                    acceptance_criteria: vec!["Plan content renders as markdown".into()],
                },
                PlanTask {
                    id: "task-2".into(),
                    phase: 2,
                    title: "Keep execution state visible".into(),
                    description: "Do not drop the running indicator during plan execution.".into(),
                    depends_on: vec!["task-1".into()],
                    status: TaskStatus::Pending,
                    estimated_iterations: 10,
                    key_files: vec!["crates/y-gui/src/hooks/useChat.ts".into()],
                    acceptance_criteria: vec!["Stop button stays visible".into()],
                },
            ],
        };

        let phase_results = vec![serde_json::json!({
            "task_id": "task-1",
            "phase": 1,
            "title": "Render markdown output",
            "status": "completed",
        })];

        let meta = build_plan_execution_metadata(
            std::path::Path::new("/tmp/gui-plan.md"),
            &plan,
            1,
            0,
            &phase_results,
        );

        assert_eq!(meta["display"]["tasks"][0]["status"], "completed");
        assert_eq!(meta["display"]["tasks"][1]["status"], "pending");
    }

    #[test]
    fn test_build_phase_execution_config_uses_registry_limits() {
        let settings = ResolvedAgentConfig {
            system_prompt: "Execute the phase".into(),
            max_iterations: 30,
            max_tool_calls: 60,
            preferred_models: vec!["test-model".into()],
            provider_tags: vec!["coding".into()],
            temperature: Some(0.7),
            max_tokens: Some(2048),
            trust_tier: Some(TrustTier::BuiltIn),
            allowed_tools: vec!["FileWrite".into(), "ShellExec".into()],
            prune_tool_history: false,
            response_format: None,
        };
        let task = PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Implement execution path".into(),
            description: "Wire the phase executor through the agent registry.".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 12,
            key_files: vec!["crates/y-service/src/plan_orchestrator.rs".into()],
            acceptance_criteria: vec!["Phase executor limit comes from registry".into()],
        };

        let config = build_phase_execution_config(
            &settings,
            &SessionId::new(),
            Uuid::new_v4(),
            &task,
            "Registry-backed phase execution",
            Path::new("/tmp/gui-plan.md"),
            1,
            3,
            vec![],
        );

        assert_eq!(config.agent_name, PHASE_EXECUTOR_AGENT_ID);
        assert_eq!(config.max_iterations, 30);
        assert_eq!(config.max_tool_calls, 60);
        assert_eq!(
            config.agent_allowed_tools,
            vec!["FileWrite".to_string(), "ShellExec".to_string()]
        );
        assert_eq!(config.messages.len(), 2);
        assert_eq!(config.messages[0].role, y_core::types::Role::System);
        assert!(config.messages[1].content.contains("phase 1 of 3"));
        assert_eq!(config.additional_read_dirs, vec!["/tmp/gui-plan.md"]);
    }

    #[test]
    fn test_plan_task_serde() {
        let task = PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "Test".into(),
            description: "Do things".into(),
            depends_on: vec![],
            status: TaskStatus::Pending,
            estimated_iterations: 15,
            key_files: vec!["file.rs".into()],
            acceptance_criteria: vec!["It works".into()],
        };
        let json = serde_json::to_string(&task).unwrap();
        let parsed: PlanTask = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "task-1");
        assert_eq!(parsed.status, TaskStatus::Pending);
    }

    #[test]
    fn test_structured_plan_serde() {
        let json = r#"{
            "plan_title": "Test Plan",
            "plan_file": "/tmp/plan.md",
            "tasks": [{
                "id": "task-1",
                "phase": 1,
                "title": "Phase 1",
                "description": "Do stuff",
                "depends_on": [],
                "status": "pending",
                "estimated_iterations": 10,
                "key_files": [],
                "acceptance_criteria": ["works"]
            }]
        }"#;
        let plan: StructuredPlan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.plan_title, "Test Plan");
        assert_eq!(plan.tasks.len(), 1);
    }

    #[tokio::test]
    async fn test_resolve_agent_config_uses_plan_writer_definition() {
        let (container, _tmpdir) = make_test_container().await;
        let config = PlanOrchestrator::resolve_agent_config(
            &container,
            "plan-writer",
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 1,
                max_tool_calls: 1,
                preferred_models: vec![],
                provider_tags: vec![],
                temperature: None,
                max_tokens: None,
                trust_tier: None,
                allowed_tools: vec![],
                prune_tool_history: false,
                response_format: None,
            },
        )
        .await;

        assert!(config.system_prompt.contains("You are a plan writer"));
        assert_eq!(config.max_iterations, 1);
        assert_eq!(config.max_tool_calls, 0);
        assert_eq!(config.provider_tags, vec!["general", "coding"]);
        assert!(config.allowed_tools.is_empty());
        assert!(!config.prune_tool_history);
    }

    #[tokio::test]
    async fn test_resolve_agent_config_uses_plan_phase_executor_definition() {
        let (container, _tmpdir) = make_test_container().await;
        let config = PlanOrchestrator::resolve_agent_config(
            &container,
            PHASE_EXECUTOR_AGENT_ID,
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 12,
                max_tool_calls: 24,
                preferred_models: vec![],
                provider_tags: vec!["fallback".into()],
                temperature: None,
                max_tokens: None,
                trust_tier: None,
                allowed_tools: vec!["FallbackTool".into()],
                prune_tool_history: true,
                response_format: None,
            },
        )
        .await;

        assert!(config.system_prompt.contains("plan phase executor"));
        assert_eq!(config.max_iterations, 60);
        assert_eq!(config.max_tool_calls, 100);
        assert!(config.allowed_tools.iter().any(|tool| tool == "FileWrite"));
        assert!(config.allowed_tools.iter().any(|tool| tool == "ToolSearch"));
        assert!(!config.prune_tool_history);
    }

    #[test]
    fn test_build_task_dag_independent_tasks() {
        let tasks = vec![
            PlanTask {
                id: "task-1".into(),
                phase: 1,
                title: "Setup schema".into(),
                description: "Create tables".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
            PlanTask {
                id: "task-2".into(),
                phase: 2,
                title: "Add API routes".into(),
                description: "Create endpoints".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
        ];

        let dag = build_task_dag(&tasks).unwrap();
        let ready = dag.ready_tasks(&HashSet::new());
        assert_eq!(ready.len(), 2, "independent tasks should both be ready");
    }

    #[test]
    fn test_build_task_dag_with_dependencies() {
        let tasks = vec![
            PlanTask {
                id: "task-1".into(),
                phase: 1,
                title: "Create model".into(),
                description: "".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
            PlanTask {
                id: "task-2".into(),
                phase: 2,
                title: "Use model in API".into(),
                description: "".into(),
                depends_on: vec!["task-1".into()],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
            PlanTask {
                id: "task-3".into(),
                phase: 3,
                title: "Independent test".into(),
                description: "".into(),
                depends_on: vec![],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
        ];

        let dag = build_task_dag(&tasks).unwrap();

        // Initially: task-1 and task-3 are ready (no deps).
        let ready = dag.ready_tasks(&HashSet::new());
        let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
        assert!(ready_ids.contains(&"task-1"));
        assert!(ready_ids.contains(&"task-3"));
        assert!(!ready_ids.contains(&"task-2"));

        // After task-1 completes: task-2 becomes ready.
        let mut completed = HashSet::new();
        completed.insert("task-1".to_string());
        let ready = dag.ready_tasks(&completed);
        let ready_ids: Vec<&str> = ready.iter().map(|n| n.id.as_str()).collect();
        assert!(ready_ids.contains(&"task-2"));
        assert!(ready_ids.contains(&"task-3"));
    }

    #[test]
    fn test_build_task_dag_detects_cycle() {
        let tasks = vec![
            PlanTask {
                id: "task-1".into(),
                phase: 1,
                title: "A".into(),
                description: "".into(),
                depends_on: vec!["task-2".into()],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
            PlanTask {
                id: "task-2".into(),
                phase: 2,
                title: "B".into(),
                description: "".into(),
                depends_on: vec!["task-1".into()],
                status: TaskStatus::Pending,
                estimated_iterations: 10,
                key_files: vec![],
                acceptance_criteria: vec![],
            },
        ];

        assert!(build_task_dag(&tasks).is_err());
    }

    #[test]
    fn test_build_task_dag_detects_missing_dependency() {
        let tasks = vec![PlanTask {
            id: "task-1".into(),
            phase: 1,
            title: "A".into(),
            description: "".into(),
            depends_on: vec!["nonexistent".into()],
            status: TaskStatus::Pending,
            estimated_iterations: 10,
            key_files: vec![],
            acceptance_criteria: vec![],
        }];

        assert!(build_task_dag(&tasks).is_err());
    }

    #[test]
    fn test_resolve_max_parallel_phases_default_when_missing() {
        let args = serde_json::json!({ "request": "do something" });
        assert_eq!(
            resolve_max_parallel_phases(&args),
            DEFAULT_MAX_PARALLEL_PHASES
        );
    }

    #[test]
    fn test_resolve_max_parallel_phases_accepts_explicit_value() {
        let args = serde_json::json!({ "max_parallel_phases": 2 });
        assert_eq!(resolve_max_parallel_phases(&args), 2);
    }

    #[test]
    fn test_resolve_max_parallel_phases_clamps_to_ceiling() {
        let args = serde_json::json!({ "max_parallel_phases": 999 });
        assert_eq!(
            resolve_max_parallel_phases(&args),
            MAX_PARALLEL_PHASES_CEILING
        );
    }

    #[test]
    fn test_plan_review_decision_revise_serde_round_trip() {
        let decision = PlanReviewDecision::Revise {
            feedback: "reduce scope".into(),
        };
        let json = serde_json::to_string(&decision).unwrap();
        assert!(json.contains(r#""decision":"revise""#));
        assert!(json.contains(r#""feedback":"reduce scope""#));
        let parsed: PlanReviewDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, decision);
    }

    #[test]
    fn test_resolve_max_parallel_phases_zero_falls_back_to_default() {
        let args = serde_json::json!({ "max_parallel_phases": 0 });
        assert_eq!(
            resolve_max_parallel_phases(&args),
            DEFAULT_MAX_PARALLEL_PHASES
        );
    }

    #[test]
    fn test_resolve_max_parallel_phases_ignores_non_numeric() {
        let args = serde_json::json!({ "max_parallel_phases": "four" });
        assert_eq!(
            resolve_max_parallel_phases(&args),
            DEFAULT_MAX_PARALLEL_PHASES
        );
    }

    #[test]
    fn test_build_plan_writer_input_includes_required_fields() {
        let raw = build_plan_writer_input("refactor auth", "src/auth/", None);
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["task"], "refactor auth");
        assert_eq!(parsed["context"], "src/auth/");
        assert!(parsed.get("plan_path").is_none());
        assert!(parsed.get("revision_feedback").is_none());
    }

    #[test]
    fn test_build_plan_writer_input_with_empty_context() {
        let raw = build_plan_writer_input("task", "", None);
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["task"], "task");
        assert_eq!(parsed["context"], "");
    }

    #[test]
    fn test_build_plan_writer_input_with_revision_feedback() {
        let raw = build_plan_writer_input("task", "ctx", Some("make it smaller"));
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["task"], "task");
        assert_eq!(parsed["revision_feedback"], "make it smaller");
    }

    #[test]
    fn test_repair_json_escapes_unescaped_interior_quotes() {
        let input = r#"["text with "interior" quotes"]"#;
        let repaired = repair_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr[0].as_str().unwrap(), r#"text with "interior" quotes"#);
    }

    #[test]
    fn test_repair_json_preserves_valid_json() {
        let input = r#"{"key": "value", "arr": [1, 2]}"#;
        let repaired = repair_json(input);
        assert_eq!(repaired, input);
    }

    #[test]
    fn test_repair_json_handles_cjk_unescaped_quotes() {
        let input = concat!(
            r#"["normal step", "#,
            r#""record "#,
            "\"this CVE\"",
            r#" as conclusion"]"#,
        );
        let repaired = repair_json(input);
        let parsed: serde_json::Result<serde_json::Value> = serde_json::from_str(&repaired);
        assert!(parsed.is_ok(), "repaired JSON should parse: {repaired}");
    }

    #[test]
    fn test_parse_structured_plan_handles_bare_array() {
        let input = r#"[
            {
                "task_id": "phase_1",
                "title": "Search NVD",
                "objective": "Query the NVD database",
                "steps": ["step 1"],
                "depends_on": [],
                "estimated_iterations": 12,
                "key_files": [],
                "acceptance_criteria": ["found or not"],
                "status": "pending"
            }
        ]"#;
        let plan = parse_structured_plan(input).unwrap();
        assert_eq!(plan.plan_title, "Untitled Plan");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "phase_1");
        assert_eq!(plan.tasks[0].description, "Query the NVD database");
    }

    #[test]
    fn test_parse_structured_plan_aliases_task_id_and_objective() {
        let input = r#"{
            "plan_title": "Test",
            "plan_file": "",
            "tasks": [{
                "task_id": "p1",
                "phase": 1,
                "title": "Do work",
                "objective": "The objective",
                "depends_on": [],
                "status": "pending",
                "estimated_iterations": 10,
                "key_files": [],
                "acceptance_criteria": []
            }]
        }"#;
        let plan = parse_structured_plan(input).unwrap();
        assert_eq!(plan.tasks[0].id, "p1");
        assert_eq!(plan.tasks[0].description, "The objective");
    }

    #[test]
    fn test_repair_json_then_parse_bare_array_with_interior_quotes() {
        let raw = concat!(
            "```json\n",
            "[\n",
            "  {\n",
            r#"    "task_id": "phase_1","#,
            "\n",
            r#"    "title": "Search","#,
            "\n",
            r#"    "objective": "Find the CVE","#,
            "\n",
            r#"    "steps": ["check NVD", "if not found, record "#,
            "\"not found in DB\"",
            r#" as conclusion"],"#,
            "\n",
            r#"    "depends_on": [],"#,
            "\n",
            r#"    "estimated_iterations": 10,"#,
            "\n",
            r#"    "key_files": [],"#,
            "\n",
            r#"    "acceptance_criteria": ["done"],"#,
            "\n",
            r#"    "status": "pending""#,
            "\n",
            "  }\n",
            "]\n",
            "```",
        );
        let extracted = extract_json_from_response(raw);
        let repaired = repair_json(&extracted);
        let plan = parse_structured_plan(&repaired).unwrap();
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].id, "phase_1");
    }
}
