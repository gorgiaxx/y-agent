//! y-browser: Browser automation via Chrome `DevTools` Protocol (CDP).
//!
//! This crate provides browser control for y-agent by connecting to
//! Chrome/Chromium via CDP WebSocket. It can optionally manage Chrome's
//! lifecycle (auto-launch a local headless instance) or connect to a
//! remote CDP provider (Browserless, Browserbase, etc.).
//!
//! # Architecture
//!
//! - [`CdpClient`] — WebSocket JSON-RPC client for CDP
//! - `BrowserActions` -- high-level operations (navigate, screenshot, click, etc.)
//! - [`BrowserTool`] — implements `y-core::tool::Tool` for agent integration
//! - [`SecurityPolicy`] — domain allowlist + SSRF protection
//! - [`ChromeLauncher`] — local Chrome process lifecycle manager

pub mod actions;
pub mod cdp_client;
pub mod config;
pub mod launcher;
pub mod security;
pub mod snapshot;
pub mod tool;

pub use cdp_client::CdpClient;
pub use config::BrowserConfig;
pub use launcher::ChromeLauncher;
pub use security::SecurityPolicy;
pub use tool::BrowserTool;
