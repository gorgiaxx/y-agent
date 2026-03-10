//! Scheduler configuration.

use serde::{Deserialize, Serialize};

/// Configuration for the scheduler module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Maximum number of concurrent schedule executions.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_executions: usize,
    /// Default missed-schedule policy.
    #[serde(default)]
    pub default_missed_policy: MissedPolicy,
    /// Default concurrency policy.
    #[serde(default)]
    pub default_concurrency_policy: ConcurrencyPolicy,
    /// Maximum execution history entries to retain per schedule.
    #[serde(default = "default_history_limit")]
    pub history_retention_limit: usize,
}

/// Policy for handling missed schedules (e.g., during downtime).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MissedPolicy {
    /// Execute once immediately to catch up.
    CatchUp,
    /// Skip missed executions, resume from next.
    #[default]
    Skip,
    /// Execute all missed occurrences in sequence.
    Backfill,
}

/// Policy for concurrent execution of the same schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyPolicy {
    /// Allow parallel executions.
    Allow,
    /// Skip trigger if previous is still running.
    #[default]
    SkipIfRunning,
    /// Queue trigger for later execution.
    Queue,
    /// Cancel previous execution and start new.
    CancelPrevious,
}

fn default_max_concurrent() -> usize {
    10
}

fn default_history_limit() -> usize {
    100
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_executions: default_max_concurrent(),
            default_missed_policy: MissedPolicy::default(),
            default_concurrency_policy: ConcurrencyPolicy::default(),
            history_retention_limit: default_history_limit(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SchedulerConfig::default();
        assert_eq!(config.max_concurrent_executions, 10);
        assert_eq!(config.default_missed_policy, MissedPolicy::Skip);
        assert_eq!(
            config.default_concurrency_policy,
            ConcurrencyPolicy::SkipIfRunning
        );
    }
}
