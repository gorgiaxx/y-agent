//! MCP server/tool discovery.

use std::sync::Arc;

use crate::client::{McpClient, McpToolInfo};
use crate::error::McpError;

/// Discovered tools from an MCP server.
#[derive(Debug, Clone)]
pub struct DiscoveredServer {
    /// Server name.
    pub name: String,
    /// Transport type used.
    pub transport_type: String,
    /// Tools available on this server.
    pub tools: Vec<McpToolInfo>,
}

/// Discovers tools available on an MCP server.
///
/// Connects to the server, calls `tools/list`, and returns
/// the discovered tools.
pub async fn discover_tools(client: &McpClient) -> Result<DiscoveredServer, McpError> {
    let tools = client.list_tools().await?;
    Ok(DiscoveredServer {
        name: client.server_name().to_string(),
        transport_type: client.transport_type().to_string(),
        tools,
    })
}

/// Register all discovered MCP tools with the given callback.
///
/// This is a convenience function that discovers tools and calls
/// the provided closure for each tool definition.
pub async fn register_discovered_tools<F>(
    client: Arc<McpClient>,
    mut register_fn: F,
) -> Result<usize, McpError>
where
    F: FnMut(crate::tool_adapter::McpToolAdapter),
{
    let tools = client.list_tools().await?;
    let count = tools.len();

    for tool in tools {
        let adapter = crate::tool_adapter::McpToolAdapter::new(
            client.clone(),
            &tool.name,
            tool.description.as_deref().unwrap_or(""),
            tool.input_schema.unwrap_or(serde_json::json!({})),
        );
        register_fn(adapter);
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discovered_server_structure() {
        let server = DiscoveredServer {
            name: "test-server".into(),
            transport_type: "stdio".into(),
            tools: vec![McpToolInfo {
                name: "search".into(),
                description: Some("Search tool".into()),
                input_schema: None,
            }],
        };
        assert_eq!(server.name, "test-server");
        assert_eq!(server.tools.len(), 1);
    }
}
