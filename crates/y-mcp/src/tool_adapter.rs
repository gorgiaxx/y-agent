//! `McpToolAdapter`: wraps MCP tools as y-core `Tool`.

use std::sync::Arc;

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use crate::client::McpClient;
use crate::manager::McpConnectionManager;

/// Wraps an MCP-hosted tool as a y-core [`Tool`] implementation.
///
/// Translates y-core [`ToolInput`] to MCP `tools/call` requests and
/// MCP responses back to [`ToolOutput`].
pub struct McpToolAdapter {
    client: Arc<McpClient>,
    def: ToolDefinition,
}

impl McpToolAdapter {
    /// Create a new adapter for an MCP tool.
    pub fn new(
        client: Arc<McpClient>,
        name: &str,
        description: &str,
        schema: serde_json::Value,
    ) -> Self {
        let def = ToolDefinition {
            name: ToolName::from_string(name),
            description: description.to_string(),
            help: None,
            parameters: schema,
            result_schema: None,
            category: ToolCategory::Custom,
            tool_type: ToolType::Mcp,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        };
        Self { client, def }
    }
}

#[async_trait]
impl Tool for McpToolAdapter {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let result = self
            .client
            .call_tool(self.def.name.as_str(), input.arguments)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("MCP call failed: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: result,
            warnings: vec![],
            metadata: serde_json::json!({
                "source": "mcp",
                "server": self.client.server_name(),
            }),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

/// Wraps an MCP tool via [`McpConnectionManager`] rather than a direct client.
///
/// Reconnection and server lifecycle are handled transparently by the manager.
/// The adapter stores the original (unprefixed) tool name for the `tools/call`
/// request and the prefixed name (`mcp_{server}_{tool}`) for the registry.
pub struct McpManagedToolAdapter {
    manager: Arc<McpConnectionManager>,
    server_name: String,
    original_tool_name: String,
    def: ToolDefinition,
}

impl McpManagedToolAdapter {
    pub fn new(
        manager: Arc<McpConnectionManager>,
        server_name: &str,
        tool_name: &str,
        prefixed_name: &str,
        description: &str,
        schema: serde_json::Value,
    ) -> Self {
        let def = ToolDefinition {
            name: ToolName::from_string(prefixed_name),
            description: description.to_string(),
            help: None,
            parameters: schema,
            result_schema: None,
            category: ToolCategory::Custom,
            tool_type: ToolType::Mcp,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        };
        Self {
            manager,
            server_name: server_name.to_string(),
            original_tool_name: tool_name.to_string(),
            def,
        }
    }

    pub fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[async_trait]
impl Tool for McpManagedToolAdapter {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let result = self
            .manager
            .call_tool(&self.server_name, &self.original_tool_name, input.arguments)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("MCP call failed: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: result,
            warnings: vec![],
            metadata: serde_json::json!({
                "source": "mcp",
                "server": self.server_name,
            }),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    use crate::error::McpError;
    use crate::transport::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransport};

    use super::*;

    struct DummyTransport;

    #[async_trait::async_trait]
    impl McpTransport for DummyTransport {
        async fn send(&self, _req: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            Err(McpError::Other {
                message: "dummy".into(),
            })
        }
        async fn send_notification(&self, _n: JsonRpcNotification) -> Result<(), McpError> {
            Ok(())
        }
        async fn close(&self) -> Result<(), McpError> {
            Ok(())
        }
        fn transport_type(&self) -> &'static str {
            "dummy"
        }
    }

    #[test]
    fn test_adapter_definition() {
        let transport: Arc<dyn McpTransport> = Arc::new(DummyTransport);
        let client = Arc::new(McpClient::new(transport, "test"));
        let adapter = McpToolAdapter::new(
            client,
            "mcp_search",
            "Search via MCP",
            serde_json::json!({"type": "object"}),
        );
        let def = adapter.definition();
        assert_eq!(def.name.as_str(), "mcp_search");
        assert_eq!(def.tool_type, ToolType::Mcp);
        assert_eq!(def.category, ToolCategory::Custom);
    }
}
