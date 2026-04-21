//! E2E integration test: Chat flow through the full stack.
//!
//! Tests:
//! - Single-turn chat (mock provider → response)
//! - Multi-turn conversation (message history preserved)
//! - Provider failure propagation

use y_core::provider::{ChatRequest, LlmProvider, RequestMode, ToolCallingMode};
use y_core::session::{CreateSessionOptions, SessionStore, SessionType, TranscriptStore};
use y_core::types::{Message, Role};
use y_test_utils::{MockProvider, MockSessionStore, MockTranscriptStore};

fn make_msg(role: Role, content: &str) -> Message {
    Message {
        message_id: y_core::types::generate_message_id(),
        role,
        content: content.into(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn e2e_single_turn_chat() {
    // 1. Create session
    let session_store = MockSessionStore::new();
    let transcript_store = MockTranscriptStore::new();
    let provider = MockProvider::fixed("Hello! How can I help you?");

    let session = session_store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("E2E chat test".into()),
        })
        .await
        .unwrap();

    // 2. Append user message to transcript
    let user_msg = make_msg(Role::User, "Hi there!");
    transcript_store
        .append(&session.id, &user_msg)
        .await
        .unwrap();

    // 3. Build chat request from transcript
    let messages = transcript_store.read_all(&session.id).await.unwrap();
    let request = ChatRequest {
        messages,
        model: Some("mock".into()),
        request_mode: RequestMode::TextChat,
        max_tokens: Some(1024),
        temperature: None,
        top_p: None,
        stop: vec![],
        tools: vec![],
        tool_calling_mode: ToolCallingMode::default(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
        image_generation_options: None,
    };

    // 4. Get provider response
    let response = provider.chat_completion(&request).await.unwrap();
    assert_eq!(
        response.content.as_deref(),
        Some("Hello! How can I help you?")
    );

    // 5. Append assistant response to transcript
    let assistant_msg = make_msg(Role::Assistant, response.content.as_deref().unwrap_or(""));
    transcript_store
        .append(&session.id, &assistant_msg)
        .await
        .unwrap();

    // 6. Verify transcript has 2 messages
    let count = transcript_store.message_count(&session.id).await.unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn e2e_multi_turn_conversation() {
    let session_store = MockSessionStore::new();
    let transcript_store = MockTranscriptStore::new();
    let provider = MockProvider::echo();

    let session = session_store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("multi-turn".into()),
        })
        .await
        .unwrap();

    // Turn 1
    let msg1 = make_msg(Role::User, "What is Rust?");
    transcript_store.append(&session.id, &msg1).await.unwrap();

    let messages = transcript_store.read_all(&session.id).await.unwrap();
    let req = ChatRequest {
        messages,
        model: Some("mock".into()),
        request_mode: RequestMode::TextChat,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        tools: vec![],
        tool_calling_mode: ToolCallingMode::default(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
        image_generation_options: None,
    };
    let resp = provider.chat_completion(&req).await.unwrap();
    let reply1 = make_msg(Role::Assistant, resp.content.as_deref().unwrap_or(""));
    transcript_store.append(&session.id, &reply1).await.unwrap();

    // Turn 2 — verify history grows
    let msg2 = make_msg(Role::User, "Tell me more");
    transcript_store.append(&session.id, &msg2).await.unwrap();

    let messages = transcript_store.read_all(&session.id).await.unwrap();
    assert_eq!(messages.len(), 3); // user, assistant, user

    let req2 = ChatRequest {
        messages,
        model: Some("mock".into()),
        request_mode: RequestMode::TextChat,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        tools: vec![],
        tool_calling_mode: ToolCallingMode::default(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
        image_generation_options: None,
    };
    let resp2 = provider.chat_completion(&req2).await.unwrap();
    assert_eq!(resp2.content.as_deref(), Some("echo: Tell me more"));

    let all = transcript_store.read_all(&session.id).await.unwrap();
    // After appending resp2, should be 3 (haven't appended yet)
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn e2e_provider_failure_propagation() {
    let provider = MockProvider::failing("service unavailable");

    let req = ChatRequest {
        messages: vec![make_msg(Role::User, "hello")],
        model: Some("mock".into()),
        request_mode: RequestMode::TextChat,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: vec![],
        tools: vec![],
        tool_calling_mode: ToolCallingMode::default(),
        extra: serde_json::Value::Null,
        thinking: None,
        response_format: None,
        image_generation_options: None,
    };

    let err = provider.chat_completion(&req).await.unwrap_err();
    assert!(err.to_string().contains("service unavailable"));
}
