//! Shared helpers for chat and print commands.

use y_core::session::SessionNode;
use y_core::types::{Message, SessionId};
use y_service::chat::{TurnError, TurnResult};

use crate::orchestrator::{self, TurnInput};
use crate::wire::AppServices;

/// Execute a single chat turn and append new messages to `history`.
///
/// Shared by `chat` (REPL) and `print` (single-shot). Persists the user
/// message, runs the turn via the orchestrator, and appends assistant/tool
/// messages to the local history. Returns the turn result.
pub(crate) async fn run_single_turn(
    services: &AppServices,
    session: &SessionNode,
    history: &mut Vec<Message>,
    turn_number: &mut u32,
    input: &str,
    working_directory: Option<String>,
    session_uuid: uuid::Uuid,
) -> Result<TurnResult, TurnError> {
    // Build user message.
    let user_msg = Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: input.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    };

    // Persist user message.
    let _ = services
        .session_manager
        .append_message(&session.id, &user_msg)
        .await;

    history.push(user_msg);

    let turn_input = TurnInput {
        user_input: input,
        session_id: session.id.clone(),
        session_uuid,
        history,
        turn_number: *turn_number,
        provider_id: None,
        request_mode: y_core::provider::RequestMode::TextChat,
        working_directory,
        knowledge_collections: vec![],
        thinking: None,
        plan_mode: None,
        operation_mode: y_service::chat_types::OperationMode::Default,
        agent_name: "chat-turn".to_string(),
        toolcall_enabled: true,
        preferred_models: vec![],
        provider_tags: vec![],
        temperature: None,
        max_completion_tokens: None,
        max_iterations: None,
        max_tool_calls: None,
        trust_tier: None,
        agent_allowed_tools: vec![],
        prune_tool_history: false,
        mcp_mode: None,
        mcp_servers: vec![],
        image_generation_options: None,
        pre_turn_message_count: None,
    };

    let result = orchestrator::execute_turn(services, &turn_input).await?;

    history.extend(result.new_messages.clone());
    *turn_number += 1;

    Ok(result)
}

/// Read the transcript for a session into a `Vec<Message>` history.
pub(crate) async fn load_history(services: &AppServices, session_id: &SessionId) -> Vec<Message> {
    services
        .session_manager
        .read_transcript(session_id)
        .await
        .unwrap_or_default()
}
