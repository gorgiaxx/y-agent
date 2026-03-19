//! Pruning report: observability data from a pruning operation.

use serde::{Deserialize, Serialize};

/// Identifies which pruning strategy was applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PruningStrategyType {
    Retry,
    Progressive,
}

/// Result of a pruning operation, used for observability and logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruningReport {
    /// Which strategy was applied.
    pub strategy_used: PruningStrategyType,
    /// Number of messages pruned.
    pub messages_pruned: usize,
    /// Total tokens in candidate messages before pruning.
    pub tokens_before: u32,
    /// Total tokens remaining after pruning.
    pub tokens_after: u32,
    /// Tokens reclaimed.
    pub tokens_saved: u32,
    /// Whether pruning was skipped (below threshold).
    pub skipped: bool,
    /// Whether a progressive summary was inserted.
    pub summary_inserted: bool,
}

impl PruningReport {
    /// Create a report indicating pruning was skipped.
    pub fn skipped(strategy: PruningStrategyType) -> Self {
        Self {
            strategy_used: strategy,
            messages_pruned: 0,
            tokens_before: 0,
            tokens_after: 0,
            tokens_saved: 0,
            skipped: true,
            summary_inserted: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skipped_report() {
        let report = PruningReport::skipped(PruningStrategyType::Retry);
        assert!(report.skipped);
        assert_eq!(report.messages_pruned, 0);
        assert_eq!(report.tokens_saved, 0);
    }

    #[test]
    fn test_report_serde() {
        let report = PruningReport {
            strategy_used: PruningStrategyType::Progressive,
            messages_pruned: 5,
            tokens_before: 3000,
            tokens_after: 1200,
            tokens_saved: 1800,
            skipped: false,
            summary_inserted: true,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"progressive\""));
        assert!(json.contains("\"tokens_saved\":1800"));
    }
}
