//! System status and health command handlers.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use y_core::session::SessionFilter;
use y_service::{ProviderInfo, SystemService};

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
    let provider_count = SystemService::list_providers(&state.container).await.len();

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

// ---------------------------------------------------------------------------
// Application paths
// ---------------------------------------------------------------------------

/// Paths returned to the frontend for display in Settings > General.
#[derive(Debug, Serialize, Clone)]
pub struct AppPaths {
    /// Config directory path (e.g. `~/.config/y-agent/`).
    pub config_dir: String,
    /// Data directory path (e.g. `~/.local/state/y-agent/`).
    pub data_dir: String,
}

/// Return the config and data directory paths for display.
#[tauri::command]
pub async fn app_paths(state: State<'_, AppState>) -> Result<AppPaths, String> {
    let config = state.config_dir.display().to_string();
    let data = data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Ok(AppPaths {
        config_dir: config,
        data_dir: data,
    })
}

/// Get the XDG state base directory for y-agent (`~/.local/state/y-agent/`).
///
/// Mirrors the `state_dir()` helper in `lib.rs`.
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

/// Quick health check.
#[tauri::command]
pub async fn health_check() -> Result<String, String> {
    Ok("ok".to_string())
}

// ---------------------------------------------------------------------------
// Provider list
// ---------------------------------------------------------------------------

/// List all configured providers (id, model, type) for the frontend selector.
#[tauri::command]
pub async fn provider_list(state: State<'_, AppState>) -> Result<Vec<ProviderInfo>, String> {
    Ok(SystemService::list_providers(&state.container).await)
}

// ---------------------------------------------------------------------------
// DevTools
// ---------------------------------------------------------------------------

/// Show the main window.
///
/// Called by the frontend after the initial render completes to avoid the
/// white-flash that occurs when the webview loads with a blank background.
/// The window starts hidden (`visible: false` in `tauri.conf.json`) and is
/// shown only once the React tree is mounted and CSS has been applied.
#[tauri::command]
pub async fn show_window(window: tauri::WebviewWindow) {
    let _ = window.show();
}

/// Toggle the `WebView` developer tools (Ctrl+Shift+I shortcut handler).
#[tauri::command]
pub async fn toggle_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}
