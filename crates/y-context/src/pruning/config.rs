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
            max_retries: default_max_retries(),
            preserve_identifiers: true,
        }
    }
}

/// Configuration for retry pruning.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetryPruningConfig {
    /// Additional regex patterns for failure detection.
    #[serde(default)]
    pub heuristic_patterns: Vec<String>,
}

/// Configuration for intra-turn pruning.
///
/// Intra-turn pruning removes failed tool call branches from the in-memory
/// `working_history` between tool call iterations, before each LLM call.
/// Only `RetryPruning` heuristics are used (zero LLM cost).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntraTurnPruningConfig {
    /// Enable intra-turn pruning of working history.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum loop iteration before intra-turn pruning activates.
    /// Iterations below this threshold are skipped (nothing to prune early on).
    #[serde(default = "default_min_iteration")]
    pub min_iteration: u32,
    /// Minimum candidate tokens before intra-turn pruning activates.
    #[serde(default = "default_intra_turn_token_threshold")]
    pub token_threshold: u32,
}

impl Default for IntraTurnPruningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_iteration: default_min_iteration(),
            token_threshold: default_intra_turn_token_threshold(),
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
    /// Intra-turn pruning settings.
    #[serde(default)]
    pub intra_turn: IntraTurnPruningConfig,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            token_threshold: default_token_threshold(),
            strategy: PruningStrategyMode::default(),
            progressive: ProgressivePruningConfig::default(),
            retry: RetryPruningConfig::default(),
            intra_turn: IntraTurnPruningConfig::default(),
        }
    }
}

fn default_max_retries() -> u32 {
    2
}

fn default_token_threshold() -> u32 {
    2000
}

fn default_min_iteration() -> u32 {
    3
}

fn default_intra_turn_token_threshold() -> u32 {
    1000
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
        assert!(config.intra_turn.enabled);
        assert_eq!(config.intra_turn.min_iteration, 3);
        assert_eq!(config.intra_turn.token_threshold, 1000);
    }

    #[test]
    fn test_strategy_mode_serde() {
        let json = serde_json::to_string(&PruningStrategyMode::RetryOnly).unwrap();
        assert_eq!(json, "\"retry_only\"");
        let parsed: PruningStrategyMode = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(parsed, PruningStrategyMode::Auto);
    }

    #[test]
    fn test_intra_turn_config_defaults_from_empty_json() {
        let config: IntraTurnPruningConfig = serde_json::from_str("{}").unwrap();
        assert!(config.enabled);
        assert_eq!(config.min_iteration, 3);
        assert_eq!(config.token_threshold, 1000);
    }

    #[test]
    fn test_pruning_config_without_intra_turn_field() {
        // Existing configs without the intra_turn field should deserialize fine.
        let json = r#"{"enabled": true, "token_threshold": 2000}"#;
        let config: PruningConfig = serde_json::from_str(json).unwrap();
        assert!(config.intra_turn.enabled);
        assert_eq!(config.intra_turn.min_iteration, 3);
    }
}
