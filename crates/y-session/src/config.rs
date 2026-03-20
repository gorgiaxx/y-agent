//! Session configuration.

use serde::{Deserialize, Serialize};

/// Configuration for the session manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Maximum tree depth for branching sessions.
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,

    /// Maximum number of active sessions per root.
    #[serde(default = "default_max_active_per_root")]
    pub max_active_per_root: usize,

    /// Compaction trigger threshold as a percentage of the serving provider's
    /// context window (e.g. 85 = compact when usage exceeds 85%).
    ///
    /// Replaces the former fixed-token `compaction_threshold` field.
    /// The actual token threshold is computed at runtime as:
    ///   `context_window * compaction_threshold_pct / 100`
    #[serde(default = "default_compaction_threshold_pct")]
    pub compaction_threshold_pct: u32,

    /// Whether to auto-archive sessions when merged.
    #[serde(default = "default_auto_archive_merged")]
    pub auto_archive_merged: bool,

    /// Number of user messages between title re-summarization (0 = disabled).
    #[serde(default = "default_title_summarize_interval")]
    pub title_summarize_interval: u32,
}

fn default_max_depth() -> u32 {
    10
}

fn default_max_active_per_root() -> usize {
    50
}

fn default_compaction_threshold_pct() -> u32 {
    85
}

fn default_auto_archive_merged() -> bool {
    true
}

fn default_title_summarize_interval() -> u32 {
    3
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_depth: default_max_depth(),
            max_active_per_root: default_max_active_per_root(),
            compaction_threshold_pct: default_compaction_threshold_pct(),
            auto_archive_merged: default_auto_archive_merged(),
            title_summarize_interval: default_title_summarize_interval(),
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
        assert_eq!(config.compaction_threshold_pct, 85);
        assert!(config.auto_archive_merged);
        assert_eq!(config.title_summarize_interval, 3);
    }
}
