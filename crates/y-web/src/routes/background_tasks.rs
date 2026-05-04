//! Background task endpoints.
//!
//! Mirrors the GUI background task commands for shared desktop/web UI parity.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use y_service::{BackgroundTaskPollRequest, BackgroundTaskService, BackgroundTaskWriteRequest};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
struct PollBody {
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WriteBody {
    input: String,
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
}

/// `GET /api/v1/sessions/:session_id/background-tasks`
async fn list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    BackgroundTaskService::list(&state.container, session_id)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// `POST /api/v1/sessions/:session_id/background-tasks/:process_id/poll`
async fn poll(
    State(state): State<AppState>,
    Path((session_id, process_id)): Path<(String, String)>,
    Json(body): Json<PollBody>,
) -> Result<impl IntoResponse, ApiError> {
    BackgroundTaskService::poll(
        &state.container,
        BackgroundTaskPollRequest {
            session_id,
            process_id,
            yield_time_ms: body.yield_time_ms,
            max_output_bytes: body.max_output_bytes,
        },
    )
    .await
    .map(Json)
    .map_err(|e| ApiError::Internal(e.to_string()))
}

/// `POST /api/v1/sessions/:session_id/background-tasks/:process_id/write`
async fn write(
    State(state): State<AppState>,
    Path((session_id, process_id)): Path<(String, String)>,
    Json(body): Json<WriteBody>,
) -> Result<impl IntoResponse, ApiError> {
    BackgroundTaskService::write(
        &state.container,
        BackgroundTaskWriteRequest {
            session_id,
            process_id,
            input: body.input,
            yield_time_ms: body.yield_time_ms,
            max_output_bytes: body.max_output_bytes,
        },
    )
    .await
    .map(Json)
    .map_err(|e| ApiError::Internal(e.to_string()))
}

/// `POST /api/v1/sessions/:session_id/background-tasks/:process_id/kill`
async fn kill(
    State(state): State<AppState>,
    Path((session_id, process_id)): Path<(String, String)>,
    Json(body): Json<PollBody>,
) -> Result<impl IntoResponse, ApiError> {
    BackgroundTaskService::kill(
        &state.container,
        BackgroundTaskPollRequest {
            session_id,
            process_id,
            yield_time_ms: body.yield_time_ms,
            max_output_bytes: body.max_output_bytes,
        },
    )
    .await
    .map(Json)
    .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Background task route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/sessions/{session_id}/background-tasks", get(list))
        .route(
            "/api/v1/sessions/{session_id}/background-tasks/{process_id}/poll",
            post(poll),
        )
        .route(
            "/api/v1/sessions/{session_id}/background-tasks/{process_id}/write",
            post(write),
        )
        .route(
            "/api/v1/sessions/{session_id}/background-tasks/{process_id}/kill",
            post(kill),
        )
}
