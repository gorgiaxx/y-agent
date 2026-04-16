//! MCP tool integration: discovers and registers MCP tools at startup.
//!
//! Design reference: tools-design.md -- MCP Tool Discovery
//!
//! At startup, the tool registry queries configured MCP servers via `tools/list`,
//! adapts each discovered tool using `McpToolAdapter`, and registers them with
//! the naming convention `mcp_{server}_{tool}`.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::{debug, info, warn};

use y_core::tool::Tool;
use y_core::types::ToolName;
use y_mcp::client::McpClient;
use y_mcp::tool_adapter::McpToolAdapter;
use y_mcp::transport::{HttpTransport, StdioTransport};

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
    /// Environment variables for the subprocess (stdio transport).
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Custom HTTP headers sent with every request (HTTP transport).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Timeout (seconds) for the initial connection / initialize handshake.
    #[serde(default = "default_startup_timeout")]
    pub startup_timeout_secs: u64,
    /// Timeout (seconds) for individual tool calls.
    #[serde(default = "default_tool_timeout")]
    pub tool_timeout_secs: u64,
    /// Working directory for the subprocess (stdio transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Explicit bearer token for authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
    /// Whitelist of tool names to expose (None = all tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,
    /// Blacklist of tool names to hide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}

fn default_startup_timeout() -> u64 {
    30
}

fn default_tool_timeout() -> u64 {
    120
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
        "stdio" => {
            let Some(command) = config.command.as_deref() else {
                result
                    .errors
                    .push("stdio transport requires a 'command' field".into());
                return result;
            };
            match StdioTransport::spawn(command, &config.args, &config.env, config.cwd.as_deref()) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    let msg = format!("failed to spawn MCP server '{}': {e}", config.name);
                    warn!("{msg}");
                    result.errors.push(msg);
                    return result;
                }
            }
        }
        "http" => {
            let url = config.url.as_deref().unwrap_or("http://localhost:3000");
            let mut builder = HttpTransport::builder(url)
                .server_name(&config.name)
                .headers(config.headers.clone())
                .timeout(std::time::Duration::from_secs(config.tool_timeout_secs));

            // Resolve bearer token: explicit config -> env var -> auth store.
            let token = y_mcp::auth::resolve_bearer_token(
                &config.name,
                config.bearer_token.as_deref(),
                None, // auth store injected later via manager
            );
            if let Some(t) = token {
                builder = builder.bearer_token(t);
            }

            match builder.build() {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    let msg = format!("failed to create HTTP transport for '{}': {e}", config.name);
                    warn!("{msg}");
                    result.errors.push(msg);
                    return result;
                }
            }
        }
        other => {
            result
                .errors
                .push(format!("unsupported transport: {other}"));
            return result;
        }
    };

    let client = Arc::new(McpClient::new(transport, &config.name));

    // Perform the MCP initialize handshake.
    if let Err(e) = client.initialize().await {
        let msg = format!("failed to initialize MCP server '{}': {e}", config.name);
        warn!("{msg}");
        result.errors.push(msg);
        return result;
    }

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
        // Apply tool filtering (enabled_tools whitelist / disabled_tools blacklist).
        if let Some(ref whitelist) = config.enabled_tools {
            if !whitelist.contains(&tool_info.name) {
                debug!(
                    tool = %tool_info.name,
                    server = %config.name,
                    "skipping tool not in enabled_tools whitelist"
                );
                continue;
            }
        }
        if let Some(ref blacklist) = config.disabled_tools {
            if blacklist.contains(&tool_info.name) {
                debug!(
                    tool = %tool_info.name,
                    server = %config.name,
                    "skipping tool in disabled_tools blacklist"
                );
                continue;
            }
        }

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

/// Split a qualified MCP tool name into `(server_name, tool_name)`.
///
/// Qualified names follow the convention `mcp_{server}_{tool}`. The server
/// name is extracted as the first segment after the `mcp_` prefix, and the
/// remaining segments form the tool name.
///
/// Returns `None` if the name does not start with `mcp_` or has fewer than
/// three segments.
pub fn split_qualified_tool_name(qualified: &str) -> Option<(&str, &str)> {
    let rest = qualified.strip_prefix("mcp_")?;
    let underscore_pos = rest.find('_')?;
    let server = &rest[..underscore_pos];
    let tool = &rest[underscore_pos + 1..];
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
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
    fn test_split_qualified_tool_name() {
        assert_eq!(
            split_qualified_tool_name("mcp_github_search_repos"),
            Some(("github", "search_repos"))
        );
        assert_eq!(
            split_qualified_tool_name("mcp_fs_read_file"),
            Some(("fs", "read_file"))
        );
    }

    #[test]
    fn test_split_qualified_tool_name_roundtrip() {
        let name = mcp_tool_name("myserver", "do_thing");
        let (server, tool) = split_qualified_tool_name(name.as_str()).unwrap();
        assert_eq!(server, "myserver");
        assert_eq!(tool, "do_thing");
    }

    #[test]
    fn test_split_qualified_tool_name_invalid() {
        assert!(split_qualified_tool_name("not_mcp_tool").is_none());
        assert!(split_qualified_tool_name("mcp_").is_none());
        assert!(split_qualified_tool_name("mcp_server").is_none());
        assert!(split_qualified_tool_name("mcp__tool").is_none());
        assert!(split_qualified_tool_name("random").is_none());
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
        assert!(config.env.is_empty());
        // New fields default correctly.
        assert_eq!(config.startup_timeout_secs, 30);
        assert_eq!(config.tool_timeout_secs, 120);
        assert!(config.headers.is_empty());
        assert!(config.cwd.is_none());
        assert!(config.bearer_token.is_none());
        assert!(config.enabled_tools.is_none());
        assert!(config.disabled_tools.is_none());
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
    fn test_server_config_with_env() {
        let toml_str = r#"
            name = "github"
            transport = "stdio"
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-github"]

            [env]
            GITHUB_TOKEN = "ghp_test123"
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.env.get("GITHUB_TOKEN").unwrap(), "ghp_test123");
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

    #[test]
    fn test_server_config_extended_fields() {
        let toml_str = r#"
            name = "remote"
            transport = "http"
            url = "https://mcp.example.com"
            bearer_token = "my-token"
            startup_timeout_secs = 15
            tool_timeout_secs = 60
            enabled_tools = ["search", "query"]
            disabled_tools = ["delete"]
            cwd = "/workspace"

            [headers]
            X-Api-Key = "key123"
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "remote");
        assert_eq!(config.bearer_token.as_deref(), Some("my-token"));
        assert_eq!(config.startup_timeout_secs, 15);
        assert_eq!(config.tool_timeout_secs, 60);
        assert_eq!(config.cwd.as_deref(), Some("/workspace"));
        assert_eq!(
            config.enabled_tools.as_deref(),
            Some(&["search".to_string(), "query".to_string()][..])
        );
        assert_eq!(
            config.disabled_tools.as_deref(),
            Some(&["delete".to_string()][..])
        );
        assert_eq!(config.headers.get("X-Api-Key").unwrap(), "key123");
    }
}
