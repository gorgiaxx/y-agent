//! `McpToolAdapter`: wraps MCP tools as y-core `Tool`.

use std::sync::Arc;

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use crate::client::McpClient;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_definition() {
        let transport = Arc::new(crate::transport::StdioTransport);
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
