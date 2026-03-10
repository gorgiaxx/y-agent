//! MCP client for connecting to MCP servers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tracing::instrument;

use crate::error::McpError;
use crate::transport::{JsonRpcRequest, McpTransport};

/// MCP client that communicates with an MCP server via a transport.
pub struct McpClient {
    /// The underlying transport.
    transport: Arc<dyn McpTransport>,
    /// Server name (for logging).
    server_name: String,
    /// Next request ID counter.
    next_id: AtomicU64,
}

impl McpClient {
    /// Create a new MCP client with the given transport and server name.
    pub fn new(transport: Arc<dyn McpTransport>, server_name: &str) -> Self {
        Self {
            transport,
            server_name: server_name.into(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Get the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Get the transport type.
    pub fn transport_type(&self) -> &'static str {
        self.transport.transport_type()
    }

    /// List available tools on the MCP server.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let req = self.make_request("tools/list", None);
        let resp = self.transport.send(req).await?;

        if let Some(error) = resp.error {
            return Err(McpError::ServerError {
                code: error.code,
                message: error.message,
            });
        }

        let result = resp.result.ok_or_else(|| McpError::ProtocolError {
            message: "missing result in response".into(),
        })?;

        let tools: ToolListResult = serde_json::from_value(result)?;
        Ok(tools.tools)
    }

    /// Call a tool on the MCP server.
    #[instrument(skip(self, arguments), fields(server = %self.server_name, tool = %tool_name))]
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments,
        });
        let req = self.make_request("tools/call", Some(params));
        let resp = self.transport.send(req).await?;

        if let Some(error) = resp.error {
            return Err(McpError::ServerError {
                code: error.code,
                message: error.message,
            });
        }

        resp.result.ok_or_else(|| McpError::ProtocolError {
            message: "missing result in response".into(),
        })
    }

    /// Close the connection to the MCP server.
    pub async fn close(&self) -> Result<(), McpError> {
        self.transport.close().await
    }

    fn make_request(&self, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        JsonRpcRequest::new(id, method, params)
    }
}

/// Tool information returned by `tools/list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

/// Response wrapper for `tools/list`.
#[derive(Debug, serde::Deserialize)]
struct ToolListResult {
    tools: Vec<McpToolInfo>,
}

#[cfg(test)]
mod tests {
    use crate::transport::JsonRpcResponse;

    use super::*;

    /// Mock transport that returns predefined responses.
    struct MockTransport {
        response: tokio::sync::Mutex<Option<JsonRpcResponse>>,
    }

    impl MockTransport {
        fn new(response: JsonRpcResponse) -> Self {
            Self {
                response: tokio::sync::Mutex::new(Some(response)),
            }
        }
    }

    #[async_trait::async_trait]
    impl McpTransport for MockTransport {
        async fn send(&self, _request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            let resp = self.response.lock().await.take().ok_or(McpError::Other {
                message: "no more responses".into(),
            })?;
            Ok(resp)
        }

        async fn close(&self) -> Result<(), McpError> {
            Ok(())
        }

        fn transport_type(&self) -> &'static str {
            "mock"
        }
    }

    fn tools_list_response() -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 1,
            result: Some(serde_json::json!({
                "tools": [
                    {
                        "name": "search",
                        "description": "Search the web",
                        "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}
                    }
                ]
            })),
            error: None,
        }
    }

    #[tokio::test]
    async fn test_client_list_tools() {
        let transport = Arc::new(MockTransport::new(tools_list_response()));
        let client = McpClient::new(transport, "test-server");
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
    }

    #[tokio::test]
    async fn test_client_server_error() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 1,
            result: None,
            error: Some(crate::transport::JsonRpcError {
                code: -32600,
                message: "invalid request".into(),
                data: None,
            }),
        };
        let transport = Arc::new(MockTransport::new(resp));
        let client = McpClient::new(transport, "test-server");
        let err = client.list_tools().await.unwrap_err();
        assert!(matches!(err, McpError::ServerError { code: -32600, .. }));
    }

    #[test]
    fn test_client_metadata() {
        let transport = Arc::new(MockTransport::new(tools_list_response()));
        let client = McpClient::new(transport, "my-server");
        assert_eq!(client.server_name(), "my-server");
        assert_eq!(client.transport_type(), "mock");
    }
}
