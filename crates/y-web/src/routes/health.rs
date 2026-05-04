//! Health, system status, provider, and path endpoints.

use std::path::PathBuf;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use y_service::SystemService;

use crate::state::AppState;

const API_SCHEMA_VERSION: &str = "1";
const API_FEATURES: &[&str] = &[
    "agents",
    "app_paths",
    "attachments_read",
    "attachments_upload",
    "background_tasks",
    "bot_webhooks",
    "chat",
    "config",
    "diagnostics",
    "events_session_filter",
    "knowledge",
    "memory_stats",
    "mcp_config",
    "observability",
    "prompt_editing",
    "provider_test",
    "remote_auth",
    "rewind",
    "schedules",
    "sse_events",
    "skills",
    "skill_import_from_path",
    "static_spa",
    "workflows",
    "workspaces",
];

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub api_schema_version: String,
    pub app_version: String,
    pub features: Vec<String>,
}

/// Application paths.
#[derive(Debug, Serialize)]
pub struct AppPaths {
    pub config_dir: String,
    pub data_dir: String,
}

/// Snapshot of in-memory collection sizes for diagnostics.
#[derive(Debug, Serialize)]
pub struct MemoryStats {
    pub pending_runs: usize,
    pub turn_meta_cache: usize,
    pub pruning_watermarks: usize,
    pub session_permission_modes: usize,
    pub pending_interactions: usize,
    pub pending_permissions: usize,
    pub file_history_sessions: usize,
    pub file_history_total_snapshots: usize,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /health` -- liveness probe.
async fn health_check(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: state.version.clone(),
        api_schema_version: API_SCHEMA_VERSION.to_string(),
        app_version: state.version.clone(),
        features: API_FEATURES
            .iter()
            .map(|feature| (*feature).to_string())
            .collect(),
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

/// `GET /api/v1/memory-stats` -- in-memory diagnostic counters.
async fn memory_stats(State(state): State<AppState>) -> Json<MemoryStats> {
    let pending_runs = state.pending_runs.lock().map(|map| map.len()).unwrap_or(0);
    let turn_meta_cache = state
        .turn_meta_cache
        .lock()
        .map(|map| map.len())
        .unwrap_or(0);
    let pruning_watermarks = state.container.pruning_watermarks.read().await.len();
    let session_permission_modes = state.container.session_permission_modes.read().await.len();
    let pending_interactions = state.container.pending_interactions.lock().await.len();
    let pending_permissions = state.container.pending_permissions.lock().await.len();

    let file_history_managers = state.container.file_history_managers.read().await;
    let file_history_sessions = file_history_managers.len();
    let file_history_total_snapshots = file_history_managers
        .values()
        .map(|manager| manager.snapshots().len())
        .sum();

    Json(MemoryStats {
        pending_runs,
        turn_meta_cache,
        pruning_watermarks,
        session_permission_modes,
        pending_interactions,
        pending_permissions,
        file_history_sessions,
        file_history_total_snapshots,
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
    Router::new().route("/health", get(health_check))
}

/// Protected system route group.
pub fn protected_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/status", get(system_status))
        .route("/api/v1/providers", get(provider_list))
        .route("/api/v1/app-paths", get(app_paths))
        .route("/api/v1/memory-stats", get(memory_stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let resp = HealthResponse {
            status: "ok".into(),
            version: "0.1.0".into(),
            api_schema_version: API_SCHEMA_VERSION.into(),
            app_version: "0.1.0".into(),
            features: API_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
        assert!(json.contains("\"api_schema_version\":\"1\""));
        assert!(json.contains("\"app_version\":\"0.1.0\""));
        assert!(json.contains("\"attachments_upload\""));
        assert!(json.contains("\"background_tasks\""));
        assert!(json.contains("\"diagnostics\""));
        assert!(json.contains("\"memory_stats\""));
        assert!(json.contains("\"remote_auth\""));
        assert!(json.contains("\"static_spa\""));
    }

    #[test]
    fn test_memory_stats_serialization() {
        let stats = MemoryStats {
            pending_runs: 1,
            turn_meta_cache: 2,
            pruning_watermarks: 3,
            session_permission_modes: 4,
            pending_interactions: 5,
            pending_permissions: 6,
            file_history_sessions: 7,
            file_history_total_snapshots: 8,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"pending_runs\":1"));
        assert!(json.contains("\"file_history_total_snapshots\":8"));
    }
}
