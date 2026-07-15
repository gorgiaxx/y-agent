//! Sub-agent runner and prompt construction.
//!
//! Contains `ServiceAgentRunner` (bridges `AgentPool.delegate()` to
//! `AgentService.execute()`) and `build_subagent_system_prompt`.

use std::sync::Arc;

use uuid::Uuid;

use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::ToolCallingMode;
use y_core::runtime::{RuntimeAdapter, RuntimeBackend};
use y_core::template::RuntimeTemplateVars;
use y_core::types::{Message, Role};

use crate::container::ServiceContainer;

use super::{AgentExecutionConfig, AgentExecutionError, AgentService};

// ---------------------------------------------------------------------------
// Sub-agent prompt augmentation
// ---------------------------------------------------------------------------

/// Build the effective system prompt for a sub-agent.
///
/// When `filtered_defs` is empty the base prompt is returned unchanged.
///
/// In [`ToolCallingMode::Native`] the base prompt is returned unchanged
/// because tools are sent via the API `tools` field -- no prompt injection
/// needed.
///
/// In [`ToolCallingMode::PromptBased`] the XML tool protocol and an
/// available-tools summary table are appended to the base prompt.
pub(crate) fn build_subagent_system_prompt(
    base_prompt: &str,
    filtered_defs: &[y_core::tool::ToolDefinition],
    tool_calling_mode: ToolCallingMode,
    tool_dialect: y_core::provider::ToolDialect,
    runtime_backend: RuntimeBackend,
    template_vars: &RuntimeTemplateVars,
) -> String {
    let base = if RuntimeTemplateVars::content_has_templates(base_prompt) {
        template_vars.expand(base_prompt)
    } else {
        base_prompt.to_string()
    };

    if filtered_defs.is_empty() {
        return base;
    }

    let tool_protocol = y_prompt::tool_protocol_for(runtime_backend);

    match tool_calling_mode {
        ToolCallingMode::Native => {
            format!("{base}\n\n{tool_protocol}")
        }
        ToolCallingMode::PromptBased => {
            let tools_summary = crate::container::build_agent_tools_summary(filtered_defs);
            let syntax = y_tools::prompt_tool_call_syntax_for(tool_dialect);
            format!("{base}\n\n{tool_protocol}\n\n{syntax}\n\n{tools_summary}")
        }
    }
}

// ---------------------------------------------------------------------------
// ServiceAgentRunner -- bridges AgentPool.delegate() -> AgentService.execute()
// ---------------------------------------------------------------------------

/// `AgentRunner` implementation that uses `AgentService::execute()`.
///
/// Replaces `SingleTurnRunner` -- sub-agents now get the same execution loop
/// as the root chat agent (with capabilities controlled by `AgentRunConfig`).
pub struct ServiceAgentRunner {
    container: Arc<ServiceContainer>,
}

impl ServiceAgentRunner {
    /// Create a new `ServiceAgentRunner` backed by the given `ServiceContainer`.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait::async_trait]
impl AgentRunner for ServiceAgentRunner {
    async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
        let start = std::time::Instant::now();

        // Filter tool definitions from the agent allowlist.
        // When allowed_tools is non-empty, agents can make tool calls across
        // multiple iterations (e.g. skill-ingestion reading companion files).
        let filtered_defs =
            AgentService::filter_tool_definitions(&self.container, &config.allowed_tools).await;

