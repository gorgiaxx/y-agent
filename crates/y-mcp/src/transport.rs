//! MCP transport abstraction.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::McpError;

/// A JSON-RPC request for the MCP protocol.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: Option<Value>,
}

/// A JSON-RPC response from an MCP server.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
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

/// Transport layer for MCP communication.
///
/// Abstracts over stdio and HTTP transports.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and receive a response.
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError>;

    /// Close the transport connection.
    async fn close(&self) -> Result<(), McpError>;

    /// Transport type name (for logging).
    fn transport_type(&self) -> &'static str;
}

/// Placeholder stdio transport (deferred to Phase 5).
pub struct StdioTransport;

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, _request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        Err(McpError::TransportError {
            message: "stdio transport not yet implemented".into(),
        })
    }

    async fn close(&self) -> Result<(), McpError> {
        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "stdio"
    }
}

/// Placeholder HTTP/SSE transport (deferred to Phase 5).
pub struct HttpTransport;

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send(&self, _request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        Err(McpError::TransportError {
            message: "HTTP transport not yet implemented".into(),
        })
    }

    async fn close(&self) -> Result<(), McpError> {
        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "http"
    }
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

    #[tokio::test]
    async fn test_stdio_transport_not_implemented() {
        let transport = StdioTransport;
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let result = transport.send(req).await;
        assert!(matches!(result, Err(McpError::TransportError { .. })));
    }

    #[tokio::test]
    async fn test_http_transport_not_implemented() {
        let transport = HttpTransport;
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let result = transport.send(req).await;
        assert!(matches!(result, Err(McpError::TransportError { .. })));
    }
}
