//! Session management endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::session::{CreateSessionOptions, SessionFilter, SessionState, SessionType};
use y_core::types::SessionId;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Query params for `GET /api/v1/sessions`.
#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    /// Filter by state: "Active", "Archived", or unset for all.
    pub state: Option<String>,
}

/// Request body for `POST /api/v1/sessions`.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
}

/// Request body for `POST /api/v1/sessions/:id/branch`.
#[derive(Debug, Deserialize)]
pub struct BranchRequest {
    pub label: Option<String>,
}

/// Minimal success message.
#[derive(Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/sessions`
async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let filter = match query.state.as_deref() {
        Some("Active") => SessionFilter {
            state: Some(SessionState::Active),
            ..Default::default()
        },
        Some("Archived") => SessionFilter {
            state: Some(SessionState::Archived),
            ..Default::default()
        },
        _ => SessionFilter::default(),
    };

    let sessions = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(serde_json::to_value(sessions).unwrap_or_default()))
}

/// `POST /api/v1/sessions`
async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<Option<CreateSessionRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let title = body.and_then(|b| b.title);
    let session = state
        .container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: title.clone(),
        })
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(session).unwrap_or_default()),
    ))
}

/// `GET /api/v1/sessions/:id`
async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let session = state
        .container
        .session_manager
        .get_session(&id)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(serde_json::to_value(session).unwrap_or_default()))
}

/// `POST /api/v1/sessions/:id/archive`
async fn archive_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    state
        .container
        .session_manager
        .transition_state(&id, SessionState::Archived)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(MessageResponse {
        message: format!("session {session_id} archived"),
    }))
}

/// `POST /api/v1/sessions/:id/branch`
async fn branch_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<Option<BranchRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let label = body.and_then(|b| b.label);
    let branch = state
        .container
        .session_manager
        .branch(&id, label)
        .await
        .map_err(|e| ApiError::Internal(format!("branch failed: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(branch).unwrap_or_default()),
    ))
}

/// `GET /api/v1/sessions/:id/messages`
async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let messages = state
        .container
        .session_manager
        .read_transcript(&id)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    let selected: Vec<_> = if let Some(n) = params.last {
        messages
            .into_iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        messages
    };

    Ok(Json(serde_json::to_value(selected).unwrap_or_default()))
}

/// Query params for message listing.
#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub last: Option<usize>,
}

/// Session route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/sessions", get(list_sessions).post(create_session))
        .route("/api/v1/sessions/{session_id}", get(get_session))
        .route(
            "/api/v1/sessions/{session_id}/archive",
            post(archive_session),
        )
        .route("/api/v1/sessions/{session_id}/branch", post(branch_session))
        .route("/api/v1/sessions/{session_id}/messages", get(list_messages))
}
