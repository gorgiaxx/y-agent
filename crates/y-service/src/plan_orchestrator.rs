//! Plan orchestrator: handles the `Plan` tool call by delegating to
//! sub-agents (`plan-writer`, `task-decomposer`) and executing phases
//! in child sessions.
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
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::{ToolError, ToolOutput};
use y_core::trust::TrustTier;
use y_core::types::{Message, SessionId};

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError, AgentService};
use crate::chat::{TurnEvent, TurnEventSender};
use crate::container::ServiceContainer;

const PLAN_CANCELLED_MESSAGE: &str = "Cancelled";
const PLAN_ANALYST_AGENT_ID: &str = "plan-analyst";
const PHASE_EXECUTOR_AGENT_ID: &str = "plan-phase-executor";
/// Default maximum number of phases to execute concurrently.
const DEFAULT_MAX_PARALLEL_PHASES: usize = 4;
/// Hard upper bound to protect against runaway concurrency from caller input.
const MAX_PARALLEL_PHASES_CEILING: usize = 16;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// A structured task extracted from the plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanTask {
    pub id: String,
    pub phase: usize,
    pub title: String,
    pub description: String,
    pub depends_on: Vec<String>,
    pub status: TaskStatus,
    pub estimated_iterations: usize,
    pub key_files: Vec<String>,
    pub acceptance_criteria: Vec<String>,
}

/// Structured plan output from the task decomposer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredPlan {
    pub plan_title: String,
    #[serde(default)]
    pub plan_file: String,
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
}

impl PlanOrchestrator {
    /// Handle a `Plan` tool call.
    ///
    /// Workflow:
    /// 0. (optional) Run plan-analyst for pre-planning analysis
    /// 1. Create a child session for the `plan-writer` sub-agent
    /// 2. Execute plan-writer (codebase exploration + plan generation)
    /// 3. Create a child session for the `task-decomposer` sub-agent
    /// 4. Execute task-decomposer (structured JSON task list)
    /// 5. Execute phases (parallel when dependencies allow, sequential fallback)
    /// 6. Return consolidated results
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

        // Phase 0: Pre-planning analysis (optional)
        let skip_analysis = arguments
            .get("skip_analysis")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let analysis = if skip_analysis {
            String::new()
        } else {
            tracing::info!(
                request = %request,
                "plan orchestrator: starting plan-analyst"
            );
            match Self::run_plan_analyst(
                container,
                parent_session_id,
                request,
                context,
                progress,
                cancel.as_ref(),
            )
            .await
            {
                Ok(result) => result,
                Err(e) => {
                    // Non-fatal: log and continue without analysis.
                    tracing::warn!(
                        error = %e,
                        "plan-analyst failed, continuing without analysis"
                    );
                    String::new()
                }
            }
        };

        // Phase 1: Plan writing
        tracing::info!(request = %request, "plan orchestrator: starting plan-writer");
        let plan_content = Self::run_plan_writer(
            container,
            parent_session_id,
            request,
            context,
            &analysis,
            &plan_path,
            progress,
            cancel.as_ref(),
        )
        .await?;

        // Phase 2: Task decomposition
        tracing::info!("plan orchestrator: starting task-decomposer");
        let structured_plan = Self::run_task_decomposer(
            container,
            parent_session_id,
            &plan_content,
            &plan_path,
            progress,
            cancel.as_ref(),
        )
        .await?;

        let total_tasks = structured_plan.tasks.len();

        let max_parallel = resolve_max_parallel_phases(arguments);

