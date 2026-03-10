//! Hook system configuration.

use serde::Deserialize;
use std::time::Duration;

/// Configuration for the hook system.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookConfig {
    /// Per-middleware timeout in milliseconds.
    #[serde(default = "default_middleware_timeout_ms")]
    pub middleware_timeout_ms: u64,

    /// Event bus channel capacity per subscriber.
    #[serde(default = "default_event_channel_capacity")]
    pub event_channel_capacity: usize,

    /// Maximum number of subscribers allowed.
    #[serde(default = "default_max_subscribers")]
    pub max_subscribers: usize,
}

fn default_middleware_timeout_ms() -> u64 {
    5000
}

fn default_event_channel_capacity() -> usize {
    1000
}

fn default_max_subscribers() -> usize {
    100
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            middleware_timeout_ms: default_middleware_timeout_ms(),
            event_channel_capacity: default_event_channel_capacity(),
            max_subscribers: default_max_subscribers(),
        }
    }
}

impl HookConfig {
    /// Get middleware timeout as a `Duration`.
    pub fn middleware_timeout(&self) -> Duration {
        Duration::from_millis(self.middleware_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = HookConfig::default();
        assert_eq!(config.middleware_timeout_ms, 5000);
        assert_eq!(config.event_channel_capacity, 1000);
        assert_eq!(config.max_subscribers, 100);
    }

    #[test]
    fn test_middleware_timeout_duration() {
        let config = HookConfig::default();
        assert_eq!(config.middleware_timeout(), Duration::from_millis(5000));
    }
}
