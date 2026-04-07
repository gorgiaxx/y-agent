//! MCP transport abstraction.
//!
//! Provides the [`McpTransport`] trait and JSON-RPC protocol types.
//! Concrete implementations: [`StdioTransport`] (subprocess) and
//! [`HttpTransport`] (HTTP Streamable).

mod http;
mod stdio;

pub use http::HttpTransport;
pub use stdio::StdioTransport;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::McpError;

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

/// A JSON-RPC 2.0 notification (no `id`, no response expected).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    /// Create a new JSON-RPC request.
    pub fn new(id: u64, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl JsonRpcNotification {
    /// Create a new JSON-RPC notification.
    pub fn new(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// Transport layer for MCP communication.
///
/// Abstracts over stdio and HTTP transports.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and receive a response.
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError>;

    /// Send a notification (fire-and-forget, no response expected).
    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<(), McpError>;

    /// Close the transport connection.
    async fn close(&self) -> Result<(), McpError>;

    /// Transport type name (for logging).
    fn transport_type(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_creation() {
        let req = JsonRpcRequest::new(1, "tools/list", None);
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, 1);
        assert_eq!(req.method, "tools/list");
        assert!(req.params.is_none());
    }

    #[test]
    fn test_jsonrpc_request_with_params() {
        let params = serde_json::json!({"name": "search"});
        let req = JsonRpcRequest::new(2, "tools/call", Some(params.clone()));
        assert_eq!(req.params.unwrap(), params);
    }

    #[test]
    fn test_jsonrpc_notification_creation() {
        let notif = JsonRpcNotification::new("notifications/initialized", None);
        assert_eq!(notif.jsonrpc, "2.0");
        assert_eq!(notif.method, "notifications/initialized");
        assert!(notif.params.is_none());
    }
}
