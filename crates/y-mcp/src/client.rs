//! MCP client for connecting to MCP servers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures::FutureExt;
use tokio::sync::Mutex;
use tracing::{info, instrument};

use crate::error::McpError;
use crate::transport::{
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, McpTransport, NotificationHandler,
    RequestHandler,
};

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

impl ServerCapabilities {
    /// Whether the server advertises support for resources.
    pub fn supports_resources(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|v| v.get("resources"))
            .is_some()
    }

    /// Whether the server advertises support for prompts.
    pub fn supports_prompts(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|v| v.get("prompts"))
            .is_some()
    }

    /// Whether the server advertises support for tools.
    pub fn supports_tools(&self) -> bool {
        self.capabilities
            .as_ref()
            .and_then(|v| v.get("tools"))
            .is_some()
    }
}

/// Server identity returned during initialization.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: Option<String>,
}

/// A filesystem or URI root exposed by the client to the server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpRoot {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// Filesystem/URI roots advertised to the server.
    roots: Arc<Mutex<Vec<McpRoot>>>,
}

impl McpClient {
    /// Create a new MCP client with the given transport and server name.
    pub fn new(transport: Arc<dyn McpTransport>, server_name: &str) -> Self {
        Self {
            transport,
            server_name: server_name.into(),
            next_id: AtomicU64::new(1),
            initialized: AtomicBool::new(false),
            roots: Arc::new(Mutex::new(Vec::new())),
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

    /// Set the filesystem/URI roots advertised to the server.
    ///
    /// Must be called before [`McpClient::initialize`] for the `roots` capability to be
    /// declared. Also installs a request handler for `roots/list` on the
    /// transport.
    pub async fn set_roots(self: &Arc<Self>, roots: Vec<McpRoot>) {
        *self.roots.lock().await = roots;

        let roots_handle = Arc::clone(&self.roots);
        let handler: RequestHandler = Arc::new(move |method, _params| {
            let roots_handle = Arc::clone(&roots_handle);
            async move {
                if method == "roots/list" {
                    let roots = roots_handle.lock().await.clone();
                    Ok(serde_json::json!({ "roots": roots }))
                } else {
                    Err(JsonRpcError {
                        code: -32601,
                        message: format!("method not found: {method}"),
                        data: None,
                    })
                }
            }
            .boxed()
        });
        self.transport.set_request_handler(handler);
    }

    /// Notify the server that the roots list has changed.
    pub async fn notify_roots_changed(&self) -> Result<(), McpError> {
        self.transport
            .send_notification(JsonRpcNotification::new(
                "notifications/roots/list_changed",
                None,
            ))
            .await
    }

    /// Perform the MCP initialize handshake.
    ///
    /// Must be called before `list_tools`, `list_resources`, etc. Sends an
    /// `initialize` request followed by a `notifications/initialized`
    /// notification.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn initialize(&self) -> Result<ServerCapabilities, McpError> {
        let has_roots = !self.roots.lock().await.is_empty();
        let mut client_caps = serde_json::Map::new();
        if has_roots {
            client_caps.insert("roots".into(), serde_json::json!({ "listChanged": true }));
        }

        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": serde_json::Value::Object(client_caps),
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

    /// Generic `method` RPC call returning the parsed `result` field.
    async fn call_method<T>(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<T, McpError>
    where
        T: serde::de::DeserializeOwned,
    {
        self.require_initialized()?;
        let req = self.make_request(method, params);
        let resp = self.transport.send(req).await?;

        if let Some(error) = resp.error {
            return Err(McpError::ServerError {
                code: error.code,
                message: error.message,
            });
        }

        let result = resp.result.ok_or_else(|| McpError::ProtocolError {
            message: format!("missing result in {method} response"),
        })?;
        Ok(serde_json::from_value(result)?)
    }

    /// List available tools on the MCP server.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let result: ToolListResult = self.call_method("tools/list", None).await?;
        Ok(result.tools)
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
        self.require_initialized()?;
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

    /// List resources exposed by the MCP server.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn list_resources(&self) -> Result<Vec<McpResource>, McpError> {
        let result: ResourceListResult = self.call_method("resources/list", None).await?;
        Ok(result.resources)
    }

    /// Read the contents of a resource by URI.
    #[instrument(skip(self), fields(server = %self.server_name, %uri))]
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        let params = serde_json::json!({ "uri": uri });
        self.call_method("resources/read", Some(params)).await
    }

    /// List prompts exposed by the MCP server.
    #[instrument(skip(self), fields(server = %self.server_name))]
    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>, McpError> {
        let result: PromptListResult = self.call_method("prompts/list", None).await?;
        Ok(result.prompts)
    }

    /// Retrieve a prompt by name, optionally substituting arguments.
    #[instrument(skip(self, arguments), fields(server = %self.server_name, prompt = %name))]
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: HashMap<String, String>,
    ) -> Result<GetPromptResult, McpError> {
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        self.call_method("prompts/get", Some(params)).await
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

    /// Register a channel to receive unexpected-disconnect signals.
    pub fn set_disconnect_signal(&self, tx: tokio::sync::mpsc::UnboundedSender<()>) {
        self.transport.set_disconnect_signal(tx);
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

/// Resource metadata returned by `resources/list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Contents of a resource returned by `resources/read`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// Result of `resources/read`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContents>,
}

/// Prompt metadata returned by `prompts/list`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// One named argument accepted by an MCP prompt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// Result of `prompts/get`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetPromptResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

/// A single prompt message.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: serde_json::Value,
}

/// Response wrapper for `tools/list`.
#[derive(Debug, serde::Deserialize)]
struct ToolListResult {
    tools: Vec<McpToolInfo>,
}

/// Response wrapper for `resources/list`.
#[derive(Debug, serde::Deserialize)]
struct ResourceListResult {
    resources: Vec<McpResource>,
}

/// Response wrapper for `prompts/list`.
#[derive(Debug, serde::Deserialize)]
struct PromptListResult {
    prompts: Vec<McpPrompt>,
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

