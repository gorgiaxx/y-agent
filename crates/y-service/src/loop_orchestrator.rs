//! Loop orchestrator: handles the `Loop` tool call by spawning fresh
//! sub-agent rounds that iteratively converge toward the user's goal.
//!
//! Each round reads a persistent progress file (`PROGRESS.md`), works on
//! remaining tasks, updates the file, and optionally signals convergence.
//! A mandatory self-review round verifies completion before stopping.
//!
//! Follows the same orchestration pattern as `PlanOrchestrator` --
//! the `tool_dispatch` layer intercepts `Loop` tool calls and routes
//! them here.

use std::path::Path;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_agent::AgentDefinition;
use y_core::provider::ResponseFormat;
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::{ToolError, ToolOutput};
use y_core::trust::TrustTier;
use y_core::types::{Message, SessionId};

use y_diagnostics::DiagnosticsEvent;

use crate::agent_service::{AgentExecutionConfig, AgentService};
use crate::chat::{TurnEvent, TurnEventSender};
use crate::container::ServiceContainer;

const DEFAULT_MAX_ROUNDS: usize = 10;
const MAX_ROUNDS_CEILING: usize = 25;
const MIN_MAX_ROUNDS: usize = 2;
const LOOP_EXECUTOR_AGENT_ID: &str = "loop-executor";

// ---------------------------------------------------------------------------
// Loop data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopOutcome {
    Converged,
    BudgetExhausted,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoundSummary {
    pub round: usize,
    pub status: String,
    pub tasks_completed: Vec<String>,
    pub tasks_remaining: Vec<String>,
    pub converged: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct ProgressFrontMatter {
    title: String,
    status: String,
    total_rounds: usize,
    max_rounds: usize,
    converged: bool,
}

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
    #[allow(dead_code)]
    response_format: Option<ResponseFormat>,
}

// ---------------------------------------------------------------------------
// Loop orchestrator
// ---------------------------------------------------------------------------

pub struct LoopOrchestrator;

impl LoopOrchestrator {
    /// Handle a `Loop` tool call.
    ///
    /// Workflow:
    /// 1. Initialize a progress file
    /// 2. Run rounds (each a fresh `SubAgent` session)
    /// 3. On convergence, run a self-review round
    /// 4. Return consolidated results
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

