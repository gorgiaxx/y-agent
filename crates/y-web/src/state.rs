//! Application state shared across all handlers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use y_bot::discord::DiscordBot;
use y_bot::feishu::FeishuBot;
use y_service::ServiceContainer;

use crate::routes::events::SseEvent;

pub use y_service::chat_types::TurnMeta;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the web server.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Host to bind to.
    pub host: String,
    /// Port to bind to.
    pub port: u16,
    /// Optional bearer token for authentication.
    pub auth_token: Option<String>,
    /// Optional path to static SPA assets (dist-web/).
    pub static_dir: Option<PathBuf>,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
            auth_token: None,
            static_dir: None,
        }
    }
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Shared application state, injected into every handler via axum `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// The service container holding all wired domain services.
    pub container: Arc<ServiceContainer>,
    /// Application version string.
    pub version: String,
    /// Path to the user config directory (`~/.config/y-agent/`).
    pub config_dir: PathBuf,
    /// Feishu bot adapter (None if not configured).
    pub feishu_bot: Option<Arc<FeishuBot>>,
    /// Discord bot adapter (None if not configured).
    pub discord_bot: Option<Arc<DiscordBot>>,
    /// In-flight LLM cancellation tokens keyed by `run_id`.
    pub pending_runs: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Last completed turn metadata keyed by `session_id`.
    pub turn_meta_cache: Arc<Mutex<HashMap<String, TurnMeta>>>,
    /// Broadcast channel for SSE events.
    pub event_tx: broadcast::Sender<SseEvent>,
    /// Optional bearer token for authentication.
    pub auth_token: Option<String>,
    /// Optional path to static SPA assets for serving the web frontend.
    pub static_dir: Option<PathBuf>,
}

impl AppState {
    /// Create a new `AppState`.
    pub fn new(container: Arc<ServiceContainer>, version: &str) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            container,
            version: version.to_string(),
            config_dir: PathBuf::new(),
            feishu_bot: None,
            discord_bot: None,
            pending_runs: Arc::new(Mutex::new(HashMap::new())),
            turn_meta_cache: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            auth_token: None,
            static_dir: None,
        }
    }

    /// Set the config directory.
    #[must_use]
    pub fn with_config_dir(mut self, dir: PathBuf) -> Self {
        self.config_dir = dir;
        self
    }

    /// Set the authentication token.
    #[must_use]
    pub fn with_auth_token(mut self, token: Option<String>) -> Self {
        self.auth_token = token;
        self
    }

    /// Set the static SPA directory.
    #[must_use]
    pub fn with_static_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.static_dir = dir;
        self
    }

    /// Create a new `AppState` with a Feishu bot adapter.
    #[must_use]
    pub fn with_feishu_bot(mut self, bot: FeishuBot) -> Self {
        self.feishu_bot = Some(Arc::new(bot));
        self
    }

    /// Create a new `AppState` with a Discord bot adapter.
    #[must_use]
    pub fn with_discord_bot(mut self, bot: DiscordBot) -> Self {
        self.discord_bot = Some(Arc::new(bot));
        self
    }
}
