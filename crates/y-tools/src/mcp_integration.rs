//! MCP tool integration: configuration and naming utilities.
//!
//! Design reference: tools-design.md -- MCP Tool Discovery
//!
//! Tool registration is handled by `ServiceContainer::register_mcp_tools()`
//! in `y-service`, which bridges `McpConnectionManager` discovered tools into
//! the tool registry via `McpManagedToolAdapter`.

use std::collections::HashMap;

use y_core::types::ToolName;

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
    /// Whether to automatically reconnect on unexpected disconnect.
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    /// Maximum reconnection attempts before giving up.
    #[serde(default = "default_max_reconnect_attempts")]
    pub max_reconnect_attempts: u32,
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

fn default_max_reconnect_attempts() -> u32 {
    5
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

// Tests
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