        // Phase 3: Dependency-aware parallel execution
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
                "phases": phase_results,
            }),
            warnings: vec![],
            metadata,
        })
    }

    /// Create a child session and run the plan-analyst agent.
    ///
    /// Returns the analyst's structured analysis text. The caller should
    /// treat failures as non-fatal (analysis is optional enrichment).
    async fn run_plan_analyst(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        context: &str,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, ToolError> {
        if is_cancelled(cancel) {
            return Err(cancelled_tool_error());
        }

        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string(PLAN_ANALYST_AGENT_ID)),
                title: Some("Plan Analyst".to_string()),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to create plan-analyst session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            PLAN_ANALYST_AGENT_ID,
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 10,
                max_tool_calls: 10,
                preferred_models: vec![],
                provider_tags: vec!["general".to_string()],
                temperature: Some(0.3),
                max_tokens: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                allowed_tools: vec!["FileRead".into(), "Glob".into(), "Grep".into()],
                prune_tool_history: false,
            },
        )
        .await;

        let user_msg = if context.is_empty() {
            format!("Analyze this request for pre-planning:\n\n{request}")
        } else {
            format!(
                "Analyze this request for pre-planning:\n\n\
                 Request: {request}\n\n\
                 Context:\n{context}"
            )
        };

        let messages = build_subagent_messages(&settings.system_prompt, user_msg);
        let tool_defs =
            Self::load_tool_schemas_for_allowed_tools(container, &settings.allowed_tools).await;

        let exec_config = AgentExecutionConfig {
            agent_name: PLAN_ANALYST_AGENT_ID.to_string(),
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
            response_format: None,
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error(PLAN_ANALYST_AGENT_ID, e))?;

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: "plan-analyst completed".into(),
                result_preview: "Pre-planning analysis done".into(),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: None,
            });
        }

        Ok(result.content)
    }

    /// Create a child session under the parent and run the plan-writer agent.
    async fn run_plan_writer(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        context: &str,
        analysis: &str,
        plan_path: &std::path::Path,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, ToolError> {
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
                max_iterations: 12,
                max_tool_calls: 8,
                preferred_models: vec![],
                provider_tags: vec!["general".to_string()],
                temperature: Some(0.3),
                max_tokens: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                allowed_tools: vec!["FileRead".into(), "Glob".into(), "Grep".into()],
                prune_tool_history: false,
            },
        )
        .await;

        // Build the user message for the plan-writer as structured JSON.
        let user_msg = build_plan_writer_input(request, context, plan_path, analysis);

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
            response_format: None,
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error("plan-writer", e))?;

        // Prefer the content already present in the FileWrite tool call so we
        // can pass it directly to task-decomposer without re-reading from
        // disk. Fall back to the written file, then to the agent text output.
        let plan_content =
            extract_plan_content_from_tool_calls(&result.tool_calls_executed, plan_path)
                .or_else(|| {
                    if plan_path.exists() {
                        std::fs::read_to_string(plan_path).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| result.content.clone());

        if let Err(error) = persist_plan_content(plan_path, &plan_content).await {
            tracing::warn!(path = %plan_path.display(), %error, "failed to persist generated plan");
        }

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: "plan-writer completed".into(),
                result_preview: format!("Plan written to {}", plan_path.display()),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: Some(build_plan_writer_stage_metadata(plan_path, &plan_content)),
            });
        }

        Ok(plan_content)
    }

    /// Create a child session and run the task-decomposer agent.
    async fn run_task_decomposer(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        plan_content: &str,
        plan_path: &std::path::Path,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<StructuredPlan, ToolError> {
        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string("task-decomposer")),
                title: Some("Task Decomposer".to_string()),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to create task-decomposer session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            "task-decomposer",
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 1,
                max_tool_calls: 0,
                preferred_models: vec![],
                provider_tags: vec!["general".to_string()],
                temperature: Some(0.0),
                max_tokens: None,
                trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
                allowed_tools: vec![],
                prune_tool_history: false,
            },
        )
        .await;

        let messages = build_subagent_messages(
            &settings.system_prompt,
            format!("Plan file: {}\n\n{}", plan_path.display(), plan_content),
        );

        let exec_config = AgentExecutionConfig {
            agent_name: "task-decomposer".to_string(),
            system_prompt: settings.system_prompt.clone(),
            max_iterations: settings.max_iterations,
            max_tool_calls: settings.max_tool_calls,
            tool_definitions: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            messages,
            provider_id: None,
            preferred_models: settings.preferred_models.clone(),
            provider_tags: settings.provider_tags.clone(),
            request_mode: y_core::provider::RequestMode::TextChat,
            temperature: settings.temperature,
            max_tokens: settings.max_tokens,
            thinking: None,
            session_id: Some(child_session.id.clone()),
            session_uuid: child_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: "decompose plan into tasks".to_string(),
            external_trace_id: None,
            trust_tier: settings.trust_tier,
            agent_allowed_tools: settings.allowed_tools.clone(),
            prune_tool_history: settings.prune_tool_history,
            response_format: None,
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error("task-decomposer", e))?;

        // Parse the JSON output. Try to extract from markdown code block if
        // the LLM wrapped it.
        let json_text = extract_json_from_response(&result.content);
        let plan: StructuredPlan = serde_json::from_str(&json_text).map_err(|e| {
            tracing::error!(
                raw_output = %result.content,
                error = %e,
                "failed to parse task-decomposer output"
            );
            ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to parse task-decomposer output: {e}"),
            }
        })?;

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: "Plan".into(),
                success: true,
                duration_ms: 0,
                input_preview: "task-decomposer completed".into(),
                result_preview: format!("{} tasks extracted", plan.tasks.len()),
                agent_name: "plan-orchestrator".into(),
                url_meta: None,
                metadata: Some(build_task_decomposer_stage_metadata(plan_path, &plan)),
            });
        }

        Ok(plan)
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

        Ok(phase_results)
    }

    /// Execute a single phase in its own child session.
    async fn run_phase(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        task: &PlanTask,
        plan_title: &str,
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
            phase_num,
            total_phases,
            tool_defs,
        );

        let phase_name = format!("phase-{phase_num}");
        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error(&phase_name, e))?;

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
    }
}

