//! Tool registry configuration.

use serde::{Deserialize, Serialize};

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
    }
}
