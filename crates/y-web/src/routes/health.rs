//! Health, system status, provider, and path endpoints.

use std::path::PathBuf;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use y_service::SystemService;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Application paths.
#[derive(Debug, Serialize)]
pub struct AppPaths {
    pub config_dir: String,
    pub data_dir: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /health` -- liveness probe.
async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: state.version.clone(),
    })
}

/// `GET /api/v1/status` -- full system status with diagnostics.
async fn system_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let report = SystemService::health(&state.container, &state.version).await;
    Json(serde_json::to_value(report).unwrap_or_default())
}

/// `GET /api/v1/providers` -- list all configured providers.
async fn provider_list(State(state): State<AppState>) -> Json<serde_json::Value> {
    let providers = SystemService::list_providers(&state.container).await;
    Json(serde_json::to_value(providers).unwrap_or_default())
}

/// `GET /api/v1/app-paths` -- return config and data directory paths.
async fn app_paths(State(state): State<AppState>) -> Json<AppPaths> {
    let config = state.config_dir.display().to_string();
    let data = data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Json(AppPaths {
        config_dir: config,
        data_dir: data,
    })
}

/// Get the XDG state base directory for y-agent.
fn data_dir() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        });
    state_home.map(|s| s.join("y-agent"))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Health route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health_check))
        .route("/api/v1/status", get(system_status))
        .route("/api/v1/providers", get(provider_list))
        .route("/api/v1/app-paths", get(app_paths))
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
