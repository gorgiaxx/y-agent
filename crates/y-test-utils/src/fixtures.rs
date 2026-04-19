//! Factory functions for creating test data.

use y_core::provider::{
    ChatRequest, ChatResponse, FinishReason, ProviderMetadata, ProviderType, ToolCallingMode,
};
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::{Message, ProviderId, Role, SessionId, TokenUsage, WorkflowId};

/// Create a user message with the given content.
#[must_use]
pub fn make_user_message(content: &str) -> Message {
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

/// Create an assistant message with the given content.
#[must_use]
pub fn make_assistant_message(content: &str) -> Message {
    Message {
        message_id: y_core::types::generate_message_id(),
        role: Role::Assistant,
        content: content.into(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    }
}

/// Create a simple chat request with one user message.
#[must_use]
pub fn make_chat_request(user_input: &str) -> ChatRequest {
    ChatRequest {
        messages: vec![make_user_message(user_input)],
        model: Some("test-model".into()),
        max_tokens: Some(1024),
        temperature: Some(0.7),
        top_p: None,
        stop: vec![],
        tools: vec![],
        tool_calling_mode: ToolCallingMode::default(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
    }
}

/// Create a successful chat response.
#[must_use]
pub fn make_chat_response(content: &str) -> ChatResponse {
    ChatResponse {
        id: uuid::Uuid::new_v4().to_string(),
        model: "test-model".into(),
        content: Some(content.into()),
        reasoning_content: None,
        tool_calls: vec![],
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: None,
            cache_write_tokens: None,
            ..Default::default()
        },
        finish_reason: FinishReason::Stop,
        raw_request: None,
        raw_response: None,
        provider_id: None,
        generated_images: vec![],
    }
}

/// Create a provider metadata entry for testing.
#[must_use]
pub fn make_provider_metadata(name: &str) -> ProviderMetadata {
    ProviderMetadata {
        id: ProviderId::from_string(format!("provider-{name}")),
        provider_type: ProviderType::Custom,
        model: "test-model".into(),
        tags: vec!["test".into()],
        max_concurrency: 10,
        context_window: 4096,
        cost_per_1k_input: 0.001,
        cost_per_1k_output: 0.002,
        tool_calling_mode: ToolCallingMode::default(),
    }
}

/// Create session options for a main session.
#[must_use]
pub fn make_session_options(title: &str) -> CreateSessionOptions {
    CreateSessionOptions {
        parent_id: None,
        session_type: SessionType::Main,
        agent_id: None,
        title: Some(title.into()),
    }
}

/// Create a new random `SessionId`.
#[must_use]
pub fn make_session_id() -> SessionId {
    SessionId::new()
}

/// Create a new random `WorkflowId`.
#[must_use]
pub fn make_workflow_id() -> WorkflowId {
    WorkflowId::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_user_message() {
        let msg = make_user_message("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hello");
    }

    #[test]
    fn test_make_chat_request_response() {
        let req = make_chat_request("test input");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.model, Some("test-model".into()));

        let resp = make_chat_response("output");
        assert_eq!(resp.content.as_deref(), Some("output"));
        assert_eq!(resp.usage.total(), 30);
    }

    #[test]
    fn test_make_provider_metadata() {
        let meta = make_provider_metadata("openai");
        assert!(meta.id.as_str().contains("openai"));
    }
}
