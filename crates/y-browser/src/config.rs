//! Browser configuration.

use serde::{Deserialize, Serialize};

/// Configuration for browser automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Whether browser tools are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Launch a local Chrome/Chromium process automatically.
    /// When true, y-agent spawns a headless Chrome and manages its lifecycle.
    /// When false, connects to a remote CDP endpoint via `cdp_url`.
    #[serde(default)]
    pub auto_launch: bool,

    /// Path to Chrome/Chromium executable.
    /// Empty string = auto-detect from well-known locations.
    #[serde(default)]
    pub chrome_path: String,

    /// Port for the locally launched Chrome's remote debugging.
    #[serde(default = "default_local_cdp_port")]
    pub local_cdp_port: u16,

    /// CDP endpoint URL (used when `auto_launch` is false).
    /// Supports `http://`, `https://`, `ws://`, `wss://` schemes.
    ///
    /// - HTTP(S): discovers WebSocket URL via `/json/version`
    /// - WS(S): connects directly
    ///
    /// Default: `http://127.0.0.1:9222`
    #[serde(default = "default_cdp_url")]
    pub cdp_url: String,

    /// Default timeout for CDP operations in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    /// Allowed domains for navigation. Empty = all blocked.
    /// Use `["*"]` to allow all public domains.
    #[serde(default)]
    pub allowed_domains: Vec<String>,

    /// Block navigation to private/local network addresses (SSRF protection).
    #[serde(default = "default_true")]
    pub block_private_networks: bool,

    /// Maximum screenshot dimension (width or height) in pixels.
    #[serde(default = "default_max_screenshot_dim")]
    pub max_screenshot_dim: u32,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_launch: false,
            chrome_path: String::new(),
            local_cdp_port: default_local_cdp_port(),
            cdp_url: default_cdp_url(),
            timeout_ms: default_timeout_ms(),
            allowed_domains: vec!["*".into()],
            block_private_networks: true,
            max_screenshot_dim: default_max_screenshot_dim(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_cdp_url() -> String {
    "http://127.0.0.1:9222".into()
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_max_screenshot_dim() -> u32 {
    4096
}

fn default_local_cdp_port() -> u16 {
    9222
}
