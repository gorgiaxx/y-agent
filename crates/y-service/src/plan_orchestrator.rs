//! Plan orchestrator: handles the `Plan` tool call by delegating to
//! sub-agents (`plan-writer`, `task-decomposer`) and executing phases
//! in child sessions.
//!
//! Follows the same pattern as `TaskDelegationOrchestrator` and
//! `ToolSearchOrchestrator` -- the `tool_dispatch` layer intercepts
//! `Plan` tool calls and routes them here.

use std::fmt::Write as _;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::{ToolError, ToolOutput};
use y_core::types::SessionId;

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError, AgentService};
use crate::chat::{TurnEvent, TurnEventSender};
use crate::container::ServiceContainer;

const PLAN_CANCELLED_MESSAGE: &str = "Cancelled";

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

impl PlanOrchestrator {
    /// Handle a `Plan` tool call.
    ///
    /// Workflow:
    /// 1. Create a child session for the `plan-writer` sub-agent
    /// 2. Execute plan-writer (codebase exploration + plan generation)
    /// 3. Create a child session for the `task-decomposer` sub-agent
    /// 4. Execute task-decomposer (structured JSON task list)
    /// 5. Execute each phase sequentially in its own child session
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

        // Phase 1: Plan writing
        tracing::info!(request = %request, "plan orchestrator: starting plan-writer");
        let plan_content = Self::run_plan_writer(
            container,
            parent_session_id,
            request,
            context,
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

        // Phase 3: Sequential execution
        tracing::info!(total_tasks, "plan orchestrator: starting phase execution");
        let mut phase_results = Vec::with_capacity(total_tasks);
        for (idx, task) in structured_plan.tasks.iter().enumerate() {
            if is_cancelled(cancel.as_ref()) {
                return Err(cancelled_tool_error());
            }

            tracing::info!(
                phase = idx + 1,
                title = %task.title,
                "plan orchestrator: executing phase"
            );

            match Self::run_phase(
                container,
                parent_session_id,
                task,
                &structured_plan.plan_title,
                idx + 1,
                total_tasks,
                progress,
                cancel.as_ref(),
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
                    // Continue with remaining phases despite failure.
                }
            }
        }

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

    /// Create a child session under the parent and run the plan-writer agent.
    async fn run_plan_writer(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        context: &str,
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

        // Build the user message for the plan-writer.
        let mut user_msg = format!("Create an execution plan for the following task:\n\n{request}");
        if !context.is_empty() {
            let _ = write!(user_msg, "\n\nAdditional context:\n{context}");
        }
        let _ = write!(user_msg, "\n\nWrite the plan to: {}", plan_path.display());

        let messages = vec![y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::User,
            content: user_msg,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];

        // Load agent definition for tool schemas.
        let tool_defs = Self::load_agent_tool_schemas(container, "plan-writer").await;

        let exec_config = AgentExecutionConfig {
            agent_name: "plan-writer".to_string(),
            system_prompt: String::new(), // Uses context pipeline.
            max_iterations: 30,
            tool_definitions: tool_defs,
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            messages,
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec!["general".to_string()],
            temperature: Some(0.3),
            max_tokens: None,
            thinking: None,
            session_id: Some(child_session.id.clone()),
            session_uuid: child_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false, // Sub-agent uses its own system prompt.
            user_query: request.to_string(),
            external_trace_id: None,
            trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
            agent_allowed_tools: vec![
                "FileRead".into(),
                "Glob".into(),
                "Grep".into(),
                "SearchCode".into(),
                "WebFetch".into(),
                "Browser".into(),
                "ShellExec".into(),
                "FileWrite".into(),
            ],
            prune_tool_history: true,
        };

        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error("plan-writer", e))?;

        // Try to read the plan file. If it doesn't exist, use the agent's
        // text output as the plan content.
        let plan_content = if plan_path.exists() {
            tokio::fs::read_to_string(plan_path)
                .await
                .unwrap_or_else(|_| result.content.clone())
        } else {
            result.content.clone()
        };

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

        // Load the agent's system prompt from registry.
        let system_prompt = {
            let registry = container.agent_registry.lock().await;
            registry
                .get("task-decomposer")
                .map(|d| d.system_prompt.clone())
                .unwrap_or_default()
        };

