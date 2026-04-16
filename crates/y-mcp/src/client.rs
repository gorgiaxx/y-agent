//! MCP client for connecting to MCP servers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tracing::{info, instrument};

use crate::error::McpError;
use crate::transport::{JsonRpcNotification, JsonRpcRequest, McpTransport, NotificationHandler};

/// Capabilities and metadata returned by the MCP `initialize` handshake.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServerCapabilities {
    /// Protocol version the server speaks.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// Server identity.
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,
    /// Feature capabilities advertised by the server.
    pub capabilities: Option<serde_json::Value>,
}

/// Server identity returned during initialization.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: Option<String>,
}

/// MCP client that communicates with an MCP server via a transport.
pub struct McpClient {
    /// The underlying transport.
    transport: Arc<dyn McpTransport>,
    /// Server name (for logging).
    server_name: String,
    /// Next request ID counter.
    next_id: AtomicU64,
    /// Whether the initialize handshake has completed.
    initialized: AtomicBool,
}

impl McpClient {
    /// Create a new MCP client with the given transport and server name.
    pub fn new(transport: Arc<dyn McpTransport>, server_name: &str) -> Self {
        Self {
            transport,
            server_name: server_name.into(),
            next_id: AtomicU64::new(1),
            initialized: AtomicBool::new(false),
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

    /// Perform the MCP initialize handshake.
    ///
    /// Must be called before `list_tools` or `call_tool`. Sends an `initialize`
    /// request followed by a `notifications/initialized` notification.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn initialize(&self) -> Result<ServerCapabilities, McpError> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "y-agent",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });

        let req = self.make_request("initialize", Some(params));
        let resp = self.transport.send(req).await?;

        if let Some(error) = resp.error {
            return Err(McpError::ServerError {
                code: error.code,
                message: error.message,
            });
        }

        let result = resp.result.ok_or_else(|| McpError::ProtocolError {
            message: "missing result in initialize response".into(),
        })?;

        let caps: ServerCapabilities = serde_json::from_value(result)?;

        // Send the initialized notification.
        let notif = JsonRpcNotification::new("notifications/initialized", None);
        self.transport.send_notification(notif).await?;

        self.initialized.store(true, Ordering::Release);

        info!(
            server = %self.server_name,
            protocol = %caps.protocol_version,
            server_info = ?caps.server_info,
            "MCP server initialized"
        );

        Ok(caps)
    }

    /// Check that the client has been initialized.
    fn require_initialized(&self) -> Result<(), McpError> {
        if !self.initialized.load(Ordering::Acquire) {
            return Err(McpError::ProtocolError {
                message: "MCP client not initialized; call initialize() first".into(),
            });
        }
        Ok(())
    }

    /// List available tools on the MCP server.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        self.require_initialized()?;

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
        self.require_initialized()?;

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

    /// Register a handler for server-sent notifications.
    ///
    /// The handler is called for every notification from the server (e.g.,
    /// `notifications/tools/list_changed`). Only stdio transport supports
    /// this; HTTP transport is stateless and ignores the handler.
    pub fn set_notification_handler(&self, handler: NotificationHandler) {
        self.transport.set_notification_handler(handler);
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
    use crate::transport::{JsonRpcNotification, JsonRpcResponse};

    use super::*;

    /// Mock transport that returns a sequence of predefined responses.
    struct MockTransport {
        responses: tokio::sync::Mutex<Vec<JsonRpcResponse>>,
        notifications: tokio::sync::Mutex<Vec<String>>,
    }

    impl MockTransport {
        fn new(responses: Vec<JsonRpcResponse>) -> Self {
            Self {
                responses: tokio::sync::Mutex::new(responses),
                notifications: tokio::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl McpTransport for MockTransport {
        async fn send(&self, _request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            let mut resps = self.responses.lock().await;
            if resps.is_empty() {
                return Err(McpError::Other {
                    message: "no more responses".into(),
                });
            }
            Ok(resps.remove(0))
        }

        async fn send_notification(
            &self,
            notification: JsonRpcNotification,
        ) -> Result<(), McpError> {
            self.notifications.lock().await.push(notification.method);
            Ok(())
        }

        async fn close(&self) -> Result<(), McpError> {
            Ok(())
        }

        fn transport_type(&self) -> &'static str {
            "mock"
        }
    }

    fn init_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "protocolVersion": "2025-03-26",
                "serverInfo": { "name": "test-server", "version": "1.0" },
                "capabilities": { "tools": {} }
            })),
            error: None,
        }
    }

    fn tools_list_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "tools": [
                    {
                        "name": "search",
                        "description": "Search the web",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"query": {"type": "string"}}
                        }
                    }
                ]
            })),
            error: None,
        }
    }

    #[tokio::test]
    async fn test_initialize_handshake() {
        let transport = Arc::new(MockTransport::new(vec![init_response(1)]));
        let transport_ref = Arc::clone(&transport);
        let client = McpClient::new(transport_ref as Arc<dyn McpTransport>, "test-server");

        let caps = client.initialize().await.unwrap();
        assert_eq!(caps.protocol_version, "2025-03-26");
        assert_eq!(caps.server_info.unwrap().name, "test-server");

        // Verify that initialized notification was sent.
        let notifs = transport.notifications.lock().await;
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0], "notifications/initialized");
    }

    #[tokio::test]
    async fn test_list_tools_before_initialize() {
        let transport = Arc::new(MockTransport::new(vec![]));
        let client = McpClient::new(transport, "test-server");
        let err = client.list_tools().await.unwrap_err();
        assert!(matches!(err, McpError::ProtocolError { .. }));
    }

    #[tokio::test]
    async fn test_call_tool_before_initialize() {
        let transport = Arc::new(MockTransport::new(vec![]));
        let client = McpClient::new(transport, "test-server");
        let err = client
            .call_tool("test", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, McpError::ProtocolError { .. }));
    }

    #[tokio::test]
    async fn test_client_list_tools() {
        let transport = Arc::new(MockTransport::new(vec![
            init_response(1),
            tools_list_response(2),
        ]));
        let client = McpClient::new(transport, "test-server");
        client.initialize().await.unwrap();

        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
    }

    #[tokio::test]
    async fn test_client_server_error() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: 2,
            result: None,
            error: Some(crate::transport::JsonRpcError {
                code: -32600,
                message: "invalid request".into(),
                data: None,
            }),
        };
        let transport = Arc::new(MockTransport::new(vec![init_response(1), resp]));
        let client = McpClient::new(transport, "test-server");
        client.initialize().await.unwrap();

        let err = client.list_tools().await.unwrap_err();
        assert!(matches!(err, McpError::ServerError { code: -32600, .. }));
    }

    #[test]
    fn test_client_metadata() {
        let transport = Arc::new(MockTransport::new(vec![]));
        let client = McpClient::new(transport, "my-server");
        assert_eq!(client.server_name(), "my-server");
        assert_eq!(client.transport_type(), "mock");
    }
}