        let max_rounds = arguments
            .get("max_rounds")
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_MAX_ROUNDS, |v| {
                (v as usize).clamp(MIN_MAX_ROUNDS, MAX_ROUNDS_CEILING)
            });

        // Create progress directory and file.
        let slug = slug_from_request(request);
        let loop_dir = container.data_dir.join("loop").join(&slug);
        if let Err(e) = tokio::fs::create_dir_all(&loop_dir).await {
            tracing::warn!(error = %e, "failed to create loop directory");
        }
        let progress_path = loop_dir.join("PROGRESS.md");

        let initial_content = initialize_progress_file(request, context, max_rounds);
        if let Err(e) = tokio::fs::write(&progress_path, &initial_content).await {
            return Err(ToolError::RuntimeError {
                name: "Loop".into(),
                message: format!("failed to write progress file: {e}"),
            });
        }

        // Emit init event.
        emit_loop_event(
            progress,
            true,
            serde_json::json!({
                "display": {
                    "kind": "loop_init",
                    "request": request,
                    "progress_file": progress_path.display().to_string(),
                    "max_rounds": max_rounds,
                }
            }),
        );

        // Round loop.
        let mut round_summaries: Vec<RoundSummary> = Vec::new();
        let mut outcome = LoopOutcome::BudgetExhausted;

        let mut round_num = 1usize;
        while round_num <= max_rounds {
            if is_cancelled(cancel.as_ref()) {
                outcome = LoopOutcome::Cancelled;
                break;
            }

            let round_start = std::time::Instant::now();

            let round_result = Self::run_round(
                container,
                parent_session_id,
                request,
                &progress_path,
                round_num,
                max_rounds,
                progress,
                cancel.as_ref(),
            )
            .await;

            let duration_ms = round_start.elapsed().as_millis() as u64;

            match round_result {
                Ok(()) => {
                    let progress_content = read_progress_file(&progress_path).await?;
                    let fm = parse_progress_front_matter(&progress_content);
                    let (done, in_progress, todo) = extract_task_lists(&progress_content);

                    let mut all_remaining = in_progress.clone();
                    all_remaining.extend(todo.clone());

                    let summary = RoundSummary {
                        round: round_num,
                        status: "completed".into(),
                        tasks_completed: done.clone(),
                        tasks_remaining: all_remaining.clone(),
                        converged: fm.converged,
                        duration_ms,
                    };
                    round_summaries.push(summary);

                    emit_loop_event(
                        progress,
                        true,
                        build_round_metadata(
                            round_num,
                            max_rounds,
                            "completed",
                            &done,
                            &all_remaining,
                            fm.converged,
                            &round_summaries,
                        ),
                    );

                    if fm.converged {
                        // Run self-review round.
                        let review_passed = Self::run_review(
                            container,
                            parent_session_id,
                            request,
                            &progress_path,
                            round_num,
                            progress,
                            cancel.as_ref(),
                        )
                        .await?;

                        emit_loop_event(
                            progress,
                            true,
                            serde_json::json!({
                                "display": {
                                    "kind": "loop_review",
                                    "review_status": if review_passed { "passed" } else { "failed" },
                                    "total_rounds": round_num,
                                }
                            }),
                        );

                        if review_passed {
                            outcome = LoopOutcome::Converged;
                            break;
                        }
                        // Review failed -- continue loop.
                    }
                }
                Err(e) => {
                    tracing::warn!(round = round_num, error = %e, "loop round failed");
                    let summary = RoundSummary {
                        round: round_num,
                        status: "failed".into(),
                        tasks_completed: vec![],
                        tasks_remaining: vec![],
                        converged: false,
                        duration_ms,
                    };
                    round_summaries.push(summary);

                    emit_loop_event(
                        progress,
                        false,
                        build_round_metadata(
                            round_num,
                            max_rounds,
                            "failed",
                            &[],
                            &[],
                            false,
                            &round_summaries,
                        ),
                    );
                }
            }

            round_num += 1;
        }

        // Emit final event.
        let total_rounds = round_summaries.len();
        emit_loop_event(
            progress,
            outcome == LoopOutcome::Converged,
            serde_json::json!({
                "display": {
                    "kind": "loop_execution",
                    "final_status": outcome,
                    "total_rounds": total_rounds,
                    "max_rounds": max_rounds,
                    "progress_file": progress_path.display().to_string(),
                    "rounds": round_summaries,
                }
            }),
        );

        Ok(ToolOutput {
            success: outcome == LoopOutcome::Converged,
            content: serde_json::json!({
                "outcome": outcome,
                "total_rounds": total_rounds,
                "max_rounds": max_rounds,
                "progress_file": progress_path.display().to_string(),
                "rounds": round_summaries,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Execute a single round: create a fresh `SubAgent` session, run the
    /// loop-executor agent with the progress file content as input.
    async fn run_round(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        progress_path: &Path,
        round_num: usize,
        max_rounds: usize,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), ToolError> {
        let progress_content = read_progress_file(progress_path).await?;

        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string(LOOP_EXECUTOR_AGENT_ID)),
                title: Some(format!("Loop Round {round_num}")),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Loop".into(),
                message: format!("failed to create round session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            LOOP_EXECUTOR_AGENT_ID,
            default_executor_config(),
        )
        .await;

        let user_msg = format!(
            "Round {round_num} of {max_rounds}.\n\n\
             Progress file path: {}\n\n\
             Current progress file content:\n\
             ```\n{progress_content}\n```\n\n\
             Work on the remaining tasks. Update the progress file when done.",
            progress_path.display(),
        );

        let messages = build_subagent_messages(&settings.system_prompt, user_msg);
        let tool_defs =
            Self::load_tool_schemas_for_allowed_tools(container, &settings.allowed_tools).await;

        let exec_config = AgentExecutionConfig {
            agent_name: LOOP_EXECUTOR_AGENT_ID.to_string(),
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
            image_generation_options: None,
        };

        AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
            .await
            .map_err(|e| map_loop_agent_error("loop-executor", &e))?;

        emit_subagent_completed(container, child_uuid, LOOP_EXECUTOR_AGENT_ID, true);

        Ok(())
    }

    /// Run a self-review round: spawn a fresh session that critically reviews
    /// the progress file and either confirms or reverts convergence.
    async fn run_review(
        container: &ServiceContainer,
        parent_session_id: &SessionId,
        request: &str,
        progress_path: &Path,
        total_rounds: usize,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<bool, ToolError> {
        let progress_content = read_progress_file(progress_path).await?;

        let child_session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: Some(parent_session_id.clone()),
                session_type: SessionType::SubAgent,
                agent_id: Some(y_core::types::AgentId::from_string(LOOP_EXECUTOR_AGENT_ID)),
                title: Some("Loop Self-Review".to_string()),
            })
            .await
            .map_err(|e| ToolError::RuntimeError {
                name: "Loop".into(),
                message: format!("failed to create review session: {e}"),
            })?;

        let child_uuid =
            Uuid::parse_str(child_session.id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        let settings = Self::resolve_agent_config(
            container,
            LOOP_EXECUTOR_AGENT_ID,
            default_executor_config(),
        )
        .await;

        let review_msg = format!(
            "SELF-REVIEW ROUND after {total_rounds} execution rounds.\n\n\
             Progress file path: {}\n\n\
             Current progress file content:\n\
             ```\n{progress_content}\n```\n\n\
             You are reviewing the work done so far. Be critical and skeptical.\n\
             [DONE] tasks may be incomplete or low quality from earlier rounds.\n\n\
             Instructions:\n\
             1. Read the progress file carefully.\n\
             2. Verify each [DONE] task is truly complete and correct.\n\
             3. Check the original request is fully satisfied.\n\
             4. If issues are found: set `converged: false` in front matter, \
                add new [TODO] tasks for the issues.\n\
             5. If everything is truly done: keep `converged: true`.\n\
             6. Update the progress file via FileWrite with your review findings.",
            progress_path.display(),
        );

        let messages = build_subagent_messages(&settings.system_prompt, review_msg);
        let tool_defs =
            Self::load_tool_schemas_for_allowed_tools(container, &settings.allowed_tools).await;

        let exec_config = AgentExecutionConfig {
            agent_name: LOOP_EXECUTOR_AGENT_ID.to_string(),
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
            image_generation_options: None,
        };

        AgentService::execute(container, &exec_config, progress.cloned(), cancel.cloned())
            .await
            .map_err(|e| map_loop_agent_error("loop-review", &e))?;

        emit_subagent_completed(container, child_uuid, "loop-review", true);

        // Re-read progress file and check if convergence was maintained.
        let updated_content = read_progress_file(progress_path).await?;
        let fm = parse_progress_front_matter(&updated_content);
        Ok(fm.converged)
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
        config_from_definition(def)
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

fn default_executor_config() -> ResolvedAgentConfig {
    ResolvedAgentConfig {
        system_prompt: String::new(),
        max_iterations: 60,
        max_tool_calls: 100,
        preferred_models: vec![],
        provider_tags: vec![],
        temperature: Some(0.7),
        max_tokens: None,
        trust_tier: Some(TrustTier::BuiltIn),
        allowed_tools: vec![
            "ToolSearch".into(),
            "FileRead".into(),
            "FileWrite".into(),
            "ShellExec".into(),
            "WebFetch".into(),
            "Browser".into(),
            "Glob".into(),
            "Grep".into(),
        ],
        prune_tool_history: false,
        response_format: None,
    }
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
        format!("loop-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))
    } else {
        slug
    }
}

fn initialize_progress_file(request: &str, context: &str, max_rounds: usize) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let context_section = if context.is_empty() {
        String::new()
    } else {
        format!("\n\n{context}")
    };

    format!(
        "---\n\
         title: \"\"\n\
         status: initial\n\
         total_rounds: 0\n\
         max_rounds: {max_rounds}\n\
         converged: false\n\
         created_at: \"{now}\"\n\
         updated_at: \"{now}\"\n\
         ---\n\n\
         ## Original Request\n\n\
         {request}{context_section}\n\n\
         ## Tasks\n\n\
         ## Insights\n\n\
         ## Round Log\n"
    )
}

fn parse_progress_front_matter(content: &str) -> ProgressFrontMatter {
    let mut fm = ProgressFrontMatter::default();
    let trimmed = content.trim();

    let Some(rest) = trimmed.strip_prefix("---") else {
        return fm;
    };

    for line in rest.lines() {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(val) = line.strip_prefix("title:") {
            fm.title = val.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(val) = line.strip_prefix("status:") {
            fm.status = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("total_rounds:") {
            fm.total_rounds = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("max_rounds:") {
            fm.max_rounds = val.trim().parse().unwrap_or(DEFAULT_MAX_ROUNDS);
        } else if let Some(val) = line.strip_prefix("converged:") {
            fm.converged = val.trim().eq_ignore_ascii_case("true");
        }
    }
    fm
}

fn extract_task_lists(content: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut done = Vec::new();
    let mut in_progress = Vec::new();
    let mut todo = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("### [DONE]") {
            done.push(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("### [IN PROGRESS]") {
            in_progress.push(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("### [TODO]") {
            todo.push(rest.trim().to_string());
        }
    }

    (done, in_progress, todo)
}

async fn read_progress_file(path: &Path) -> Result<String, ToolError> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ToolError::RuntimeError {
            name: "Loop".into(),
            message: format!("failed to read progress file: {e}"),
        })
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

fn emit_loop_event(progress: Option<&TurnEventSender>, success: bool, metadata: serde_json::Value) {
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::ToolResult {
            name: "Loop".into(),
            success,
            duration_ms: 0,
            input_preview: String::new(),
            result_preview: String::new(),
            agent_name: "loop-orchestrator".into(),
            url_meta: None,
            metadata: Some(metadata),
        });
    }
}

fn build_round_metadata(
    round: usize,
    max_rounds: usize,
    status: &str,
    tasks_completed: &[String],
    tasks_remaining: &[String],
    converged: bool,
    round_summaries: &[RoundSummary],
) -> serde_json::Value {
    serde_json::json!({
        "display": {
            "kind": "loop_round",
            "round": round,
            "max_rounds": max_rounds,
            "round_status": status,
            "tasks_completed": tasks_completed,
            "tasks_remaining": tasks_remaining,
            "converged": converged,
            "rounds": round_summaries,
        }
    })
}

fn is_cancelled(cancel: Option<&CancellationToken>) -> bool {
    cancel.is_some_and(CancellationToken::is_cancelled)
}

fn cancelled_tool_error() -> ToolError {
    ToolError::RuntimeError {
        name: "Loop".into(),
        message: "Cancelled".into(),
    }
}

fn map_loop_agent_error(
    agent_name: &str,
    error: &crate::agent_service::AgentExecutionError,
) -> ToolError {
    ToolError::RuntimeError {
        name: "Loop".into(),
        message: format!("{agent_name} failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_progress_file() {
        let content = initialize_progress_file("Build a CLI tool", "Using Rust", 10);
        assert!(content.contains("## Original Request"));
        assert!(content.contains("Build a CLI tool"));
        assert!(content.contains("Using Rust"));
        assert!(content.contains("max_rounds: 10"));
        assert!(content.contains("converged: false"));
        assert!(content.contains("status: initial"));
    }

    #[test]
    fn test_parse_progress_front_matter() {
        let content = "\
---
title: \"My Task\"
status: running
total_rounds: 3
max_rounds: 10
converged: true
created_at: \"2026-05-11T12:00:00Z\"
updated_at: \"2026-05-11T12:05:00Z\"
---

## Original Request
";
        let fm = parse_progress_front_matter(content);
        assert_eq!(fm.title, "My Task");
        assert_eq!(fm.status, "running");
        assert_eq!(fm.total_rounds, 3);
        assert_eq!(fm.max_rounds, 10);
        assert!(fm.converged);
    }

    #[test]
    fn test_parse_progress_front_matter_missing() {
        let fm = parse_progress_front_matter("No front matter here");
        assert_eq!(fm.title, "");
        assert!(!fm.converged);
    }

    #[test]
    fn test_parse_progress_front_matter_false() {
        let content = "\
---
converged: false
status: initial
---
";
        let fm = parse_progress_front_matter(content);
        assert!(!fm.converged);
        assert_eq!(fm.status, "initial");
    }

    #[test]
    fn test_extract_task_lists() {
        let content = "\
## Tasks

### [DONE] Design schema
- Details here

### [IN PROGRESS] Implement API
- In progress details

### [TODO] Write tests
### [TODO] Deploy
";
        let (done, in_progress, todo) = extract_task_lists(content);
        assert_eq!(done, vec!["Design schema"]);
        assert_eq!(in_progress, vec!["Implement API"]);
        assert_eq!(todo, vec!["Write tests", "Deploy"]);
    }

    #[test]
    fn test_extract_task_lists_empty() {
        let (done, in_progress, todo) = extract_task_lists("No tasks here");
        assert!(done.is_empty());
        assert!(in_progress.is_empty());
        assert!(todo.is_empty());
    }

    #[test]
    fn test_slug_from_request() {
        assert_eq!(
            slug_from_request("Build a distributed task queue"),
            "build-a-distributed-task-queue"
        );
    }

    #[test]
    fn test_slug_from_request_special_chars() {
        assert_eq!(
            slug_from_request("Fix bug #123 in the API!"),
            "fix-bug--123-in-the-api"
        );
    }

    #[test]
    fn test_slug_from_request_empty() {
        let slug = slug_from_request("");
        assert!(slug.starts_with("loop-"));
    }
}
