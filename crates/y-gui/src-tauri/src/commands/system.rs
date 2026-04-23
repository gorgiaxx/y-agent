//! System status and health command handlers.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Heartbeat pong from the frontend webview.
///
/// Called periodically by the frontend to signal that the `WKWebView` content
/// process is alive. The Rust-side monitor checks this timestamp to detect
/// when macOS has terminated the content process (blank screen).
#[tauri::command]
pub async fn heartbeat_pong(state: State<'_, AppState>) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    state.last_heartbeat_pong.store(now, Ordering::Relaxed);
    Ok(())
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
///
/// In release builds this is a no-op: devtools are disabled in production
/// to prevent end-users from accessing the inspector or reloading the page.
#[tauri::command]
pub async fn toggle_devtools(window: tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    {
        if window.is_devtools_open() {
            window.close_devtools();
        } else {
            window.open_devtools();
        }
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = window;
    }
}

/// Apply the window decoration mode to the main window.
///
/// Platform behavior:
/// - **macOS**: toggles the title bar style between `Overlay` (custom
///   decorations on -- layered chrome, no native title) and `Visible`
///   (custom decorations off -- standard macOS title bar with traffic
///   lights and window title). Unlike `set_decorations(false)` which
///   removes the traffic lights entirely, `set_title_bar_style` preserves
///   them in both modes.
/// - **Linux / Windows**: `set_decorations(!use_custom)` is applied so the
///   frontend can draw its own chrome (min / max / close buttons). Linux
///   compositors (KDE, GNOME) often mishandle client-side decorations; the
///   user-facing toggle exists precisely so they can fall back to native.
#[tauri::command]
pub async fn window_set_decorations(
    window: tauri::WebviewWindow,
    use_custom: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let style = if use_custom {
            tauri::TitleBarStyle::Overlay
        } else {
            tauri::TitleBarStyle::Visible
        };
        window
            .set_title_bar_style(style)
            .map_err(|e| format!("Failed to set title bar style: {e}"))?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_decorations(!use_custom)
            .map_err(|e| format!("Failed to set window decorations: {e}"))?;
    }

    Ok(())
}

/// Minimize the main window (called by the custom titlebar on Linux/Windows).
#[tauri::command]
pub async fn window_minimize(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

/// Toggle maximized state (called by the custom titlebar on Linux/Windows).
#[tauri::command]
pub async fn window_toggle_maximize(window: tauri::WebviewWindow) -> Result<(), String> {
    let is_max = window.is_maximized().map_err(|e| e.to_string())?;
    if is_max {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

/// Close the main window (called by the custom titlebar on Linux/Windows).
#[tauri::command]
pub async fn window_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Memory stats (diagnostics endpoint for monitoring memory growth)
// ---------------------------------------------------------------------------

/// Snapshot of in-memory collection sizes for debugging memory growth.
#[derive(Debug, Serialize, Clone)]
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

/// Return current sizes of key in-memory collections.
///
/// Intended for the diagnostics / observability panel so users can spot
/// unbounded growth without attaching a profiler.
#[tauri::command]
pub async fn memory_stats(state: State<'_, AppState>) -> Result<MemoryStats, String> {
    let pending_runs = state.pending_runs.lock().map(|m| m.len()).unwrap_or(0);
    let turn_meta_cache = state.turn_meta_cache.lock().map(|m| m.len()).unwrap_or(0);
    let pruning_watermarks = state.container.pruning_watermarks.read().await.len();
    let session_permission_modes = state.container.session_permission_modes.read().await.len();
    let pending_interactions = state.container.pending_interactions.lock().await.len();
    let pending_permissions = state.container.pending_permissions.lock().await.len();

    let fhm = state.container.file_history_managers.read().await;
    let file_history_sessions = fhm.len();
    let file_history_total_snapshots: usize = fhm.values().map(|m| m.snapshots().len()).sum();
    drop(fhm);

    Ok(MemoryStats {
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

/// Sync the native window theme with the app's resolved theme.
///
/// On macOS this drives the vibrancy material appearance so the frosted-glass
/// sidebar matches the app's dark/light mode regardless of the system setting.
#[tauri::command]
pub async fn window_set_theme(window: tauri::WebviewWindow, theme: String) -> Result<(), String> {
    let native_theme = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        _ => None,
    };
    window
        .set_theme(native_theme)
        .map_err(|e| format!("Failed to set window theme: {e}"))
}
