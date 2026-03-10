//! MCP-specific error types.

use thiserror::Error;

/// Errors from the MCP protocol layer.
#[derive(Debug, Error)]
pub enum McpError {
    /// Connection to MCP server failed.
    #[error("connection failed: {message}")]
    ConnectionFailed { message: String },

    /// The MCP server returned an error response.
    #[error("server error ({code}): {message}")]
    ServerError { code: i32, message: String },

    /// JSON-RPC protocol error.
    #[error("protocol error: {message}")]
    ProtocolError { message: String },

    /// Timeout waiting for response.
    #[error("timeout: {message}")]
    Timeout { message: String },

    /// Transport-level error.
    #[error("transport error: {message}")]
    TransportError { message: String },

    /// Tool not found on MCP server.
    #[error("tool not found: {name}")]
    ToolNotFound { name: String },

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Generic error.
    #[error("{message}")]
    Other { message: String },
}

impl From<McpError> for y_core::tool::ToolError {
    fn from(e: McpError) -> Self {
        match e {
            McpError::ToolNotFound { name } => y_core::tool::ToolError::NotFound { name },
            other => y_core::tool::ToolError::Other {
                message: other.to_string(),
            },
        }
    }
}
