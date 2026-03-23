//! Browser configuration.

use serde::{Deserialize, Serialize};

/// How Chrome is launched and connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// Connect to a remote Chrome instance via CDP URL (default).
    #[default]
    Remote,
    /// Auto-launch a headless Chrome process.
    AutoLaunchHeadless,
    /// Auto-launch Chrome with a visible window (for debugging).
    AutoLaunchVisible,
}

impl LaunchMode {
    /// Whether to auto-launch a local Chrome process.
    pub fn is_auto_launch(&self) -> bool {
        matches!(
            self,
            LaunchMode::AutoLaunchHeadless | LaunchMode::AutoLaunchVisible
        )
    }

    /// Whether Chrome should run headless.
    pub fn is_headless(&self) -> bool {
        matches!(self, LaunchMode::Remote | LaunchMode::AutoLaunchHeadless)
    }
}

/// Configuration for browser automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Whether browser tools are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// How Chrome is launched.
    #[serde(default)]
    pub launch_mode: LaunchMode,

    /// Path to Chrome/Chromium executable.
    /// Empty string = auto-detect from well-known locations.
    #[serde(default)]
    pub chrome_path: String,

    /// Port for the locally launched Chrome's remote debugging.
    #[serde(default = "default_local_cdp_port")]
    pub local_cdp_port: u16,

    /// Use the current system user's Chrome profile (bookmarks, cookies,
    /// extensions, login sessions, etc.) instead of a clean temporary profile.
    ///
    /// When `false` (default), Chrome launches with an isolated temporary
    /// profile in the system temp directory.
    ///
    /// When `true`, Chrome uses the default user data directory:
    /// - macOS: `~/Library/Application Support/Google/Chrome`
    /// - Windows: `%LOCALAPPDATA%\Google\Chrome\User Data`
    /// - Linux: `~/.config/google-chrome`
    ///
    /// NOTE: Chrome locks its profile directory while running. If Chrome is
    /// already open with the same profile, the auto-launched instance may
    /// fail or merge into the existing window. Close other Chrome instances
    /// before enabling this option with auto-launch mode.
    #[serde(default)]
    pub use_user_profile: bool,

    /// CDP endpoint URL (used when `launch_mode` is Remote).
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

    /// Default search engine for the `search` action.
    /// Supported values: "google", "bing", "duckduckgo", "baidu".
    #[serde(default = "default_search_engine")]
    pub default_search_engine: String,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            launch_mode: LaunchMode::default(),
            chrome_path: String::new(),
            local_cdp_port: default_local_cdp_port(),
            use_user_profile: false,
            cdp_url: default_cdp_url(),
            timeout_ms: default_timeout_ms(),
            allowed_domains: vec!["*".into()],
            block_private_networks: true,
            max_screenshot_dim: default_max_screenshot_dim(),
            default_search_engine: default_search_engine(),
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

fn default_search_engine() -> String {
    "google".into()
}
