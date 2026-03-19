//! Agent runner: bridges `AgentRunConfig` → `ProviderPool::chat_completion()`.
//!
//! Design reference: `AGENT_AUTONOMY.md` §2.3
//!
//! `SingleTurnRunner` is the default implementation of `AgentRunner`
//! (from `y-core`). It converts an agent's system prompt and structured
//! input into a single `ChatRequest`, routes it via `ProviderPool`,
//! and returns the result as `AgentRunOutput`.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::instrument;

use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::types::{generate_message_id, Message, Role};

/// Executes a single-turn agent by making one `ProviderPool::chat_completion()` call.
///
/// Suitable for system agents that need one LLM inference pass:
/// `title-generator`, `compaction-summarizer`, `context-summarizer`, etc.
///
/// The runner builds a `ChatRequest` from the agent's config:
/// - System message from `config.system_prompt`
/// - User message from `config.input` (serialized to string if not already)
/// - Model routing via `config.preferred_models`
pub struct SingleTurnRunner {
    provider_pool: Arc<dyn ProviderPool>,
}

impl SingleTurnRunner {
    /// Create a new `SingleTurnRunner` backed by the given `ProviderPool`.
    pub fn new(provider_pool: Arc<dyn ProviderPool>) -> Self {
        Self { provider_pool }
    }

    /// Build a `ChatRequest` from an `AgentRunConfig`.
    fn build_request(config: &AgentRunConfig) -> ChatRequest {
        let mut messages = Vec::with_capacity(2);

        // System prompt from the agent's TOML definition.
        messages.push(Message {
            message_id: generate_message_id(),
            role: Role::System,
            content: config.system_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        // User message: serialize input JSON to string for the LLM.
        let user_content = match &config.input {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other)
                .unwrap_or_else(|_| other.to_string()),
        };

        messages.push(Message {
            message_id: generate_message_id(),
            role: Role::User,
            content: user_content,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        ChatRequest {
            messages,
            // Model preference is expressed only through `RouteRequest.preferred_model`
            // for routing. Setting it here would override the selected provider's own
            // model when the preferred model isn't available, causing 404 errors on
            // providers that don't support the requested model name.
            model: None,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
        }
    }

    /// Build a `RouteRequest` from an `AgentRunConfig`.
    fn build_route(config: &AgentRunConfig) -> RouteRequest {
        RouteRequest {
            preferred_model: config.preferred_models.first().cloned(),
            required_tags: config.provider_tags.clone(),
            ..Default::default()
        }
    }
}

#[async_trait]
impl AgentRunner for SingleTurnRunner {
    #[instrument(skip(self, config), fields(agent = %config.agent_name))]
    async fn run(
        &self,
        config: AgentRunConfig,
    ) -> Result<AgentRunOutput, DelegationError> {
        let start = std::time::Instant::now();

        let request = Self::build_request(&config);
        let route = Self::build_route(&config);

        let response = self
            .provider_pool
            .chat_completion(&request, &route)
            .await
            .map_err(|e| DelegationError::DelegationFailed {
                message: format!("LLM call failed for agent '{}': {e}", config.agent_name),
            })?;

        let text = response
            .content
            .unwrap_or_default()
            .trim()
            .to_string();

        if text.is_empty() {
            return Err(DelegationError::DelegationFailed {
                message: format!("agent '{}' returned empty response", config.agent_name),
            });
        }

        let tokens_used =
            response.usage.input_tokens as u32 + response.usage.output_tokens as u32;

        Ok(AgentRunOutput {
            text,
            tokens_used,
            input_tokens: u64::from(response.usage.input_tokens),
            output_tokens: u64::from(response.usage.output_tokens),
            model_used: response.model,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build request correctly assembles system + user messages.
    #[test]
    fn test_build_request_structure() {
        let config = AgentRunConfig {
            agent_name: "test-agent".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            input: serde_json::json!({"data": "hello world"}),
            preferred_models: vec!["gpt-4o-mini".to_string()],
            fallback_models: vec![],
            provider_tags: vec!["general".to_string()],
            temperature: Some(0.5),
            max_tokens: Some(100),
            timeout_secs: 10,
            allowed_tools: vec![],
            denied_tools: vec![],
            max_iterations: 1,
        };

        let request = SingleTurnRunner::build_request(&config);

        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, Role::System);
        assert_eq!(request.messages[0].content, "You are a test agent.");
        assert_eq!(request.messages[1].role, Role::User);
        assert!(request.messages[1].content.contains("hello world"));
        // Model preference is only used for routing, not set in ChatRequest.
        assert_eq!(request.model, None);
        assert_eq!(request.temperature, Some(0.5));
        assert_eq!(request.max_tokens, Some(100));
        assert!(request.tools.is_empty());
    }

    /// String input is passed through directly (not double-serialized).
    #[test]
    fn test_build_request_string_input() {
        let config = AgentRunConfig {
            agent_name: "test-agent".to_string(),
            system_prompt: "Prompt".to_string(),
            input: serde_json::Value::String("plain text input".to_string()),
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            max_tokens: None,
            timeout_secs: 10,
            allowed_tools: vec![],
            denied_tools: vec![],
            max_iterations: 1,
        };

        let request = SingleTurnRunner::build_request(&config);
        assert_eq!(request.messages[1].content, "plain text input");
        // Model is always None in ChatRequest (model preference is routing-only).
        assert_eq!(request.model, None);
    }

    /// Route request uses preferred model.
    #[test]
    fn test_build_route() {
        let config = AgentRunConfig {
            agent_name: "test-agent".to_string(),
            system_prompt: "Prompt".to_string(),
            input: serde_json::json!(null),
            preferred_models: vec!["claude-3-haiku".to_string()],
            fallback_models: vec![],
            provider_tags: vec!["title".to_string()],
            temperature: None,
            max_tokens: None,
            timeout_secs: 10,
            allowed_tools: vec![],
            denied_tools: vec![],
            max_iterations: 1,
        };

        let route = SingleTurnRunner::build_route(&config);
        assert_eq!(route.preferred_model, Some("claude-3-haiku".to_string()));
    }
}
