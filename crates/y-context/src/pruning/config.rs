//! Pruning configuration types.

use serde::{Deserialize, Serialize};

/// Strategy selection mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PruningStrategyMode {
    /// Only retry pruning (zero LLM cost).
    RetryOnly,
    /// Only progressive pruning (LLM summarization).
    ProgressiveOnly,
    /// Both strategies: retry first, then progressive.
    #[default]
    Auto,
}

/// Configuration for progressive pruning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressivePruningConfig {
    /// LLM model for progressive summaries.
    #[serde(default = "default_progressive_model")]
    pub model: String,
    /// Maximum retry attempts for progressive LLM calls.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Apply identifier preservation policy to summaries.
    #[serde(default = "default_true")]
    pub preserve_identifiers: bool,
}

impl Default for ProgressivePruningConfig {
    fn default() -> Self {
        Self {
            model: default_progressive_model(),
            max_retries: default_max_retries(),
            preserve_identifiers: true,
        }
    }
}

/// Configuration for retry pruning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPruningConfig {
    /// Additional regex patterns for failure detection.
    #[serde(default)]
    pub heuristic_patterns: Vec<String>,
}

impl Default for RetryPruningConfig {
    fn default() -> Self {
        Self {
            heuristic_patterns: Vec::new(),
        }
    }
}

/// Top-level pruning configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningConfig {
    /// Master switch for pruning.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum cumulative tokens in a branch before pruning activates.
    #[serde(default = "default_token_threshold")]
    pub token_threshold: u32,
    /// Strategy selection mode.
    #[serde(default)]
    pub strategy: PruningStrategyMode,
    /// Progressive pruning settings.
    #[serde(default)]
    pub progressive: ProgressivePruningConfig,
    /// Retry pruning settings.
    #[serde(default)]
    pub retry: RetryPruningConfig,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            token_threshold: default_token_threshold(),
            strategy: PruningStrategyMode::default(),
            progressive: ProgressivePruningConfig::default(),
            retry: RetryPruningConfig::default(),
        }
    }
}

fn default_progressive_model() -> String {
    "gpt-4o-mini".into()
}

fn default_max_retries() -> u32 {
    2
}

fn default_token_threshold() -> u32 {
    2000
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PruningConfig::default();
        assert!(config.enabled);
        assert_eq!(config.token_threshold, 2000);
        assert_eq!(config.strategy, PruningStrategyMode::Auto);
        assert_eq!(config.progressive.max_retries, 2);
        assert!(config.progressive.preserve_identifiers);
    }

    #[test]
    fn test_strategy_mode_serde() {
        let json = serde_json::to_string(&PruningStrategyMode::RetryOnly).unwrap();
        assert_eq!(json, "\"retry_only\"");
        let parsed: PruningStrategyMode = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(parsed, PruningStrategyMode::Auto);
    }
}
