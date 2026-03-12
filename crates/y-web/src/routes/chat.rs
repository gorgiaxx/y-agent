//! Chat turn execution endpoint.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::SessionId;
use y_service::{ChatService, TurnInput};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/chat`.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// User message content.
    pub message: String,
    /// Session ID to continue. Auto-creates a new session if omitted.
    pub session_id: Option<String>,
}

/// Successful chat response.
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    /// Assistant response text.
    pub content: String,
    /// Model that served the request.
    pub model: String,
    /// Session ID (useful when auto-created).
    pub session_id: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Tool calls executed during this turn.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Number of LLM iterations.
    pub iterations: usize,
}

/// Tool call record in the response.
#[derive(Debug, Serialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `POST /api/v1/chat` — execute a single chat turn.
async fn chat_turn(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("message must not be empty".into()));
    }

    // Resolve or create session.
    let session_id = if let Some(ref sid) = body.session_id {
        let id = SessionId(sid.clone());
        // Verify the session exists.
        let _ = state
            .container
            .session_manager
            .get_session(&id)
            .await
            .map_err(|_| ApiError::NotFound(format!("session {sid} not found")))?;
        id
    } else {
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
            .map_err(|e| ApiError::Internal(format!("failed to create session: {e}")))?;
        session.id
    };

    // Generate a UUID for diagnostics tracing.
    let session_uuid = Uuid::new_v4();

    // Persist the user message.
    let user_msg = y_core::types::Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: body.message.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    };
    let _ = state
        .container
        .session_manager
        .append_message(&session_id, &user_msg)
        .await;

    // Read current transcript for context.
    let history = state
        .container
        .session_manager
        .read_transcript(&session_id)
        .await
        .unwrap_or_default();

    let turn_number = u32::try_from(history.len()).unwrap_or(u32::MAX);

    let input = TurnInput {
        user_input: &body.message,
        session_id: session_id.clone(),
        session_uuid,
        history: &history,
        turn_number,
    };

    let result = ChatService::execute_turn(&state.container, &input)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(ChatResponse {
        content: result.content,
        model: result.model,
        session_id: session_id.0,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        cost_usd: result.cost_usd,
        tool_calls: result
            .tool_calls_executed
            .iter()
            .map(|tc| ToolCallRecord {
                name: tc.name.clone(),
                success: tc.success,
                duration_ms: tc.duration_ms,
            })
            .collect(),
        iterations: result.iterations,
    }))
}

/// Chat route group.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/chat", post(chat_turn))
}
