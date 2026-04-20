//! E2E integration test: Multi-agent delegation patterns.

use y_core::provider::{LlmProvider, ToolCallingMode};
use y_core::session::{
    CreateSessionOptions, SessionState, SessionStore, SessionType, TranscriptStore,
};
use y_core::types::{AgentId, Role};
use y_test_utils::{make_user_message, MockProvider, MockSessionStore, MockTranscriptStore};

#[tokio::test]
async fn e2e_parent_delegates_to_child() {
    let session_store = MockSessionStore::new();
    let transcript_store = MockTranscriptStore::new();

    // Parent agent's session
    let parent_session = session_store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: Some(AgentId::from_string("parent-agent")),
            title: Some("Parent task".into()),
        })
        .await
        .unwrap();

    // Parent receives a task
    transcript_store
        .append(
            &parent_session.id,
            &make_user_message("Research and summarize the topic"),
        )
        .await
        .unwrap();

    // Parent creates a child session for delegation
    let child_session = session_store
        .create(CreateSessionOptions {
            parent_id: Some(parent_session.id.clone()),
            session_type: SessionType::Child,
            agent_id: Some(AgentId::from_string("researcher-agent")),
            title: Some("Research subtask".into()),
        })
        .await
        .unwrap();

    assert_eq!(child_session.session_type, SessionType::Child);

    // Child agent processes the task using its own provider
    let child_provider =
        MockProvider::fixed("Research findings: AI agents can delegate tasks efficiently.");

    transcript_store
        .append(
            &child_session.id,
            &make_user_message("Research the given topic"),
        )
        .await
        .unwrap();

    let child_messages = transcript_store.read_all(&child_session.id).await.unwrap();
    let child_request = y_core::provider::ChatRequest {
        messages: child_messages,
        model: Some("mock".into()),
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

    let child_response = child_provider
        .chat_completion(&child_request)
        .await
        .unwrap();

    // Child writes result to its transcript
    transcript_store
        .append(
            &child_session.id,
            &y_test_utils::make_assistant_message(child_response.content.as_deref().unwrap_or("")),
        )
        .await
        .unwrap();

    // Parent collects child result
    let child_transcript = transcript_store.read_all(&child_session.id).await.unwrap();
    let last_child_msg = child_transcript.last().unwrap();
    assert_eq!(last_child_msg.role, Role::Assistant);
    assert!(last_child_msg.content.contains("Research findings"));

    // Archive child session after collection
    session_store
        .set_state(&child_session.id, SessionState::Archived)
        .await
        .unwrap();

    let child = session_store.get(&child_session.id).await.unwrap();
    assert_eq!(child.state, SessionState::Archived);
}

#[tokio::test]
async fn e2e_sequential_multi_agent_pipeline() {
    let session_store = MockSessionStore::new();
    let transcript_store = MockTranscriptStore::new();

    // Parent session
    let parent = session_store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: Some(AgentId::from_string("orchestrator")),
            title: Some("Pipeline".into()),
        })
        .await
        .unwrap();

    // Stage 1: Research agent
    let stage1 = session_store
        .create(CreateSessionOptions {
            parent_id: Some(parent.id.clone()),
            session_type: SessionType::Child,
            agent_id: Some(AgentId::from_string("researcher")),
            title: Some("Stage 1: Research".into()),
        })
        .await
        .unwrap();

    let researcher = MockProvider::fixed("Found 3 relevant papers.");
    transcript_store
        .append(&stage1.id, &make_user_message("Find relevant papers"))
        .await
        .unwrap();
    let req = y_core::provider::ChatRequest {
        messages: transcript_store.read_all(&stage1.id).await.unwrap(),
        model: Some("mock".into()),
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
    let resp1 = researcher.chat_completion(&req).await.unwrap();

    // Stage 2: Summarizer receives output from stage 1
    let stage2 = session_store
        .create(CreateSessionOptions {
            parent_id: Some(parent.id.clone()),
            session_type: SessionType::Child,
            agent_id: Some(AgentId::from_string("summarizer")),
            title: Some("Stage 2: Summarize".into()),
        })
        .await
        .unwrap();

    let summarizer = MockProvider::fixed("Summary: 3 papers found, key insights extracted.");
    let input_for_summarizer = format!(
        "Summarize the following research: {}",
        resp1.content.as_deref().unwrap_or("")
    );
    transcript_store
        .append(&stage2.id, &make_user_message(&input_for_summarizer))
        .await
        .unwrap();
    let req2 = y_core::provider::ChatRequest {
        messages: transcript_store.read_all(&stage2.id).await.unwrap(),
        model: Some("mock".into()),
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
    let resp2 = summarizer.chat_completion(&req2).await.unwrap();
    assert!(resp2.content.as_deref().unwrap_or("").contains("Summary"));

    // Verify parent has 2 children
    let children = session_store.children(&parent.id).await.unwrap();
    assert_eq!(children.len(), 2);
}
