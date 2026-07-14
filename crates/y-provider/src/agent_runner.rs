//! Agent runner: bridges `AgentRunConfig` → `ProviderPool::chat_completion()`.
//!
//! Standard reference: `docs/standards/AGENT_AUTONOMY.md`
//!
//! `SingleTurnRunner` is the default implementation of `AgentRunner`
//! (from `y-core`). It converts an agent's system prompt and structured
//! input into a single `ChatRequest`, routes it via `ProviderPool`,
//! and returns the result as `AgentRunOutput`.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::instrument;

use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::{
    ChatRequest, ChatResponse, ProviderError, ProviderPool, RouteRequest, ToolCallingMode,
};
use y_core::types::{generate_message_id, Message, Role};

/// Executes a single-turn agent by making one `ProviderPool::chat_completion()` call.
///
/// Suitable for system agents that need one LLM inference pass:
/// `title-generator`, `compaction-summarizer`, etc.
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
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
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
            request_mode: y_core::provider::RequestMode::TextChat,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            tool_dialect: y_core::provider::ToolDialect::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: config.response_format.clone(),
            image_generation_options: None,
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

    fn build_route_with_tags(config: &AgentRunConfig, required_tags: Vec<String>) -> RouteRequest {
        RouteRequest {
            preferred_model: config.preferred_models.first().cloned(),
            required_tags,
            ..Default::default()
        }
    }

    async fn chat_completion_with_tag_fallbacks(
        &self,
        request: &ChatRequest,
        config: &AgentRunConfig,
    ) -> Result<ChatResponse, ProviderError> {
        let primary_route = Self::build_route(config);
        let mut last_no_provider_error = match self
            .provider_pool
            .chat_completion(request, &primary_route)
            .await
        {
            Ok(response) => return Ok(response),
            Err(error @ ProviderError::NoProviderAvailable { .. }) => error,
            Err(error) => return Err(error),
        };

        for fallback_tags in &config.fallback_provider_tags {
            let fallback_route = Self::build_route_with_tags(config, fallback_tags.clone());
            match self
                .provider_pool
                .chat_completion(request, &fallback_route)
                .await
            {
                Ok(response) => return Ok(response),
                Err(error @ ProviderError::NoProviderAvailable { .. }) => {
                    last_no_provider_error = error;
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_no_provider_error)
    }
}

#[async_trait]
impl AgentRunner for SingleTurnRunner {
    #[instrument(skip(self, config), fields(agent = %config.agent_name))]
    async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
        let start = std::time::Instant::now();

        let request = Self::build_request(&config);

        let response = self
            .chat_completion_with_tag_fallbacks(&request, &config)
            .await
            .map_err(|e| DelegationError::DelegationFailed {
                message: format!("LLM call failed for agent '{}': {e}", config.agent_name),
            })?;

        let text = response.content.unwrap_or_default().trim().to_string();

        if text.is_empty() {
            return Err(DelegationError::DelegationFailed {
                message: format!("agent '{}' returned empty response", config.agent_name),
            });
        }

        let tokens_used =
            u64::from(response.usage.input_tokens) + u64::from(response.usage.output_tokens);

        Ok(AgentRunOutput {
            text,
            tokens_used,
            input_tokens: u64::from(response.usage.input_tokens),
            output_tokens: u64::from(response.usage.output_tokens),
            model_used: response.model,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use y_core::provider::{
        ChatResponse, ChatStreamResponse, FinishReason, ProviderError, ProviderPool,
        ProviderStatus, ResponseFormat,
    };
    use y_core::types::{ProviderId, TokenUsage};

    #[derive(Default)]
    struct RecordingPool {
        routes: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ProviderPool for RecordingPool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            let required_tags = route.required_tags.clone();
            self.routes
                .lock()
                .expect("routes mutex poisoned")
                .push(required_tags.clone());

            if required_tags == ["translation"] {
                return Err(ProviderError::NoProviderAvailable {
                    tags: required_tags,
                });
            }

            Ok(ChatResponse {
                id: "response-1".to_string(),
                model: "fallback-model".to_string(),
                content: Some("translated text".to_string()),
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 4,
                    output_tokens: 2,
                    ..Default::default()
                },
                finish_reason: FinishReason::Stop,
                raw_request: None,
                raw_response: None,
                provider_id: None,
                generated_images: vec![],
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            panic!("RecordingPool::chat_completion_stream should not be called")
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            vec![]
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

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
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
            response_format: None,
            fallback_provider_tags: vec![],
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
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
            response_format: None,
            fallback_provider_tags: vec![],
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
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
            response_format: None,
            fallback_provider_tags: vec![],
        };

        let route = SingleTurnRunner::build_route(&config);
        assert_eq!(route.preferred_model, Some("claude-3-haiku".to_string()));
    }

    #[tokio::test]
    async fn test_agent_runner_falls_back_to_general_tags_when_translation_provider_missing() {
        let pool = Arc::new(RecordingPool::default());
        let runner = SingleTurnRunner::new(pool.clone());
        let config = AgentRunConfig {
            agent_name: "translator".to_string(),
            system_prompt: "Translate".to_string(),
            input: serde_json::json!({"text": "hello"}),
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec!["translation".to_string()],
            fallback_provider_tags: vec![vec!["general".to_string()]],
            temperature: Some(0.3),
            max_tokens: None,
            timeout_secs: 30,
            allowed_tools: vec![],
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
            response_format: None::<ResponseFormat>,
        };

        let output = runner.run(config).await.unwrap();

        assert_eq!(output.text, "translated text");
        let routes = pool.routes.lock().expect("routes mutex poisoned");
        assert_eq!(
            routes.as_slice(),
            &[vec!["translation".to_string()], vec!["general".to_string()]]
        );
    }
}
