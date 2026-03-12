//! Health and system status endpoints.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// `GET /health` — liveness probe.
async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: state.version.clone(),
    })
}

/// `GET /api/v1/status` — full system status with diagnostics.
async fn system_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let report = y_service::SystemService::health(&state.container, &state.version).await;
    Json(serde_json::to_value(report).unwrap_or_default())
}

/// Health route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/status", get(system_status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let resp = HealthResponse {
            status: "ok".into(),
            version: "0.1.0".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
    }
}
