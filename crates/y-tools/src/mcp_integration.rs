//! MCP tool integration: discovers and registers MCP tools at startup.
//!
//! Design reference: tools-design.md §MCP Tool Discovery
//!
//! At startup, the tool registry queries configured MCP servers via `tools/list`,
//! adapts each discovered tool using `McpToolAdapter`, and registers them with
//! the naming convention `mcp_{server}_{tool}`.

use std::sync::Arc;

use tracing::{info, warn};

use y_core::tool::Tool;
use y_core::types::ToolName;
use y_mcp::client::McpClient;
use y_mcp::tool_adapter::McpToolAdapter;

use crate::registry::ToolRegistryImpl;

/// Configuration for a single MCP server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    /// Unique server name (used in tool name prefix).
    pub name: String,
    /// Transport type: "stdio" or "http".
    pub transport: String,
    /// Command to execute (for stdio transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// URL for HTTP/SSE transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Result of MCP tool discovery for a single server.
#[derive(Debug)]
pub struct McpDiscoveryResult {
    /// Server name.
    pub server_name: String,
    /// Number of tools discovered from this server.
    pub tools_discovered: usize,
    /// Number of tools successfully registered.
    pub tools_registered: usize,
    /// Errors encountered during discovery.
    pub errors: Vec<String>,
}

/// Discover and register MCP tools from all configured servers.
///
/// Each tool is registered with the naming convention `mcp_{server}_{tool}`.
/// Tools that fail to register (e.g., name conflicts) are logged and skipped.
pub async fn discover_and_register(
    registry: &ToolRegistryImpl,
    servers: &[McpServerConfig],
) -> Vec<McpDiscoveryResult> {
    let mut results = Vec::new();

    for server_config in servers {
        if !server_config.enabled {
            info!(server = %server_config.name, "MCP server disabled, skipping");
            continue;
        }

        let result = discover_server(registry, server_config).await;
        results.push(result);
    }

    let total_registered: usize = results.iter().map(|r| r.tools_registered).sum();
    info!(
        servers = servers.len(),
        total_tools_registered = total_registered,
        "MCP tool discovery complete"
    );

    results
}

/// Discover and register tools from a single MCP server.
async fn discover_server(
    registry: &ToolRegistryImpl,
    config: &McpServerConfig,
) -> McpDiscoveryResult {
    let mut result = McpDiscoveryResult {
        server_name: config.name.clone(),
        tools_discovered: 0,
        tools_registered: 0,
        errors: Vec::new(),
    };

    // Build the MCP client with appropriate transport.
    let transport: Arc<dyn y_mcp::transport::McpTransport> = match config.transport.as_str() {
        "stdio" => Arc::new(y_mcp::transport::StdioTransport),
        "http" => Arc::new(y_mcp::transport::HttpTransport::new(
            config.url.as_deref().unwrap_or("http://localhost:3000"),
        )),
        other => {
            result
                .errors
                .push(format!("unsupported transport: {other}"));
            return result;
        }
    };

    let client = Arc::new(McpClient::new(transport, &config.name));

    // Discover tools via tools/list.
    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(e) => {
            let msg = format!("failed to discover tools from {}: {e}", config.name);
            warn!("{msg}");
            result.errors.push(msg);
            return result;
        }
    };

    result.tools_discovered = tools.len();

    // Register each discovered tool with the naming convention.
    for tool_info in tools {
        let prefixed_name = format!("mcp_{}_{}", config.name, tool_info.name);
        let adapter = McpToolAdapter::new(
            Arc::clone(&client),
            &prefixed_name,
            tool_info.description.as_deref().unwrap_or(""),
            tool_info.input_schema.unwrap_or(serde_json::json!({})),
        );

        let def = adapter.definition().clone();
        match registry.register_tool(Arc::new(adapter), def).await {
            Ok(()) => {
                info!(
                    tool = %prefixed_name,
                    server = %config.name,
                    "registered MCP tool"
                );
                result.tools_registered += 1;
            }
            Err(e) => {
                let msg = format!("failed to register MCP tool {prefixed_name}: {e}");
                warn!("{msg}");
                result.errors.push(msg);
            }
        }
    }

    result
}

/// Generate the canonical MCP tool name from server and tool names.
pub fn mcp_tool_name(server_name: &str, tool_name: &str) -> ToolName {
    ToolName::from_string(format!("mcp_{server_name}_{tool_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_name_convention() {
        let name = mcp_tool_name("github", "search_repos");
        assert_eq!(name.as_str(), "mcp_github_search_repos");
    }

    #[test]
    fn test_server_config_deserialize() {
        let toml_str = r#"
            name = "filesystem"
            transport = "stdio"
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
            enabled = true
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "filesystem");
        assert_eq!(config.transport, "stdio");
        assert_eq!(config.command, Some("npx".into()));
        assert!(config.enabled);
    }

    #[test]
    fn test_server_config_default_enabled() {
        let toml_str = r#"
            name = "test"
            transport = "http"
            url = "http://localhost:3000"
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert!(config.enabled);
    }

    #[test]
    fn test_discovery_result_structure() {
        let result = McpDiscoveryResult {
            server_name: "test".into(),
            tools_discovered: 5,
            tools_registered: 4,
            errors: vec!["one error".into()],
        };
        assert_eq!(result.tools_discovered, 5);
        assert_eq!(result.tools_registered, 4);
        assert_eq!(result.errors.len(), 1);
    }
}