        // Convert filtered definitions to OpenAI function-calling JSON.
        let tool_definitions: Vec<serde_json::Value> = filtered_defs
            .iter()
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                })
            })
            .collect();

        // Determine max_iterations: if tools are available, use the agent
        // definition's max_iterations; otherwise single-turn.
        let max_iterations = if tool_definitions.is_empty() {
            1
        } else {
            config.max_iterations
        };

        // Determine tool calling mode: use Native when tools are available.
        let tool_calling_mode = if tool_definitions.is_empty() {
            ToolCallingMode::default()
        } else {
            ToolCallingMode::Native
        };

        // Augment the system prompt with tool protocol and available-tools
        // summary when the agent has tools. In Native mode the XML tool
        // protocol is omitted (~800 tokens saved).
        let runtime_backend = self.container.runtime_manager.backend();
        let workspace = {
            let pc = self.container.prompt_context.read().await;
            pc.working_directory.clone()
        };
        let template_vars = RuntimeTemplateVars::from_runtime(workspace.as_deref());
        let system_prompt = build_subagent_system_prompt(
            &config.system_prompt,
            &filtered_defs,
            tool_calling_mode,
            y_core::provider::ToolDialect::default(),
            runtime_backend,
            &template_vars,
        );

        // Build messages: system_prompt + input as user message.
        let mut messages = Vec::with_capacity(2);
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: system_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        let user_content = match &config.input {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        };
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        // Pick up a pre-created trace_id from the diagnostics context
        // (set via DIAGNOSTICS_CTX task-local by DiagnosticsAgentDelegator).
        let external_trace_id = y_diagnostics::DIAGNOSTICS_CTX
            .try_with(|ctx| ctx.trace_id)
            .ok();

        // Pick up the parent turn's interaction context (set at the `Task`
        // interception in tool_dispatch). When present, the sub-agent runs
        // under a dedicated child session so its transcript is drill-in-able
        // from the info panel -- mirroring the plan / loop orchestrators.
        // Tool permissions follow the parent session's mode (incl. HITL), and
        // progress / cancel are wired to the parent turn. When absent (internal
        // / system delegations), the sub-agent stays detached as before.
        let interaction = crate::agent_service::delegation_ctx::DELEGATION_INTERACTION_CTX
            .try_with(Clone::clone)
            .ok();

        // Resolve the child session for this delegation. When an interaction
        // context is present (Task tool), create a SubAgent child session
        // under the parent so the delegation is visible in the info panel and
        // its transcript can be opened as a drill-in sub-chat. The child also
        // inherits the parent's permission / operation modes so HITL and
        // "allow all for session" behave identically.
        let (session_id, session_uuid, progress, cancel, child_session_handle) = match interaction {
            Some(ctx) => {
                let parent_id = ctx.session_id.clone();

                let child = self
                    .container
                    .session_manager
                    .create_session(y_core::session::CreateSessionOptions {
                        parent_id: Some(parent_id.clone()),
                        session_type: y_core::session::SessionType::SubAgent,
                        agent_id: Some(y_core::types::AgentId::from_string(&config.agent_name)),
                        title: Some(config.agent_name.clone()),
                    })
                    .await
                    .map_err(|e| DelegationError::DelegationFailed {
                        message: format!(
                            "failed to create sub-agent session for '{}': {e}",
                            config.agent_name
                        ),
                    })?;

                let child_uuid = Uuid::parse_str(child.id.as_str()).unwrap_or_else(|_| Uuid::nil());

                // Inherit the parent's permission / operation modes so the
                // sub-agent's tool gatekeeper resolves the same overrides the
                // parent session has (e.g. BypassPermissions, FullAccess).
                inherit_parent_modes(&self.container, &parent_id, &child.id).await;

                (
                    Some(child.id.clone()),
                    child_uuid,
                    ctx.progress,
                    ctx.cancel,
                    Some(ChildSessionHandle {
                        id: child.id,
                        user_query: user_content.clone(),
                    }),
                )
            }
            None => (None, Uuid::nil(), None, None, None),
        };

        let exec_config = AgentExecutionConfig {
            agent_name: config.agent_name.clone(),
            system_prompt,
            max_iterations,
            max_tool_calls: usize::MAX,
            tool_definitions,
            tool_calling_mode,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages,
            provider_id: None,
            preferred_models: config.preferred_models.clone(),
            provider_tags: config.provider_tags.clone(),
            fallback_provider_tags: config.fallback_provider_tags.clone(),
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: workspace,
            additional_read_dirs: vec![],
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            thinking: None,
            session_id,
            session_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: user_content,
            external_trace_id,
            trust_tier: config.trust_tier,
            agent_allowed_tools: config.allowed_tools.clone(),
            prune_tool_history: config.prune_tool_history,
            response_format: config.response_format.clone(),
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: self
                .container
                .dynamic_agent_service
                .execution_trace_metadata(&config.agent_name),
        };

        let result = AgentService::execute(&self.container, &exec_config, progress, cancel).await;

        // When running under a child session, persist the transcript so the
        // drill-in view is populated. The SubagentCompleted broadcast (which
        // triggers info-panel child-session reload) is already emitted by the
        // surrounding DiagnosticsAgentDelegator with the parent session's UUID.
        if let Some(handle) = child_session_handle.as_ref() {
            match &result {
                Ok(exec_result) if exec_result.content.is_empty() => {
                    persist_failed_subagent_turn(
                        &self.container,
                        handle,
                        &AgentExecutionError::LlmError {
                            message: format!(
                                "agent '{}' returned empty response",
                                config.agent_name
                            ),
                            provider_error: None,
                            partial_messages: Vec::new(),
                        },
                    )
                    .await;
                }
                Ok(exec_result) => {
                    crate::chat::ChatService::persist_subagent_turn(
                        &self.container,
                        &handle.id,
                        &handle.user_query,
                        exec_result,
                    )
                    .await;
                }
                Err(err) => {
                    persist_failed_subagent_turn(&self.container, handle, err).await;
                }
            }
        }

        let result = result.map_err(|e| DelegationError::DelegationFailed {
            message: format!(
                "AgentService execution failed for agent '{}': {e}",
                config.agent_name
            ),
        })?;

        if result.content.is_empty() {
            return Err(DelegationError::DelegationFailed {
                message: format!("agent '{}' returned empty response", config.agent_name),
            });
        }

        let tokens_used = result.input_tokens + result.output_tokens;

        Ok(AgentRunOutput {
            text: result.content,
            tokens_used,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            model_used: result.model,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(0),
        })
    }
}

