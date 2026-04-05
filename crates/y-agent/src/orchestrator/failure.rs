//! Failure strategies and retry configuration for workflow tasks.
//!
//! Design reference: orchestrator-design.md, Failure Handling and Edge Cases

use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Failure strategy
// ---------------------------------------------------------------------------

/// Strategy to apply when a task fails after exhausting retries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureStrategy {
    /// Abort the entire workflow immediately.
    #[default]
    FailFast,
    /// Mark the task as failed but continue executing unblocked branches.
    ContinueOnError,
    /// Re-execute with backoff (delegates to `RetryConfig`).
    Retry,
    /// Execute compensation tasks in reverse dependency order.
    Rollback,
    /// Mark the failed task as succeeded and continue.
    Ignore,
    /// Execute a specific compensating task.
    Compensation {
        /// Task ID of the compensation handler.
        compensation_task_id: String,
    },
}

// ---------------------------------------------------------------------------
// Backoff strategy
// ---------------------------------------------------------------------------

/// How to increase delay between retry attempts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    /// Fixed delay between attempts.
    Fixed,
    /// Delay increases linearly: base * attempt.
    Linear,
    /// Delay doubles each attempt: base * 2^(attempt-1).
    #[default]
    Exponential,
}

// ---------------------------------------------------------------------------
// Retry configuration
// ---------------------------------------------------------------------------

/// Retry policy for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of attempts (including the first try).
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    /// Base delay between retries in milliseconds.
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    /// Backoff strategy.
    #[serde(default)]
    pub backoff: BackoffStrategy,
}

fn default_max_attempts() -> u32 {
    3
}

fn default_delay_ms() -> u64 {
    500
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            delay_ms: default_delay_ms(),
            backoff: BackoffStrategy::default(),
        }
    }
}

impl RetryConfig {
    /// Compute the delay for a given attempt number (1-indexed).
    ///
    /// - `Fixed`: always returns `self.delay_ms`
    /// - `Linear`: `self.delay_ms * attempt`
    /// - `Exponential`: `self.delay_ms * 2^(attempt - 1)`
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self.delay_ms;
        let ms = match self.backoff {
            BackoffStrategy::Fixed => base,
            BackoffStrategy::Linear => base.saturating_mul(u64::from(attempt)),
            BackoffStrategy::Exponential => {
                base.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
            }
        };
        Duration::from_millis(ms)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P1-04: `FailureStrategy` default is `FailFast`.
    #[test]
    fn test_failure_strategy_default() {
        assert_eq!(FailureStrategy::default(), FailureStrategy::FailFast);
    }

    /// T-P1-05a: `RetryConfig` with exponential backoff calculates correct delays.
    #[test]
    fn test_retry_exponential_backoff() {
        let config = RetryConfig {
            max_attempts: 4,
            delay_ms: 100,
            backoff: BackoffStrategy::Exponential,
        };
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(100));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(400));
        assert_eq!(config.delay_for_attempt(4), Duration::from_millis(800));
    }

    /// T-P1-05b: `RetryConfig` with linear backoff calculates correct delays.
    #[test]
    fn test_retry_linear_backoff() {
        let config = RetryConfig {
            max_attempts: 3,
            delay_ms: 200,
            backoff: BackoffStrategy::Linear,
        };
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(600));
    }

    /// T-P1-05c: `RetryConfig` with fixed backoff always returns base delay.
    #[test]
    fn test_retry_fixed_backoff() {
        let config = RetryConfig {
            max_attempts: 3,
            delay_ms: 500,
            backoff: BackoffStrategy::Fixed,
        };
        assert_eq!(config.delay_for_attempt(1), Duration::from_millis(500));
        assert_eq!(config.delay_for_attempt(2), Duration::from_millis(500));
        assert_eq!(config.delay_for_attempt(3), Duration::from_millis(500));
    }

    /// T-P1-05d: Default `RetryConfig` has sensible values.
    #[test]
    fn test_retry_config_defaults() {
        let config = RetryConfig::default();
        assert_eq!(config.max_attempts, 3);
        assert_eq!(config.delay_ms, 500);
        assert_eq!(config.backoff, BackoffStrategy::Exponential);
    }

    /// `FailureStrategy` serialization round-trips.
    #[test]
    fn test_failure_strategy_serialization() {
        let strategy = FailureStrategy::Compensation {
            compensation_task_id: "cleanup".into(),
        };
        let json = serde_json::to_string(&strategy).unwrap();
        let deserialized: FailureStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, strategy);
    }
}
