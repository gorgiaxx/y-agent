//! `HookLlmRunner` implementation for y-provider.
//!
//! Wraps a `ProviderPool` to implement the `HookLlmRunner` trait from y-core,
//! enabling prompt hook handlers in y-hooks to evaluate decisions via LLM calls
//! without depending directly on y-provider.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tracing::debug;

use y_core::hook::HookLlmRunner;
use y_core::provider::{ChatRequest, ProviderPool, RoutePriority, RouteRequest, ToolCallingMode};
use y_core::types::{Message, Role};

/// An implementation of `HookLlmRunner` backed by a `ProviderPool`.
///
/// Sends single-turn completion requests for prompt hook evaluation.
/// Uses the fastest available provider by default (tag "fast"), with an
/// optional model override from the hook config.
pub struct ProviderPoolHookLlmRunner {
    pool: Arc<dyn ProviderPool>,
}

impl ProviderPoolHookLlmRunner {
    /// Create a new runner backed by the given provider pool.
    pub fn new(pool: Arc<dyn ProviderPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HookLlmRunner for ProviderPoolHookLlmRunner {
    async fn evaluate(
        &self,
        system_prompt: &str,
        user_message: &str,
        model: Option<&str>,
        timeout: Duration,
    ) -> Result<String, String> {
        let now = y_core::types::now();

        let request = ChatRequest {
            messages: vec![
                Message {
                    message_id: y_core::types::generate_message_id(),
                    role: Role::System,
                    content: system_prompt.to_string(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: now,
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: y_core::types::generate_message_id(),
                    role: Role::User,
                    content: user_message.to_string(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: now,
                    metadata: serde_json::Value::Null,
                },
            ],
            model: model.map(std::string::ToString::to_string),
            max_tokens: Some(256),  // Hook responses are short JSON.
            temperature: Some(0.0), // Deterministic for safety decisions.
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
        };

        let route = RouteRequest {
            required_tags: vec![],
            preferred_model: model.map(std::string::ToString::to_string),
            preferred_provider_id: None,
            priority: RoutePriority::Normal,
        };

        debug!(
            model = model.unwrap_or("(default)"),
            timeout_ms = timeout.as_millis() as u64,
            "executing prompt hook LLM call"
        );

        let response = tokio::time::timeout(timeout, self.pool.chat_completion(&request, &route))
            .await
            .map_err(|_| {
                format!(
                    "prompt hook LLM call timed out after {}ms",
                    timeout.as_millis()
                )
            })?
            .map_err(|e| format!("prompt hook LLM call failed: {e}"))?;

        response
            .content
            .ok_or_else(|| "LLM response had no content".to_string())
    }
}

impl std::fmt::Debug for ProviderPoolHookLlmRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderPoolHookLlmRunner")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::provider::{
        ChatResponse, ChatStreamResponse, FinishReason, ProviderError, ProviderStatus,
    };
    use y_core::types::{ProviderId, TokenUsage};

    struct MockPool {
        response: ChatResponse,
    }

    #[async_trait]
    impl ProviderPool for MockPool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(self.response.clone())
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            unimplemented!("not needed for hook tests")
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

    fn allow_response() -> ChatResponse {
        ChatResponse {
            id: "test-resp".into(),
            model: "test-model".into(),
            content: Some(r#"{"ok": true, "reason": "safe"}"#.into()),
            reasoning_content: None,
            tool_calls: vec![],
            usage: TokenUsage::default(),
            finish_reason: FinishReason::Stop,
            raw_request: None,
            raw_response: None,
            provider_id: None,
        }
    }

    fn block_response() -> ChatResponse {
        ChatResponse {
            id: "test-resp".into(),
            model: "test-model".into(),
            content: Some(r#"{"ok": false, "reason": "dangerous"}"#.into()),
            reasoning_content: None,
            tool_calls: vec![],
            usage: TokenUsage::default(),
            finish_reason: FinishReason::Stop,
            raw_request: None,
            raw_response: None,
            provider_id: None,
        }
    }

    #[tokio::test]
    async fn test_evaluate_allow() {
        let pool = Arc::new(MockPool {
            response: allow_response(),
        });
        let runner = ProviderPoolHookLlmRunner::new(pool);

        let result = runner
            .evaluate("sys prompt", "user message", None, Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().contains("\"ok\": true"));
    }

    #[tokio::test]
    async fn test_evaluate_block() {
        let pool = Arc::new(MockPool {
            response: block_response(),
        });
        let runner = ProviderPoolHookLlmRunner::new(pool);

        let result = runner
            .evaluate("sys prompt", "user message", None, Duration::from_secs(5))
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().contains("\"ok\": false"));
    }

    #[tokio::test]
    async fn test_evaluate_no_content() {
        let pool = Arc::new(MockPool {
            response: ChatResponse {
                id: "test".into(),
                model: "test".into(),
                content: None,
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage::default(),
                finish_reason: FinishReason::Stop,
                raw_request: None,
                raw_response: None,
                provider_id: None,
            },
        });
        let runner = ProviderPoolHookLlmRunner::new(pool);

        let result = runner
            .evaluate("sys", "user", None, Duration::from_secs(5))
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no content"));
    }

    #[tokio::test]
    async fn test_evaluate_with_model_override() {
        let pool = Arc::new(MockPool {
            response: allow_response(),
        });
        let runner = ProviderPoolHookLlmRunner::new(pool);

        let result = runner
            .evaluate(
                "sys prompt",
                "user message",
                Some("claude-3-haiku"),
                Duration::from_secs(5),
            )
            .await;

        assert!(result.is_ok());
    }
}
