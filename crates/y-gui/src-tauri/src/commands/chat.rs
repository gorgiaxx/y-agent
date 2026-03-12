//! Chat command handlers — send messages and stream LLM responses.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::SessionId;
use y_service::{ChatService, TurnInput};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response / event types
// ---------------------------------------------------------------------------

/// Returned immediately when a chat turn is started.
#[derive(Debug, Serialize, Clone)]
pub struct ChatStarted {
    /// Session ID (may have been auto-created).
    pub session_id: String,
    /// Unique run identifier for event correlation.
    pub run_id: String,
}

/// Payload emitted on `chat:complete`.
#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletePayload {
    pub run_id: String,
    pub content: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallInfo>,
    pub iterations: usize,
}

/// Tool call summary in the completion payload.
#[derive(Debug, Serialize, Clone)]
pub struct ToolCallInfo {
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Payload emitted on `chat:error`.
#[derive(Debug, Serialize, Clone)]
pub struct ChatErrorPayload {
    pub run_id: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Send a chat message and receive streaming response via Tauri events.
///
/// Events emitted:
/// - `chat:complete` — full response on success
/// - `chat:error` — error details on failure
#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    message: String,
    session_id: Option<String>,
) -> Result<ChatStarted, String> {
    if message.trim().is_empty() {
        return Err("Message must not be empty".into());
    }

    let run_id = Uuid::new_v4().to_string();

    // Resolve or create session.
    let sid = match session_id {
        Some(ref id) => {
            let id = SessionId(id.clone());
            state
                .container
                .session_manager
                .get_session(&id)
                .await
                .map_err(|e| format!("Session not found: {e}"))?;
            id
        }
        None => {
            let session = state
                .container
                .session_manager
                .create_session(CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: None,
                })
                .await
                .map_err(|e| format!("Failed to create session: {e}"))?;
            session.id
        }
    };

    let result_sid = sid.0.clone();
    let result_run_id = run_id.clone();

    // Persist user message.
    let user_msg = y_core::types::Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: message.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    };
    let _ = state
        .container
        .session_manager
        .append_message(&sid, &user_msg)
        .await;

    // Spawn async LLM turn — results streamed via events.
    let container = state.container.clone();
    let sid_clone = sid.clone();
    let run_id_clone = run_id.clone();

    tokio::spawn(async move {
        let history = container
            .session_manager
            .read_transcript(&sid_clone)
            .await
            .unwrap_or_default();

        let turn_number = u32::try_from(history.len()).unwrap_or(u32::MAX);
        let session_uuid = Uuid::new_v4();

        let input = TurnInput {
            user_input: &message,
            session_id: sid_clone,
            session_uuid,
            history: &history,
            turn_number,
        };

        match ChatService::execute_turn(&container, &input).await {
            Ok(result) => {
                let _ = app.emit(
                    "chat:complete",
                    ChatCompletePayload {
                        run_id: run_id_clone,
                        content: result.content,
                        model: result.model,
                        input_tokens: result.input_tokens,
                        output_tokens: result.output_tokens,
                        cost_usd: result.cost_usd,
                        tool_calls: result
                            .tool_calls_executed
                            .iter()
                            .map(|tc| ToolCallInfo {
                                name: tc.name.clone(),
                                success: tc.success,
                                duration_ms: tc.duration_ms,
                            })
                            .collect(),
                        iterations: result.iterations,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "chat:error",
                    ChatErrorPayload {
                        run_id: run_id_clone,
                        error: e.to_string(),
                    },
                );
            }
        }
    });

    Ok(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    })
}
