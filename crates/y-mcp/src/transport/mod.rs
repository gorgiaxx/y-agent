//! MCP transport abstraction.
//!
//! Provides the [`McpTransport`] trait and JSON-RPC protocol types.
//! Concrete implementations: [`StdioTransport`] (subprocess) and
//! [`HttpTransport`] (HTTP Streamable).

mod http;
mod stdio;

pub use http::{HttpTransport, HttpTransportBuilder};
pub use stdio::StdioTransport;

use std::sync::Arc;

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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A JSON-RPC 2.0 notification (no `id`, no response expected).
///
/// Used for outgoing notifications sent to the server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// An incoming JSON-RPC 2.0 notification received from the server.
///
/// Unlike [`JsonRpcResponse`], notifications have no `id` field.
/// Used internally by transports to distinguish notifications from responses.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct IncomingNotification {
    pub method: String,
    pub params: Option<Value>,
}

/// Handler for server-sent notifications.
///
/// Called by the transport when the server sends a notification (e.g.,
/// `notifications/tools/list_changed`).
pub type NotificationHandler = Arc<dyn Fn(&str, Option<Value>) + Send + Sync>;

/// Handler for server-initiated requests (e.g., `roots/list`).
///
/// Returns either a JSON result or a JSON-RPC error object.
pub type RequestHandler = Arc<
    dyn Fn(
            String,
            Option<Value>,
        ) -> futures::future::BoxFuture<'static, Result<Value, JsonRpcError>>
        + Send
        + Sync,
>;

/// Raw JSON message from the server, before type discrimination.
///
/// Used by the stdio reader to decide whether an incoming line is a response
/// (has `id`, no `method`), a notification (has `method`, no `id`), or a
/// server-initiated request (has both `id` and `method`).
#[derive(Debug, serde::Deserialize)]
struct RawJsonRpcMessage {
    id: Option<u64>,
    method: Option<String>,
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

    /// Set a handler for incoming server notifications.
    ///
    /// The default implementation is a no-op for transports that do not support
    /// server-initiated notifications (e.g., HTTP).
    fn set_notification_handler(&self, _handler: NotificationHandler) {}

    /// Set a handler for incoming server-initiated requests.
    ///
    /// The default implementation is a no-op for transports that do not support
    /// server-initiated requests (e.g., HTTP).
    fn set_request_handler(&self, _handler: RequestHandler) {}

    /// Register a channel to be notified when the transport disconnects
    /// unexpectedly (e.g., stdio child process exits).
    ///
    /// The default implementation is a no-op.
    fn set_disconnect_signal(&self, _tx: tokio::sync::mpsc::UnboundedSender<()>) {}
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
