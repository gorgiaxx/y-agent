//! Application state shared across all handlers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use y_bot::discord::DiscordBot;
use y_bot::feishu::FeishuBot;
use y_knowledge::config::KnowledgeConfig;
use y_service::knowledge_service::KnowledgeService;
use y_service::ServiceContainer;

use crate::routes::events::SseEvent;

// ---------------------------------------------------------------------------
// Turn metadata (mirrors y-gui TurnMeta)
// ---------------------------------------------------------------------------

/// Cached metadata for the last completed LLM turn in a session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnMeta {
    pub provider_id: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub context_window: usize,
    pub context_tokens_used: u64,
}

// ---------------------------------------------------------------------------
// Knowledge state wrapper
// ---------------------------------------------------------------------------

/// Thread-safe wrapper for storing a `KnowledgeService`.
pub struct KnowledgeState {
    pub service: Arc<tokio::sync::Mutex<KnowledgeService>>,
}

impl KnowledgeState {
    /// Create from a shared `KnowledgeService`.
    pub fn from_shared(service: Arc<tokio::sync::Mutex<KnowledgeService>>) -> Self {
        Self { service }
    }

    /// Create a standalone instance with persistence to the given data directory.
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            service: Arc::new(tokio::sync::Mutex::new(KnowledgeService::with_data_dir(
                KnowledgeConfig::default(),
                data_dir,
            ))),
        }
    }
}

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
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 3000,
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
    /// Knowledge service wrapper (None if not configured).
    pub knowledge: Option<Arc<KnowledgeState>>,
    /// Broadcast channel for SSE events.
    pub event_tx: broadcast::Sender<SseEvent>,
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
            knowledge: None,
            event_tx,
        }
    }

    /// Set the config directory.
    #[must_use]
    pub fn with_config_dir(mut self, dir: PathBuf) -> Self {
        self.config_dir = dir;
        self
    }

    /// Set the knowledge service.
    #[must_use]
    pub fn with_knowledge(mut self, ks: KnowledgeState) -> Self {
        self.knowledge = Some(Arc::new(ks));
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
