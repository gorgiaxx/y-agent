//! System status and health command handlers.

use serde::Serialize;
use tauri::State;

use y_core::session::SessionFilter;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// System status returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct SystemStatus {
    /// Application version.
    pub version: String,
    /// Whether the service is operational.
    pub healthy: bool,
    /// Number of configured providers.
    pub provider_count: usize,
    /// Active session count (if available).
    pub session_count: Option<usize>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Get system status.
#[tauri::command]
pub async fn system_status(state: State<'_, AppState>) -> Result<SystemStatus, String> {
    let provider_count = state.container.provider_pool().await.list_metadata().len();

    let filter = SessionFilter::default();
    let session_count = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map(|s| s.len())
        .ok();

    Ok(SystemStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        healthy: true,
        provider_count,
        session_count,
    })
}

/// Quick health check.
#[tauri::command]
pub async fn health_check() -> Result<String, String> {
    Ok("ok".to_string())
}
