//! Cron-style schedule trigger.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A cron-based schedule trigger.
///
/// Supports standard 5-field cron expressions (minute, hour, day-of-month,
/// month, day-of-week). Full cron parsing is deferred to a future phase;
/// this implementation provides a simplified "every N hours" approach
/// for the initial version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    /// Cron expression string (e.g., "0 2 * * *").
    pub expression: String,
    /// Timezone for evaluation (default: UTC).
    #[serde(default = "default_tz")]
    pub timezone: String,
    /// Computed interval in seconds (simplified; full cron parsing deferred).
    #[serde(skip)]
    interval_secs: Option<u64>,
}

fn default_tz() -> String {
    "UTC".into()
}

impl CronSchedule {
    /// Create a new cron schedule with the given expression.
    pub fn new(expression: &str) -> Self {
        let interval_secs = Self::parse_simple(expression);
        Self {
            expression: expression.to_string(),
            timezone: "UTC".into(),
            interval_secs,
        }
    }

    /// Get the next fire time after `after`.
    ///
    /// For the simplified implementation, computes based on parsed interval.
    pub fn next_fire(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.interval_secs
            .and_then(|secs| i64::try_from(secs).ok())
            .map(|secs| after + chrono::Duration::seconds(secs))
    }

    /// Simple parser: extracts interval from common patterns.
    ///
    /// Supports:
    /// - `"0 * * * *"` → every hour (3600s)
    /// - `"0 */N * * *"` → every N hours
    /// - `"*/N * * * *"` → every N minutes
    fn parse_simple(expr: &str) -> Option<u64> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return None;
        }

        // "*/N * * * *" → every N minutes.
        if parts[0].starts_with("*/") {
            if let Ok(n) = parts[0][2..].parse::<u64>() {
                return Some(n * 60);
            }
        }

        // "0 */N * * *" → every N hours.
        if parts[0] == "0" && parts[1].starts_with("*/") {
            if let Ok(n) = parts[1][2..].parse::<u64>() {
                return Some(n * 3600);
            }
        }

        // "0 * * * *" → every hour.
        if parts[0] == "0" && parts[1] == "*" {
            return Some(3600);
        }

        // "0 N * * *" → daily at hour N (24h interval).
        if parts[0] == "0" && parts[2] == "*" && parts[3] == "*" && parts[4] == "*" {
            return Some(86400); // 24 hours.
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_every_hour() {
        let cron = CronSchedule::new("0 * * * *");
        assert_eq!(cron.interval_secs, Some(3600));
    }

    #[test]
    fn test_cron_every_n_minutes() {
        let cron = CronSchedule::new("*/15 * * * *");
        assert_eq!(cron.interval_secs, Some(900)); // 15 * 60
    }

    #[test]
    fn test_cron_daily() {
        let cron = CronSchedule::new("0 2 * * *");
        assert_eq!(cron.interval_secs, Some(86400));
    }

    #[test]
    fn test_cron_next_fire() {
        let cron = CronSchedule::new("0 * * * *");
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
        assert_eq!((next - now).num_seconds(), 3600);
    }

    #[test]
    fn test_cron_invalid_expression() {
        let cron = CronSchedule::new("invalid");
        assert!(cron.interval_secs.is_none());
        assert!(cron.next_fire(Utc::now()).is_none());
    }
}
