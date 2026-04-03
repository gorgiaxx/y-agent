//! Mock `LlmProvider` for integration and E2E tests.
//!
//! Supports configurable responses, latency simulation, and token counting.

use async_trait::async_trait;
use futures::stream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, LlmProvider,
    ProviderError, ProviderMetadata, ProviderType, ToolCallingMode,
};
use y_core::types::{ProviderId, TokenUsage};

/// Behaviour preset for mock responses.
#[derive(Debug, Clone)]
pub enum MockBehaviour {
    /// Always return a fixed text response.
    FixedResponse(String),
    /// Echo the last user message.
    Echo,
    /// Return an error on every call.
    AlwaysFail(String),
    /// Cycle through responses in order.
    Cycle(Vec<String>),
}

/// A configurable mock LLM provider.
#[derive(Debug, Clone)]
pub struct MockProvider {
    metadata: ProviderMetadata,
    behaviour: MockBehaviour,
    call_count: Arc<AtomicUsize>,
}

impl MockProvider {
    /// Create with a given behaviour preset.
    #[must_use]
    pub fn new(behaviour: MockBehaviour) -> Self {
        Self {
            metadata: ProviderMetadata {
                id: ProviderId::from_string("mock-provider"),
                provider_type: ProviderType::Custom,
                model: "mock-model".into(),
                tags: vec!["test".into()],
                max_concurrency: 10,
                context_window: 4096,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode: ToolCallingMode::default(),
            },
            behaviour,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a mock that always returns a fixed response.
    #[must_use]
    pub fn fixed(response: impl Into<String>) -> Self {
        Self::new(MockBehaviour::FixedResponse(response.into()))
    }

    /// Create a mock that echoes user input.
    #[must_use]
    pub fn echo() -> Self {
        Self::new(MockBehaviour::Echo)
    }

    /// Create a mock that always fails.
    #[must_use]
    pub fn failing(msg: impl Into<String>) -> Self {
        Self::new(MockBehaviour::AlwaysFail(msg.into()))
    }

    /// Number of calls made to this provider.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }

    fn next_response(&self, request: &ChatRequest, count: usize) -> Result<String, ProviderError> {
        match &self.behaviour {
            MockBehaviour::FixedResponse(text) => Ok(text.clone()),
            MockBehaviour::Echo => {
                let last = request
                    .messages
                    .last()
                    .map(|m| m.content.clone())
                    .unwrap_or_default();
                Ok(format!("echo: {last}"))
            }
            MockBehaviour::AlwaysFail(msg) => Err(ProviderError::Other {
                message: msg.clone(),
            }),
            MockBehaviour::Cycle(responses) => {
                let idx = count % responses.len();
                Ok(responses[idx].clone())
            }
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let count = self.call_count.fetch_add(1, Ordering::Relaxed);
        let text = self.next_response(request, count)?;
        let output_tokens = u32::try_from(text.len() / 4).unwrap_or(u32::MAX);
        Ok(ChatResponse {
            id: uuid::Uuid::new_v4().to_string(),
            model: "mock-model".into(),
            content: Some(text),
            reasoning_content: None,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: request
                    .messages
                    .iter()
                    .map(|m| u32::try_from(m.content.len() / 4).unwrap_or(u32::MAX))
                    .sum(),
                output_tokens,
                cache_read_tokens: None,
                cache_write_tokens: None,
                ..Default::default()
            },
            finish_reason: FinishReason::Stop,
            raw_request: None,
            raw_response: None,
            provider_id: None,
        })
    }

    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        let count = self.call_count.fetch_add(1, Ordering::Relaxed);
        let text = self.next_response(request, count)?;
        let chunk = ChatStreamChunk {
            delta_content: Some(text),
            delta_reasoning_content: None,
            delta_tool_calls: vec![],
            usage: None,
            finish_reason: Some(FinishReason::Stop),
        };
        Ok(ChatStreamResponse {
            stream: Box::pin(stream::iter(vec![Ok(chunk)])),
            raw_request: None,
            provider_id: None,
            model: String::new(),
            context_window: 0,
        })
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::provider::ToolCallingMode;
    use y_core::types::{Message, Role};

    fn user_msg(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_req(msg: &str) -> ChatRequest {
        ChatRequest {
            messages: vec![user_msg(msg)],
            model: Some("mock".into()),
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: vec![],
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            extra: serde_json::Value::Null,
            thinking: None,
        }
    }

    #[tokio::test]
    async fn test_fixed_response() {
        let provider = MockProvider::fixed("hello world");
        let resp = provider.chat_completion(&make_req("hi")).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("hello world"));
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn test_echo_behaviour() {
        let provider = MockProvider::echo();
        let resp = provider.chat_completion(&make_req("ping")).await.unwrap();
        assert_eq!(resp.content.as_deref(), Some("echo: ping"));
    }

    #[tokio::test]
    async fn test_failing_provider() {
        let provider = MockProvider::failing("simulated outage");
        let err = provider.chat_completion(&make_req("hi")).await.unwrap_err();
        assert!(err.to_string().contains("simulated outage"));
    }

    #[tokio::test]
    async fn test_cycle_responses() {
        let provider =
            MockProvider::new(MockBehaviour::Cycle(vec!["first".into(), "second".into()]));
        let req = make_req("hi");
        let r1 = provider.chat_completion(&req).await.unwrap();
        assert_eq!(r1.content.as_deref(), Some("first"));
        let r2 = provider.chat_completion(&req).await.unwrap();
        assert_eq!(r2.content.as_deref(), Some("second"));
        let r3 = provider.chat_completion(&req).await.unwrap();
        assert_eq!(r3.content.as_deref(), Some("first"));
    }
}