        let messages = vec![
            y_core::types::Message {
                message_id: y_core::types::generate_message_id(),
                role: y_core::types::Role::System,
                content: system_prompt,
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            y_core::types::Message {
                message_id: y_core::types::generate_message_id(),
                role: y_core::types::Role::User,
                content: format!("Plan file: {}\n\n{}", plan_path.display(), plan_content),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ];

        let exec_config = AgentExecutionConfig {
            agent_name: "task-decomposer".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            tool_definitions: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            messages,
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec!["general".to_string()],
            temperature: Some(0.0),
            max_tokens: None,
            thinking: None,
            session_id: Some(child_session.id.clone()),
            session_uuid: child_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: "decompose plan into tasks".to_string(),
            external_trace_id: None,
            trust_tier: Some(y_core::trust::TrustTier::BuiltIn),
            agent_allowed_tools: vec![],
            prune_tool_history: false,
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
                agent_id: None,
                title: Some(format!("Phase {phase_num}: {}", task.title)),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Plan".into(),
                message: format!("failed to create phase session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let user_msg = format!(
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
        );

        let messages = vec![y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::User,
            content: user_msg,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];

        // Phase executors get full tool access via the essential set.
        let tool_defs = crate::chat::ChatService::build_essential_tool_definitions(container).await;

        let exec_config = AgentExecutionConfig {
            agent_name: format!("phase-{phase_num}"),
            system_prompt: String::new(),
            max_iterations: task.estimated_iterations.max(10),
            tool_definitions: tool_defs,
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            messages,
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: Some(0.7),
            max_tokens: None,
            thinking: None,
            session_id: Some(child_session.id.clone()),
            session_uuid: child_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: true,
            user_query: format!("Phase {phase_num}: {}", task.title),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: true,
        };

        let phase_name = format!("phase-{phase_num}");
        let result =
            AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
                .await
                .map_err(|e| map_plan_agent_error(&phase_name, e))?;

        Ok(result.content)
    }

    /// Load tool schemas for a given agent from its definition.
    async fn load_agent_tool_schemas(
        container: &ServiceContainer,
        agent_name: &str,
    ) -> Vec<serde_json::Value> {
        let allowed_tools = {
            let registry = container.agent_registry.lock().await;
            registry
                .get(agent_name)
                .map(|d| d.allowed_tools.clone())
                .unwrap_or_default()
        };

        if allowed_tools.is_empty() {
            return vec![];
        }

        let mut defs = Vec::new();
        for tool_name in &allowed_tools {
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

fn build_plan_writer_stage_metadata(
    plan_path: &std::path::Path,
    plan_content: &str,
) -> serde_json::Value {
    let plan_title = extract_plan_title(plan_content).unwrap_or_else(|| "Plan".to_string());
    serde_json::json!({
        "display": {
            "kind": "plan_stage",
            "stage": "plan_writer",
            "plan_title": plan_title,
            "plan_file": plan_path.display().to_string(),
            "plan_content": plan_content,
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
    plan_path: &std::path::Path,
    plan: &StructuredPlan,
    completed: usize,
    failed: usize,
    phase_results: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "action": "plan_executed",
        "display": {
            "kind": "plan_execution",
            "plan_title": plan.plan_title,
            "plan_file": plan_path.display().to_string(),
            "total_phases": plan.tasks.len(),
            "completed": completed,
            "failed": failed,
            "tasks": plan.tasks,
            "phases": phase_results,
        }
    })
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
        tool_definitions: vec![],
        tool_calling_mode: y_core::provider::ToolCallingMode::Native,
        messages,
        provider_id: provider_id.map(String::from),
        preferred_models: vec![],
        provider_tags,
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
        let mut config = crate::config::ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
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
    fn test_extract_plan_title_prefers_frontmatter_title() {
        let plan = r#"---
title: GUI Plan Stream Fix
status: pending
---

## Overview
Fix the plan stream rendering.
"#;

        assert_eq!(
            extract_plan_title(plan).as_deref(),
            Some("GUI Plan Stream Fix")
        );
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
}