fn extract_plan_title(plan_content: &str) -> Option<String> {
    let trimmed = plan_content.trim();

    if let Some(rest) = trimmed.strip_prefix("---") {
        for line in rest.lines() {
            let line = line.trim();
            if line == "---" {
                break;
            }
            if let Some(title) = line.strip_prefix("title:") {
                let title = title.trim().trim_matches('"').trim_matches('\'');
                if !title.is_empty() {
                    return Some(title.to_string());
                }
            }
        }
    }

    trimmed.lines().find_map(|line| {
        let heading = line.trim().trim_start_matches('#').trim();
        if heading.is_empty() || heading == line.trim() {
            None
        } else {
            Some(heading.to_string())
        }
    })
}

fn extract_plan_content_from_tool_calls(
    tool_calls: &[crate::chat::ToolCallRecord],
    plan_path: &Path,
) -> Option<String> {
    tool_calls.iter().rev().find_map(|call| {
        if call.name != "FileWrite" {
            return None;
        }

        let args: serde_json::Value = serde_json::from_str(&call.arguments).ok()?;
        let path = args.get("path").and_then(|value| value.as_str())?;
        if Path::new(path) != plan_path {
            return None;
        }

        args.get("content")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
    })
}

fn build_plan_writer_stage_metadata(
    plan_path: &std::path::Path,
    plan_content: &str,
) -> serde_json::Value {
    let plan_title = extract_plan_title(plan_content).unwrap_or_else(|| "Plan".to_string());
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "stage_status": "completed",
            "plan_title": plan_title,
            "plan_file": plan_path.display().to_string(),
            "plan_content": plan_content,
        }
    })
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

