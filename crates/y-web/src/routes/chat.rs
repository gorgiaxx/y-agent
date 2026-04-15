//! Chat turn execution endpoint.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::types::SessionId;
use y_service::{ChatService, PrepareTurnError, PrepareTurnRequest};

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
    /// Thinking effort level: "low", "medium", "high", or "max".
    pub thinking_effort: Option<String>,
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

    // Convert thinking_effort string to ThinkingConfig.
    let thinking = body.thinking_effort.and_then(|e| {
        use y_core::provider::{ThinkingConfig, ThinkingEffort};
        let effort = match e.as_str() {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            _ => return None,
        };
        Some(ThinkingConfig { effort })
    });

    // Prepare turn: resolve/create session, persist user message, read transcript.
    let prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: body.session_id.map(SessionId),
            user_input: body.message,
            provider_id: None,
            skills: None,
            knowledge_collections: None,
            thinking,
            user_message_metadata: None,
            plan_mode: None,
        },
    )
    .await
    .map_err(|e| match e {
        PrepareTurnError::SessionNotFound(msg) => ApiError::NotFound(msg),
        other => ApiError::Internal(other.to_string()),
    })?;

    let session_id = prepared.session_id.clone();
    let input = prepared.as_turn_input();

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
