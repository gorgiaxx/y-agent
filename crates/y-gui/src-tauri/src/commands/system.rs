//! System status and health command handlers.

use std::path::{Path, PathBuf};
use std::process::Command;
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

/// IDE option returned to Settings > General.
#[derive(Debug, Serialize, Clone)]
pub struct IdeInfo {
    /// Stable IDE identifier persisted in GUI config.
    pub id: String,
    /// User-facing IDE name.
    pub name: String,
    /// Preferred command or application name.
    pub command: String,
    /// Whether this IDE was detected on the current machine.
    pub available: bool,
}

#[derive(Debug, Clone, Copy)]
struct IdeCandidate {
    id: &'static str,
    name: &'static str,
    cli: &'static str,
    mac_app: Option<&'static str>,
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

/// List IDEs that can be used to open local file paths from tool-call labels.
#[tauri::command]
pub async fn ide_list(_state: State<'_, AppState>) -> Result<Vec<IdeInfo>, String> {
    let ide_options: Vec<IdeInfo> = ide_candidates_for_platform()
        .iter()
        .map(candidate_to_ide_info)
        .collect();
    let has_available_ide = ide_options.iter().any(|ide| ide.available);

    let mut options = vec![IdeInfo {
        id: "auto".to_string(),
        name: "Auto Detect".to_string(),
        command: "First available IDE".to_string(),
        available: has_available_ide,
    }];
    options.extend(ide_options);
    Ok(options)
}

/// Open a file path in the configured local IDE.
#[tauri::command]
pub async fn open_path_in_ide(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let selected_ide = state.gui_config.read().await.default_file_ide.clone();
    let candidates = ide_candidates_for_platform();
    let candidate = if selected_ide == "auto" {
        candidates
            .iter()
            .find(|candidate| candidate_available(candidate))
    } else {
        candidates
            .iter()
            .find(|candidate| candidate.id == selected_ide)
    };

    let Some(candidate) = candidate else {
        return Err("No local IDE detected. Configure Default File IDE in Settings.".to_string());
    };

    if !candidate_available(candidate) {
        return Err(format!(
            "{} was not found on this machine. Choose another Default File IDE in Settings.",
            candidate.name
        ));
    }

    open_path_with_candidate(candidate, Path::new(&path))
}

/// Get the XDG state base directory for y-agent (`~/.local/state/y-agent/`).
///
/// Mirrors the `state_dir()` helper in `lib.rs`.
fn data_dir() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::home_dir().map(|h| h.join(".local").join("state")));
    state_home.map(|s| s.join("y-agent"))
}

fn ide_candidates_for_platform() -> Vec<IdeCandidate> {
    #[cfg(target_os = "macos")]
    {
        macos_ide_candidates()
    }

    #[cfg(target_os = "windows")]
    {
        return windows_ide_candidates();
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        linux_ide_candidates()
    }
}

#[cfg(target_os = "macos")]
fn macos_ide_candidates() -> Vec<IdeCandidate> {
    vec![
        IdeCandidate {
            id: "cursor",
            name: "Cursor",
            cli: "cursor",
            mac_app: Some("Cursor"),
        },
        IdeCandidate {
            id: "vscode",
            name: "VS Code",
            cli: "code",
            mac_app: Some("Visual Studio Code"),
        },
        IdeCandidate {
            id: "xcode",
            name: "Xcode",
            cli: "xed",
            mac_app: Some("Xcode"),
        },
        IdeCandidate {
            id: "antigravity",
            name: "Antigravity",
            cli: "antigravity",
            mac_app: Some("Antigravity"),
        },
        IdeCandidate {
            id: "zed",
            name: "Zed",
            cli: "zed",
            mac_app: Some("Zed"),
        },
        IdeCandidate {
            id: "windsurf",
            name: "Windsurf",
            cli: "windsurf",
            mac_app: Some("Windsurf"),
        },
        IdeCandidate {
            id: "intellij",
            name: "IntelliJ IDEA",
            cli: "idea",
            mac_app: Some("IntelliJ IDEA"),
        },
        IdeCandidate {
            id: "webstorm",
            name: "WebStorm",
            cli: "webstorm",
            mac_app: Some("WebStorm"),
        },
        IdeCandidate {
            id: "rustrover",
            name: "RustRover",
            cli: "rustrover",
            mac_app: Some("RustRover"),
        },
        IdeCandidate {
            id: "sublime",
            name: "Sublime Text",
            cli: "subl",
            mac_app: Some("Sublime Text"),
        },
    ]
}

