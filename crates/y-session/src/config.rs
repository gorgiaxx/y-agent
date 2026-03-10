//! Session configuration.

use serde::Deserialize;

/// Configuration for the session manager.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    /// Maximum tree depth for branching sessions.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,

    /// Maximum number of active sessions per root.
    #[serde(default = "default_max_active_per_root")]
    pub max_active_per_root: usize,

    /// Token count threshold to trigger compaction hint.
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: u32,

    /// Whether to auto-archive sessions when merged.
    #[serde(default = "default_auto_archive_merged")]
    pub auto_archive_merged: bool,
}

fn default_max_depth() -> u32 {
    10
}

fn default_max_active_per_root() -> usize {
    50
}

fn default_compaction_threshold() -> u32 {
    100_000
}

fn default_auto_archive_merged() -> bool {
    true
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_depth: default_max_depth(),
            max_active_per_root: default_max_active_per_root(),
            compaction_threshold: default_compaction_threshold(),
            auto_archive_merged: default_auto_archive_merged(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = SessionConfig::default();
        assert_eq!(config.max_depth, 10);
        assert_eq!(config.max_active_per_root, 50);
        assert_eq!(config.compaction_threshold, 100_000);
        assert!(config.auto_archive_merged);
    }
}
