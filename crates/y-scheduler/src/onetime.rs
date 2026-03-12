//! One-time schedule trigger.
//!
//! Fires exactly once at a specific point in time and then auto-disables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A one-time schedule trigger that fires at a specific timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneTimeSchedule {
    /// The target fire time.
    pub at: DateTime<Utc>,
    /// Whether the trigger has already fired.
    #[serde(default)]
    pub fired: bool,
}

impl OneTimeSchedule {
    /// Create a new one-time trigger at the given time.
    pub fn new(at: DateTime<Utc>) -> Self {
        Self { at, fired: false }
    }

    /// Create a one-time trigger that fires after `duration` from now.
    pub fn after(duration: chrono::Duration) -> Self {
        Self::new(Utc::now() + duration)
    }

    /// Check whether the trigger should fire at the given time.
    ///
    /// Returns `true` if `now >= at` and the trigger hasn't fired yet.
    pub fn should_fire(&self, now: DateTime<Utc>) -> bool {
        !self.fired && now >= self.at
    }

    /// Mark the trigger as fired.
    pub fn mark_fired(&mut self) {
        self.fired = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_onetime_fires_at_target() {
        let target = Utc::now() - Duration::seconds(1);
        let sched = OneTimeSchedule::new(target);
        assert!(sched.should_fire(Utc::now()));
    }

    #[test]
    fn test_onetime_does_not_fire_early() {
        let target = Utc::now() + Duration::hours(1);
        let sched = OneTimeSchedule::new(target);
        assert!(!sched.should_fire(Utc::now()));
    }

    #[test]
    fn test_onetime_does_not_fire_twice() {
        let target = Utc::now() - Duration::seconds(1);
        let mut sched = OneTimeSchedule::new(target);
        assert!(sched.should_fire(Utc::now()));
        sched.mark_fired();
        assert!(!sched.should_fire(Utc::now()));
    }

    #[test]
    fn test_onetime_after_duration() {
        let sched = OneTimeSchedule::after(Duration::minutes(30));
        assert!(!sched.should_fire(Utc::now()));
        // Will fire 30 minutes from now.
        assert!(sched.should_fire(Utc::now() + Duration::minutes(31)));
    }
}
