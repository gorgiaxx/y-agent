//! Interval-based schedule trigger.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A fixed-interval schedule trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntervalSchedule {
    /// Interval in seconds between executions.
    pub interval_secs: u64,
}

impl IntervalSchedule {
    /// Create a new interval schedule.
    pub fn new(interval_secs: u64) -> Self {
        Self { interval_secs }
    }

    /// Create from minutes.
    pub fn from_minutes(minutes: u64) -> Self {
        Self::new(minutes * 60)
    }

    /// Create from hours.
    pub fn from_hours(hours: u64) -> Self {
        Self::new(hours * 3600)
    }

    /// Get the next fire time after `after`.
    pub fn next_fire(&self, after: DateTime<Utc>) -> DateTime<Utc> {
        let secs = i64::try_from(self.interval_secs).unwrap_or(i64::MAX);
        after + chrono::Duration::seconds(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_from_minutes() {
        let sched = IntervalSchedule::from_minutes(30);
        assert_eq!(sched.interval_secs, 1800);
    }

    #[test]
    fn test_interval_from_hours() {
        let sched = IntervalSchedule::from_hours(2);
        assert_eq!(sched.interval_secs, 7200);
    }

    #[test]
    fn test_interval_next_fire() {
        let sched = IntervalSchedule::new(600); // 10 minutes.
        let now = Utc::now();
        let next = sched.next_fire(now);
        assert_eq!((next - now).num_seconds(), 600);
    }
}