// ---------------------------------------------------------------------------
// Child-session finalisation helpers (Task delegation only)
// ---------------------------------------------------------------------------

/// Bookkeeping for a child session created for a Task delegation.
struct ChildSessionHandle {
    id: y_core::types::SessionId,
    user_query: String,
}

/// Copy the parent session's permission and operation modes onto the child so
/// the sub-agent's tool gatekeeper resolves the same overrides (e.g.
/// `BypassPermissions`, `FullAccess`) without re-prompting the user.
async fn inherit_parent_modes(
    container: &ServiceContainer,
    parent_id: &y_core::types::SessionId,
    child_id: &y_core::types::SessionId,
) {
    if let Some(mode) =
        crate::agent_service::tool_dispatch::session_permission_mode(container, parent_id).await
    {
        crate::agent_service::tool_dispatch::set_session_permission_mode(container, child_id, mode)
            .await;
    }
    if let Some(mode) =
        crate::agent_service::tool_dispatch::session_operation_mode(container, parent_id).await
    {
        let mut modes = container
            .session_state
            .session_operation_modes
            .write()
            .await;
        modes.insert(child_id.clone(), mode);
    }
}

/// Persist a failed delegation's partial transcript to the child session so
/// the drill-in view shows what was accomplished before the error. Mirrors
/// `persist_partial_subagent_turn` in the plan orchestrator.
async fn persist_failed_subagent_turn(
    container: &ServiceContainer,
    handle: &ChildSessionHandle,
    error: &AgentExecutionError,
) {
    use y_core::types::Role;
    let user_msg = Message {
        message_id: y_core::types::generate_message_id(),
        role: Role::User,
        content: handle.user_query.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::json!({}),
    };
    if let Err(e) = container
        .session_manager
        .append_message(&handle.id, &user_msg)
        .await
    {
        tracing::warn!(error = %e, session_id = %handle.id, "failed to persist sub-agent prompt on error");
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
        let error_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: format!(
                "[Sub-agent execution failed before any output was produced: {error}]"
            ),
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
            .append_message(&handle.id, &error_msg)
            .await
        {
            tracing::warn!(error = %e, session_id = %handle.id, "failed to persist sub-agent error message");
        }
        return;
    }

    for msg in partial_messages {
        if let Err(e) = container
            .session_manager
            .append_message(&handle.id, msg)
            .await
        {
            tracing::warn!(error = %e, session_id = %handle.id, "failed to persist partial sub-agent message");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_service::delegation_ctx::{
        DelegationInteractionCtx, DELEGATION_INTERACTION_CTX,
    };
    use crate::config::ServiceConfig;
    use crate::container::ServiceContainer;
    use tempfile::TempDir;
    use y_core::permission_types::PermissionMode;
    use y_core::session::{CreateSessionOptions, SessionState, SessionType};

    async fn make_test_container() -> (ServiceContainer, TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("tempdir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    fn make_run_config(agent_name: &str, prompt: &str) -> AgentRunConfig {
        AgentRunConfig {
            agent_name: agent_name.to_string(),
            system_prompt: "You are a helper.".to_string(),
            input: serde_json::json!({ "task": prompt }),
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            fallback_provider_tags: vec![],
            temperature: None,
            max_tokens: None,
            timeout_secs: 30,
            allowed_tools: vec![],
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
            response_format: None,
        }
    }

    /// A Task delegation (interaction context present) must create a SubAgent
    /// child session under the parent so the InfoPanel can surface it.
    #[tokio::test]
    async fn task_delegation_creates_subagent_child_session() {
        let (container, _tmp) = make_test_container().await;
        let container = Arc::new(container);

        // Parent session (the active chat session).
        let parent = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Parent".into()),
            })
            .await
            .unwrap();

        let runner = ServiceAgentRunner::new(Arc::clone(&container));
        let config = make_run_config("general-purpose", "do something");

        // Run inside a Task-delegation interaction context.
        let ctx = DelegationInteractionCtx {
            session_id: parent.id.clone(),
            progress: None,
            cancel: None,
        };
        // The run fails (no provider configured), but the child session must
        // still be created and persisted.
        let _ = DELEGATION_INTERACTION_CTX
            .scope(ctx, runner.run(config))
            .await;

        // A SubAgent child session must exist under the parent.
        let children = container
            .session_manager
            .children(&parent.id)
            .await
            .unwrap();
        let sub = children
            .iter()
            .find(|c| c.session_type == SessionType::SubAgent)
            .expect("a SubAgent child session should be created");

        assert_eq!(sub.parent_id, Some(parent.id.clone()));
        assert_eq!(sub.state, SessionState::Active);
        assert_eq!(sub.title.as_deref(), Some("general-purpose"));

        // The child's transcript must contain the user prompt (persisted even
        // on failure so the drill-in view is not blank).
        let transcript = container
            .session_manager
            .read_transcript(&sub.id)
            .await
            .unwrap_or_default();
        assert!(
            transcript
                .iter()
                .any(|m| m.role == Role::User && m.content.contains("do something")),
            "child transcript should contain the delegation prompt"
        );
    }

    /// A Task delegation must inherit the parent session's permission mode so
    /// HITL / "allow all for session" applies to the sub-agent's tools.
    #[tokio::test]
    async fn task_delegation_inherits_parent_permission_mode() {
        let (container, _tmp) = make_test_container().await;
        let container = Arc::new(container);

        let parent = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Parent".into()),
            })
            .await
            .unwrap();

        // Set a bypass-permissions override on the parent session.
        crate::agent_service::tool_dispatch::set_session_permission_mode(
            &container,
            &parent.id,
            PermissionMode::BypassPermissions,
        )
        .await;
        let runner = ServiceAgentRunner::new(Arc::clone(&container));
        let config = make_run_config("general-purpose", "task");

        let ctx = DelegationInteractionCtx {
            session_id: parent.id.clone(),
            progress: None,
            cancel: None,
        };
        let _ = DELEGATION_INTERACTION_CTX
            .scope(ctx, runner.run(config))
            .await;

        let children = container
            .session_manager
            .children(&parent.id)
            .await
            .unwrap();
        let sub = children
            .iter()
            .find(|c| c.session_type == SessionType::SubAgent)
            .expect("child session should exist");

        let child_mode =
            crate::agent_service::tool_dispatch::session_permission_mode(&container, &sub.id).await;
        assert_eq!(
            child_mode,
            Some(PermissionMode::BypassPermissions),
            "child session should inherit the parent's permission mode"
        );
    }

    /// An internal delegation (no interaction context) must NOT create a child
    /// session -- the detached behaviour is preserved for system agents.
    #[tokio::test]
    async fn internal_delegation_does_not_create_child_session() {
        let (container, _tmp) = make_test_container().await;
        let container = Arc::new(container);

        let parent = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Parent".into()),
            })
            .await
            .unwrap();

        let runner = ServiceAgentRunner::new(Arc::clone(&container));
        let config = make_run_config("title-generator", "summarise");

        // No DELEGATION_INTERACTION_CTX scope -- simulates an internal call.
        let _ = runner.run(config).await;

        let children = container
            .session_manager
            .children(&parent.id)
            .await
            .unwrap();
        assert!(
            children.is_empty(),
            "internal delegation must not create a child session"
        );
    }
}
