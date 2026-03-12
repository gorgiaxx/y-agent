//! Diagnostics trace endpoints.

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use y_service::DiagnosticsService;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Query params for `GET /api/v1/diagnostics/traces`.
#[derive(Debug, Deserialize)]
pub struct ListTracesQuery {
    /// Filter by session ID.
    pub session_id: Option<String>,
    /// Maximum number of traces (default 20).
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/diagnostics/traces`
async fn list_traces(
    State(state): State<AppState>,
    Query(query): Query<ListTracesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let store = state.container.diagnostics.store();
    let limit = query.limit.unwrap_or(20);

    let traces = if let Some(ref sid) = query.session_id {
        store
            .list_traces_by_session(sid, limit)
            .await
            .map_err(|e| ApiError::Internal(format!("{e}")))?
    } else {
        store
            .list_traces(None, None, limit)
            .await
            .map_err(|e| ApiError::Internal(format!("{e}")))?
    };

    Ok(Json(serde_json::to_value(traces).unwrap_or_default()))
}

/// `GET /api/v1/diagnostics/traces/:trace_id`
async fn get_trace(
    State(state): State<AppState>,
    Path(trace_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let uuid = Uuid::parse_str(&trace_id)
        .map_err(|_| ApiError::BadRequest(format!("invalid UUID: {trace_id}")))?;

    let store = state.container.diagnostics.store();

    let trace = DiagnosticsService::get_trace(store.clone(), uuid)
        .await
        .map_err(|e| ApiError::NotFound(format!("trace not found: {e}")))?;

    let observations = DiagnosticsService::get_observations(store, uuid)
        .await
        .unwrap_or_default();

    let detail = serde_json::json!({
        "trace": trace,
        "observations": observations,
    });

    Ok(Json(detail))
}

/// Diagnostics route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/diagnostics/traces", get(list_traces))
        .route("/api/v1/diagnostics/traces/{trace_id}", get(get_trace))
}
