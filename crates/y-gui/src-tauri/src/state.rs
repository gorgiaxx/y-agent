//! Application state managed by Tauri.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use y_service::ServiceContainer;

// ---------------------------------------------------------------------------
// Per-session turn metadata (last completed turn)
// ---------------------------------------------------------------------------

/// Cached metadata for the last completed LLM turn in a session.
///
/// Stored in memory in `AppState` so the frontend can restore the status bar
/// when switching between sessions without re-running the turn.
#[derive(Debug, Clone, Serialize)]
pub struct TurnMeta {
    /// Provider ID that handled the turn (e.g. "custom-main").
    pub provider_id: Option<String>,
    /// Model name used (e.g. "gpt-4o").
    pub model: String,
    /// Number of input tokens consumed.
    pub input_tokens: u64,
    /// Number of output tokens generated.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Provider context-window size in tokens.
    pub context_window: usize,
}

/// GUI-specific configuration (persisted to `gui.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    /// Color theme: "dark", "light", or "system".
    pub theme: String,
    /// Base font size in pixels (12–24).
    pub font_size: u16,
    /// Whether Enter key sends (Shift+Enter for newline).
    pub send_on_enter: bool,
    /// Remembered window width.
    pub window_width: u32,
    /// Remembered window height.
    pub window_height: u32,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            font_size: 14,
            send_on_enter: true,
            window_width: 1200,
            window_height: 800,
        }
    }
}

/// Shared application state injected into every Tauri command handler.
pub struct AppState {
    /// The service container holding all wired domain services.
    pub container: Arc<ServiceContainer>,
    /// GUI-specific settings.
    pub gui_config: RwLock<GuiConfig>,
    /// Path to the user config directory (`~/.config/y-agent/`).
    pub config_dir: PathBuf,
    /// In-flight LLM cancellation tokens keyed by `run_id`.
    pub pending_runs: Mutex<HashMap<String, CancellationToken>>,
    /// Last completed turn metadata keyed by `session_id` string.
    ///
    /// Arc-wrapped so the spawned chat task can clone it and write after a
    /// successful turn without holding a reference to `AppState`.
    pub turn_meta_cache: Arc<Mutex<HashMap<String, TurnMeta>>>,
}

impl AppState {
    /// Create a new `AppState`.
    pub fn new(container: Arc<ServiceContainer>, config_dir: PathBuf) -> Self {
        let gui_config = load_gui_config(&config_dir);
        Self {
            container,
            gui_config: RwLock::new(gui_config),
            config_dir,
            pending_runs: Mutex::new(HashMap::new()),
            turn_meta_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Load GUI config from `gui.toml` in the config directory.
fn load_gui_config(config_dir: &std::path::Path) -> GuiConfig {
    let path = config_dir.join("gui.toml");
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    } else {
        GuiConfig::default()
    }
}

/// Persist GUI config to `gui.toml`.
pub fn save_gui_config(config_dir: &std::path::Path, config: &GuiConfig) -> anyhow::Result<()> {
    let path = config_dir.join("gui.toml");
    std::fs::create_dir_all(config_dir)?;
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}