fn build_task_decomposer_stage_metadata(
    plan_path: &std::path::Path,
    plan: &StructuredPlan,
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "task_decomposer",
            "stage_status": "completed",
            "plan_title": plan.plan_title,
            "plan_file": if plan.plan_file.is_empty() {
                plan_path.display().to_string()
            } else {
                plan.plan_file.clone()
            },
            "tasks": plan.tasks,
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
/// Always includes `task`, `context`, and `plan_path`. Includes `analysis`
/// only when non-empty so plan-writer can detect whether pre-planning
/// analysis is available.
fn build_plan_writer_input(
    request: &str,
    context: &str,
    plan_path: &Path,
    analysis: &str,
) -> String {
    let mut input = serde_json::json!({
        "task": request,
        "context": context,
        "plan_path": plan_path.display().to_string(),
    });
    if !analysis.is_empty() {
        input["analysis"] = serde_json::Value::String(analysis.to_string());
    }
    input.to_string()
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
) -> bool {
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
    };

    match AgentService::execute(container, &exec_config, None, None).await {
        Ok(result) => {
            let response = result.content.trim().to_lowercase();
            tracing::debug!(
                classifier_response = %response,
                "plan mode complexity assessment"
            );
            response.contains("plan")
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "complexity assessment failed, defaulting to fast mode"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

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

    #[test]
    fn test_extract_plan_title_prefers_frontmatter_title() {
        let plan = r"---
title: GUI Plan Stream Fix
status: pending
---

## Overview
Fix the plan stream rendering.
";

        assert_eq!(
            extract_plan_title(plan).as_deref(),
            Some("GUI Plan Stream Fix")
        );
    }

    #[test]
    fn test_extract_plan_content_from_tool_calls_prefers_matching_file_write() {
        let tool_calls = vec![
            crate::chat::ToolCallRecord {
                name: "FileWrite".into(),
                arguments: serde_json::json!({
                    "path": "/tmp/other-plan.md",
                    "content": "# Other Plan",
                })
                .to_string(),
                success: true,
                duration_ms: 10,
                result_content: "{}".into(),
                url_meta: None,
                metadata: None,
            },
            crate::chat::ToolCallRecord {
                name: "FileWrite".into(),
                arguments: serde_json::json!({
                    "path": "/tmp/gui-plan.md",
                    "content": "# GUI Plan Stream Fix",
                })
                .to_string(),
                success: true,
                duration_ms: 12,
                result_content: "{}".into(),
                url_meta: None,
                metadata: None,
            },
        ];

        let content = extract_plan_content_from_tool_calls(
            &tool_calls,
            std::path::Path::new("/tmp/gui-plan.md"),
        );

        assert_eq!(content.as_deref(), Some("# GUI Plan Stream Fix"));
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
    fn test_build_task_decomposer_stage_metadata_includes_tasks() {
        let plan = StructuredPlan {
            plan_title: "GUI Plan Stream Fix".into(),
            plan_file: "/tmp/gui-plan.md".into(),
            tasks: vec![PlanTask {
                id: "task-1".into(),
                phase: 1,
                title: "Render task decomposer output".into(),
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

        let meta =
            build_task_decomposer_stage_metadata(std::path::Path::new("/tmp/gui-plan.md"), &plan);

        assert_eq!(meta["display"]["kind"], "plan_stage");
        assert_eq!(meta["display"]["stage"], "task_decomposer");
        assert_eq!(meta["display"]["plan_title"], "GUI Plan Stream Fix");
        assert_eq!(
            meta["display"]["tasks"][0]["title"],
            "Render task decomposer output"
        );
    }

    #[test]
    fn test_build_plan_execution_metadata_updates_task_statuses() {
        let plan = StructuredPlan {
            plan_title: "GUI Plan Stream Fix".into(),
            plan_file: "/tmp/gui-plan.md".into(),
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
    async fn test_run_task_decomposer_stops_when_cancelled() {
        let (container, tmpdir) = make_test_container().await;
        let parent = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("parent".into()),
            })
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let plan_path = tmpdir.path().join("plan.md");

        let error = PlanOrchestrator::run_task_decomposer(
            &container,
            &parent.id,
            "# Plan\n\n- Step 1",
            &plan_path,
            None,
            Some(&cancel),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error,
            ToolError::RuntimeError { ref message, .. } if message == PLAN_CANCELLED_MESSAGE
        ));
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
            },
        )
        .await;

        assert!(config.system_prompt.contains("You are a plan writer"));
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.max_tool_calls, 50);
        assert_eq!(config.provider_tags, vec!["general"]);
        assert_eq!(
            config.allowed_tools,
            vec![
                "FileRead".to_string(),
                "Glob".to_string(),
                "Grep".to_string()
            ]
        );
        assert!(!config.allowed_tools.iter().any(|tool| tool == "FileWrite"));
        assert!(!config.allowed_tools.iter().any(|tool| tool == "ShellExec"));
        assert!(!config.prune_tool_history);
    }

    #[tokio::test]
    async fn test_resolve_agent_config_uses_task_decomposer_definition() {
        let (container, _tmpdir) = make_test_container().await;
        let config = PlanOrchestrator::resolve_agent_config(
            &container,
            "task-decomposer",
            ResolvedAgentConfig {
                system_prompt: String::new(),
                max_iterations: 1,
                max_tool_calls: 1,
                preferred_models: vec![],
                provider_tags: vec![],
                temperature: None,
                max_tokens: None,
                trust_tier: None,
                allowed_tools: vec!["FallbackTool".into()],
                prune_tool_history: true,
            },
        )
        .await;

        assert!(config.system_prompt.contains("Output ONLY valid JSON"));
        assert_eq!(config.max_iterations, 50);
        assert_eq!(config.max_tool_calls, 0);
        assert_eq!(config.allowed_tools, Vec::<String>::new());
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
    fn test_build_plan_writer_input_includes_analysis_when_present() {
        let plan_path = Path::new("/tmp/plan.md");
        let raw = build_plan_writer_input(
            "refactor auth",
            "src/auth/",
            plan_path,
            "intent: REFACTORING\ngap: none",
        );
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["task"], "refactor auth");
        assert_eq!(parsed["context"], "src/auth/");
        assert_eq!(parsed["plan_path"], "/tmp/plan.md");
        assert_eq!(parsed["analysis"], "intent: REFACTORING\ngap: none");
    }

    #[test]
    fn test_build_plan_writer_input_omits_analysis_when_empty() {
        let plan_path = Path::new("/tmp/plan.md");
        let raw = build_plan_writer_input("task", "ctx", plan_path, "");
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed.get("analysis").is_none());
        assert_eq!(parsed["task"], "task");
    }
}