    fn resources_list_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "resources": [
                    {
                        "uri": "file:///tmp/a.txt",
                        "name": "a.txt",
                        "mimeType": "text/plain"
                    }
                ]
            })),
            error: None,
        }
    }

    fn read_resource_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "contents": [
                    { "uri": "file:///tmp/a.txt", "mimeType": "text/plain", "text": "hello" }
                ]
            })),
            error: None,
        }
    }

    fn prompts_list_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "prompts": [
                    {
                        "name": "summarize",
                        "description": "Summarize text",
                        "arguments": [
                            { "name": "topic", "required": true }
                        ]
                    }
                ]
            })),
            error: None,
        }
    }

    fn get_prompt_response(id: u64) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::json!({
                "description": "A greeting",
                "messages": [
                    { "role": "user", "content": "hi" }
                ]
            })),
            error: None,
        }
    }

    #[tokio::test]
    async fn test_list_and_read_resources() {
        let transport = Arc::new(MockTransport::new(vec![
            init_response(1),
            resources_list_response(2),
            read_resource_response(3),
        ]));
        let client = McpClient::new(transport, "test-server");
        client.initialize().await.unwrap();

        let resources = client.list_resources().await.unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].uri, "file:///tmp/a.txt");
        assert_eq!(resources[0].mime_type.as_deref(), Some("text/plain"));

        let read = client.read_resource("file:///tmp/a.txt").await.unwrap();
        assert_eq!(read.contents.len(), 1);
        assert_eq!(read.contents[0].text.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn test_list_and_get_prompt() {
        let transport = Arc::new(MockTransport::new(vec![
            init_response(1),
            prompts_list_response(2),
            get_prompt_response(3),
        ]));
        let client = McpClient::new(transport, "test-server");
        client.initialize().await.unwrap();

        let prompts = client.list_prompts().await.unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "summarize");

        let mut args = HashMap::new();
        args.insert("topic".into(), "rust".into());
        let got = client.get_prompt("summarize", args).await.unwrap();
        assert_eq!(got.messages.len(), 1);
        assert_eq!(got.messages[0].role, "user");
    }

    #[tokio::test]
    async fn test_list_resources_before_initialize() {
        let transport = Arc::new(MockTransport::new(vec![]));
        let client = McpClient::new(transport, "test-server");
        let err = client.list_resources().await.unwrap_err();
        assert!(matches!(err, McpError::ProtocolError { .. }));
    }

    #[test]
    fn test_server_capabilities_helpers() {
        let caps = ServerCapabilities {
            protocol_version: "2025-03-26".into(),
            server_info: None,
            capabilities: Some(serde_json::json!({
                "tools": {},
                "resources": { "listChanged": true }
            })),
        };
        assert!(caps.supports_tools());
        assert!(caps.supports_resources());
        assert!(!caps.supports_prompts());
    }
}
