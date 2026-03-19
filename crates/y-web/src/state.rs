//! Application state shared across all handlers.

use std::sync::Arc;

use y_bot::feishu::FeishuBot;
use y_service::ServiceContainer;

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

/// Shared application state, injected into every handler via axum `State` extractor.
#[derive(Clone)]
pub struct AppState {
    /// The service container holding all wired domain services.
    pub container: Arc<ServiceContainer>,
    /// Application version string.
    pub version: String,
    /// Feishu bot adapter (None if not configured).
    pub feishu_bot: Option<Arc<FeishuBot>>,
}

impl AppState {
    /// Create a new `AppState`.
    pub fn new(container: Arc<ServiceContainer>, version: &str) -> Self {
        Self {
            container,
            version: version.to_string(),
            feishu_bot: None,
        }
    }

    /// Create a new `AppState` with a Feishu bot adapter.
    #[must_use]
    pub fn with_feishu_bot(mut self, bot: FeishuBot) -> Self {
        self.feishu_bot = Some(Arc::new(bot));
        self
    }
}
