//! Rewind endpoints -- atomic rollback of conversation and filesystem state.
//!
//! Mirrors the GUI rewind commands.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use y_core::types::SessionId;
use y_service::RewindService;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RewindExecuteRequest {
    pub target_message_id: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/rewind/:session_id/points`
async fn list_points(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let points = RewindService::list_rewind_points(&state.container, &sid)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(points))
}

/// `POST /api/v1/rewind/:session_id/execute`
async fn execute(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<RewindExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let result = RewindService::execute_rewind(&state.container, &sid, &body.target_message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(result))
}

/// `POST /api/v1/rewind/:session_id/restore-files`
async fn restore_files(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<RewindExecuteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let report = RewindService::restore_files_only(&state.container, &sid, &body.target_message_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(report))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Rewind route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/rewind/{session_id}/points", get(list_points))
        .route("/api/v1/rewind/{session_id}/execute", post(execute))
        .route(
            "/api/v1/rewind/{session_id}/restore-files",
            post(restore_files),
        )
}
