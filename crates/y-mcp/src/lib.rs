//! y-mcp: MCP protocol support for third-party tools and memory.
//!
//! This crate provides the Model Context Protocol (MCP) integration for y-agent:
//!
//! - [`McpClient`] — connects to MCP servers via pluggable transports
//! - [`McpToolAdapter`] — wraps MCP-hosted tools as y-core [`Tool`](y_core::tool::Tool)
//! - [`McpTransport`] — transport abstraction (stdio, HTTP)
//! - [`discovery`] — server and tool discovery
//!
//! # Design
//!
//! MCP tools are discovered at startup via `tools/list` and registered with
//! the tool registry as [`ToolType::Mcp`](y_core::tool::ToolType::Mcp).
//! Tool calls are proxied via `tools/call` over the configured transport.
//! Transport implementations (stdio, HTTP/SSE) are pluggable.

pub mod auth;
pub mod client;
pub mod discovery;
pub mod error;
pub mod manager;
pub mod tool_adapter;
pub mod transport;

// Re-export primary types.
pub use auth::{McpAuthStore, McpAuthTokens};
pub use client::McpClient;
pub use error::McpError;
pub use manager::{McpConnectionManager, McpServerConfigRef, McpServerStatus};
pub use tool_adapter::McpToolAdapter;
pub use transport::{
    HttpTransport, HttpTransportBuilder, McpTransport, NotificationHandler, StdioTransport,
};
