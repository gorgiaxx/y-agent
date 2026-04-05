//! Sub-agent runner and prompt construction.
//!
//! Contains `ServiceAgentRunner` (bridges `AgentPool.delegate()` to
//! `AgentService.execute()`) and `build_subagent_system_prompt`.

use std::sync::Arc;

use uuid::Uuid;

use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::ToolCallingMode;
use y_core::types::{Message, Role};

use crate::container::ServiceContainer;

use super::{AgentExecutionConfig, AgentService};

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
) -> String {
    if filtered_defs.is_empty() {
        return base_prompt.to_string();
    }

    let tool_protocol = y_prompt::PROMPT_TOOL_PROTOCOL;

    match tool_calling_mode {
        ToolCallingMode::Native => {
            // Native mode: tools are sent via the API `tools` field.
            // Still provide universal tool protocol rules, but no XML syntax.
            format!("{base_prompt}\n\n{tool_protocol}")
        }
        ToolCallingMode::PromptBased => {
            let tools_summary = crate::container::build_agent_tools_summary(filtered_defs);
            let syntax = y_tools::parser::PROMPT_TOOL_CALL_SYNTAX;
            format!("{base_prompt}\n\n{tool_protocol}\n\n{syntax}\n\n{tools_summary}")
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

        // Filter tool definitions from allowed_tools/denied_tools.
        // When allowed_tools is non-empty, agents can make tool calls across
        // multiple iterations (e.g. skill-ingestion reading companion files).
        let filtered_defs = AgentService::filter_tool_definitions(
            &self.container,
            &config.allowed_tools,
            &config.denied_tools,
        )
        .await;

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
        let system_prompt =
            build_subagent_system_prompt(&config.system_prompt, &filtered_defs, tool_calling_mode);

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

        let exec_config = AgentExecutionConfig {
            agent_name: config.agent_name.clone(),
            system_prompt,
            max_iterations,
            tool_definitions,
            tool_calling_mode,
            messages,
            provider_id: None,
            preferred_models: config.preferred_models.clone(),
            provider_tags: config.provider_tags.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            thinking: None,
            session_id: None,
            session_uuid: Uuid::nil(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: user_content,
            external_trace_id,
            trust_tier: config.trust_tier,
            agent_allowed_tools: config.allowed_tools.clone(),
            prune_tool_history: config.prune_tool_history,
        };

        let result = AgentService::execute(&self.container, &exec_config, None, None)
            .await
            .map_err(|e| DelegationError::DelegationFailed {
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

        let tokens_used = u32::try_from(result.input_tokens).unwrap_or(0)
            + u32::try_from(result.output_tokens).unwrap_or(0);

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
