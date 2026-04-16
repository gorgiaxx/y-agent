//! Observability endpoints -- live system state snapshots.
//!
//! Mirrors the GUI observability panel commands.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use y_service::ObservabilityService;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub since: Option<String>,
    pub until: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/observability/snapshot`
async fn snapshot(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let snap = ObservabilityService::snapshot(&state.container).await;
    serde_json::to_value(&snap)
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// `GET /api/v1/observability/history`
async fn history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let since_dt = query
        .since
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| ApiError::BadRequest(format!("invalid 'since' timestamp: {e}")))
        })
        .transpose()?;
    let until_dt = query
        .until
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| ApiError::BadRequest(format!("invalid 'until' timestamp: {e}")))
        })
        .transpose()?;

    let snap =
        ObservabilityService::snapshot_with_history(&state.container, since_dt, until_dt).await;
    serde_json::to_value(&snap)
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Observability route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/observability/snapshot", get(snapshot))
        .route("/api/v1/observability/history", get(history))
}
