//! Cron-style schedule trigger using the `croner` crate for full
//! 5-field cron expression parsing.

use chrono::{DateTime, Utc};
use croner::Cron;
use serde::{Deserialize, Serialize};

/// A cron-based schedule trigger.
///
/// Supports standard 5-field cron expressions (minute, hour, day-of-month,
/// month, day-of-week) plus extended syntax (`L`, `#`, `W`) via `croner`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    /// Cron expression string (e.g., `"0 9 * * MON"`).
    pub expression: String,
    /// Timezone for evaluation (default: UTC).
    #[serde(default = "default_tz")]
    pub timezone: String,
}

fn default_tz() -> String {
    "UTC".into()
}

impl CronSchedule {
    /// Create a new cron schedule with the given expression.
    pub fn new(expression: &str) -> Self {
        Self {
            expression: expression.to_string(),
            timezone: "UTC".into(),
        }
    }

    /// Create a cron schedule with a specific timezone.
    #[must_use]
    pub fn with_timezone(mut self, tz: &str) -> Self {
        self.timezone = tz.to_string();
        self
    }

    /// Parse the cron expression into a `Cron` instance.
    fn parsed(&self) -> Option<Cron> {
        Cron::new(&self.expression).parse().ok()
    }

    /// Validate whether the cron expression is parseable.
    pub fn is_valid(&self) -> bool {
        self.parsed().is_some()
    }

    /// Get the next fire time after `after`.
    ///
    /// Returns `None` if the expression is invalid or has no future occurrence.
    pub fn next_fire(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let cron = self.parsed()?;
        cron.find_next_occurrence(&after, false).ok()
    }

    /// Check whether a given `DateTime` matches the cron expression.
    pub fn matches(&self, time: &DateTime<Utc>) -> bool {
        self.parsed()
            .is_some_and(|c| c.is_time_matching(time).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_valid_expression() {
        let cron = CronSchedule::new("0 9 * * MON");
        assert!(cron.is_valid());
    }

    #[test]
    fn test_cron_invalid_expression() {
        let cron = CronSchedule::new("invalid cron expr!!");
        assert!(!cron.is_valid());
        assert!(cron.next_fire(Utc::now()).is_none());
    }

    #[test]
    fn test_cron_next_fire_basic() {
        let cron = CronSchedule::new("* * * * *"); // every minute
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
        // Should be within 60 seconds.
        let diff = (next - now).num_seconds();
        assert!(diff <= 60, "Expected <=60s but got {diff}s");
    }

    #[test]
    fn test_cron_next_fire_hourly() {
        let cron = CronSchedule::new("0 * * * *"); // every hour at :00
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
        assert_eq!(next.format("%M").to_string(), "00");
    }

    #[test]
    fn test_cron_next_fire_daily() {
        let cron = CronSchedule::new("0 2 * * *"); // daily at 02:00
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
        assert_eq!(next.format("%H:%M").to_string(), "02:00");
    }

    #[test]
    fn test_cron_next_fire_day_of_week() {
        let cron = CronSchedule::new("0 9 * * 1"); // every Monday at 09:00
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
        // chrono: Monday = 1 in ISO weekday format (%u)
        assert_eq!(next.format("%u").to_string(), "1");
        assert_eq!(next.format("%H:%M").to_string(), "09:00");
    }

    #[test]
    fn test_cron_every_n_minutes() {
        let cron = CronSchedule::new("*/15 * * * *"); // every 15 minutes
        assert!(cron.is_valid());
        let now = Utc::now();
        let next = cron.next_fire(now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_cron_with_timezone() {
        let cron = CronSchedule::new("0 9 * * *").with_timezone("Asia/Shanghai");
        assert_eq!(cron.timezone, "Asia/Shanghai");
        assert!(cron.is_valid());
    }
}