#[cfg(target_os = "windows")]
fn windows_ide_candidates() -> Vec<IdeCandidate> {
    vec![
        IdeCandidate {
            id: "cursor",
            name: "Cursor",
            cli: "cursor",
            mac_app: None,
        },
        IdeCandidate {
            id: "vscode",
            name: "VS Code",
            cli: "code",
            mac_app: None,
        },
        IdeCandidate {
            id: "antigravity",
            name: "Antigravity",
            cli: "antigravity",
            mac_app: None,
        },
        IdeCandidate {
            id: "windsurf",
            name: "Windsurf",
            cli: "windsurf",
            mac_app: None,
        },
        IdeCandidate {
            id: "intellij",
            name: "IntelliJ IDEA",
            cli: "idea64",
            mac_app: None,
        },
        IdeCandidate {
            id: "webstorm",
            name: "WebStorm",
            cli: "webstorm64",
            mac_app: None,
        },
        IdeCandidate {
            id: "rustrover",
            name: "RustRover",
            cli: "rustrover64",
            mac_app: None,
        },
        IdeCandidate {
            id: "sublime",
            name: "Sublime Text",
            cli: "subl",
            mac_app: None,
        },
    ]
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn linux_ide_candidates() -> Vec<IdeCandidate> {
    vec![
        IdeCandidate {
            id: "cursor",
            name: "Cursor",
            cli: "cursor",
            mac_app: None,
        },
        IdeCandidate {
            id: "vscode",
            name: "VS Code",
            cli: "code",
            mac_app: None,
        },
        IdeCandidate {
            id: "antigravity",
            name: "Antigravity",
            cli: "antigravity",
            mac_app: None,
        },
        IdeCandidate {
            id: "zed",
            name: "Zed",
            cli: "zed",
            mac_app: None,
        },
        IdeCandidate {
            id: "windsurf",
            name: "Windsurf",
            cli: "windsurf",
            mac_app: None,
        },
        IdeCandidate {
            id: "intellij",
            name: "IntelliJ IDEA",
            cli: "idea",
            mac_app: None,
        },
        IdeCandidate {
            id: "webstorm",
            name: "WebStorm",
            cli: "webstorm",
            mac_app: None,
        },
        IdeCandidate {
            id: "rustrover",
            name: "RustRover",
            cli: "rustrover",
            mac_app: None,
        },
        IdeCandidate {
            id: "sublime",
            name: "Sublime Text",
            cli: "subl",
            mac_app: None,
        },
    ]
}

fn candidate_to_ide_info(candidate: &IdeCandidate) -> IdeInfo {
    IdeInfo {
        id: candidate.id.to_string(),
        name: candidate.name.to_string(),
        command: candidate_command_label(candidate),
        available: candidate_available(candidate),
    }
}

fn candidate_command_label(candidate: &IdeCandidate) -> String {
    if command_exists(candidate.cli) {
        return candidate.cli.to_string();
    }

    if let Some(app_name) = candidate.mac_app {
        if mac_app_exists(app_name) {
            return format!("open -a {app_name}");
        }
    }

    candidate.cli.to_string()
}

fn candidate_available(candidate: &IdeCandidate) -> bool {
    command_exists(candidate.cli) || candidate.mac_app.is_some_and(mac_app_exists)
}

fn open_path_with_candidate(candidate: &IdeCandidate, path: &Path) -> Result<(), String> {
    if command_exists(candidate.cli) {
        let mut command = Command::new(candidate.cli);
        command.arg(path);
        return spawn_ide_command(command, candidate.name);
    }

    if let Some(app_name) = candidate.mac_app {
        if mac_app_exists(app_name) {
            let mut command = Command::new("open");
            command.arg("-a").arg(app_name).arg(path);
            return spawn_ide_command(command, candidate.name);
        }
    }

    Err(format!("{} was not found on this machine.", candidate.name))
}

fn spawn_ide_command(mut command: Command, ide_name: &str) -> Result<(), String> {
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open file in {ide_name}: {e}"))
}

fn command_exists(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| command_path_exists(&dir, command))
}

#[cfg(not(target_os = "windows"))]
fn command_path_exists(dir: &Path, command: &str) -> bool {
    dir.join(command).is_file()
}

#[cfg(target_os = "windows")]
fn command_path_exists(dir: &Path, command: &str) -> bool {
    let base = dir.join(command);
    if base.is_file() {
        return true;
    }

    let extensions = std::env::var_os("PATHEXT")
        .map(|value| {
            value
                .to_string_lossy()
                .split(';')
                .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec!["exe".to_string(), "cmd".to_string(), "bat".to_string()]);

    extensions
        .iter()
        .any(|extension| dir.join(format!("{command}.{extension}")).is_file())
}

#[cfg(target_os = "macos")]
fn mac_app_exists(app_name: &str) -> bool {
    let app_bundle = format!("{app_name}.app");
    [
        PathBuf::from("/Applications"),
        std::env::var_os("HOME")
            .map_or_else(|| PathBuf::from(""), PathBuf::from)
            .join("Applications"),
    ]
    .iter()
    .any(|dir| dir.join(&app_bundle).is_dir())
}

#[cfg(not(target_os = "macos"))]
fn mac_app_exists(_app_name: &str) -> bool {
    false
}

/// Quick health check.
#[tauri::command]
pub async fn health_check() -> Result<String, String> {
    Ok("ok".to_string())
}

/// Download a remote image and save it to the specified local path.
#[tauri::command]
pub async fn save_remote_image(url: String, dest: String) -> Result<(), String> {
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("Failed to fetch image: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    std::fs::write(&dest, &bytes).map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(())
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
