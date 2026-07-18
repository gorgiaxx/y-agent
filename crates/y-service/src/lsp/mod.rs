//! Service-owned Language Server Protocol configuration and lifecycle.

#[cfg(feature = "lsp")]
mod client;
mod codec;
mod config;
#[cfg(feature = "lsp")]
mod manager;
#[cfg(feature = "lsp")]
mod runtime_transport;

#[cfg(feature = "lsp")]
pub use client::{LspClient, LspClientError, LspConnection, LspConnector};
pub use codec::{LspFrameDecoder, LspFrameError};
pub use config::{LspConfig, LspServerConfig};
#[cfg(feature = "lsp")]
pub use manager::LspManager;
#[cfg(feature = "lsp")]
pub use runtime_transport::RuntimeLspConnector;
