//! Tool registry configuration.

use serde::{Deserialize, Serialize};

use crate::mcp_integration::McpServerConfig;

/// Configuration for the tool registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRegistryConfig {
    /// Maximum number of active (fully-loaded) tools per session.
    #[serde(default = "default_max_active")]
    pub max_active: usize,

    /// Maximum number of search results returned by `ToolSearch`.
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,

    /// Whether dynamic tool creation by agents is allowed.
    #[serde(default)]
    pub allow_dynamic_tools: bool,

    /// MCP server configurations loaded from `[[mcp_servers]]` in tools.toml.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

fn default_max_active() -> usize {
    20
}

fn default_search_limit() -> usize {
    10
}

impl Default for ToolRegistryConfig {
    fn default() -> Self {
        Self {
            max_active: default_max_active(),
            search_limit: default_search_limit(),
            allow_dynamic_tools: false,
            mcp_servers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ToolRegistryConfig::default();
        assert_eq!(config.max_active, 20);
        assert_eq!(config.search_limit, 10);
        assert!(!config.allow_dynamic_tools);
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_config_with_mcp_servers() {
        let toml_str = r#"
            max_active = 30
            search_limit = 5

            [[mcp_servers]]
            name = "github"
            transport = "stdio"
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-github"]
        "#;
        let config: ToolRegistryConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_active, 30);
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "github");
    }
}
