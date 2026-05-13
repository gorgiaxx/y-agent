//! Trigger engine: evaluates all trigger types and produces fire events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cron::CronSchedule;
use crate::interval::IntervalSchedule;
use crate::onetime::OneTimeSchedule;
use crate::store::{Schedule, TriggerConfig};

/// A fired trigger event, produced when a trigger condition is met.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiredTrigger {
    /// The schedule that produced this trigger.
    pub schedule_id: String,
    /// When the trigger evaluation happened.
    pub fired_at: DateTime<Utc>,
    /// The type of trigger that fired.
    pub trigger_type: TriggerType,
}

/// Discriminant for trigger types (used in context injection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Cron,
    Interval,
    Event,
    OneTime,
}

impl std::fmt::Display for TriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cron => write!(f, "cron"),
            Self::Interval => write!(f, "interval"),
            Self::Event => write!(f, "event"),
            Self::OneTime => write!(f, "onetime"),
        }
    }
}

/// Evaluate a single schedule and determine whether it should fire.
///
/// Returns `Some(FiredTrigger)` if the trigger condition is met, `None` otherwise.
pub fn evaluate_trigger(schedule: &Schedule, now: DateTime<Utc>) -> Option<FiredTrigger> {
    if !schedule.enabled {
        return None;
    }

    let should_fire = match &schedule.trigger {
        TriggerConfig::Cron { expression, .. } => {
            let cron = CronSchedule::new(expression);
            match schedule.last_fire {
                Some(last) => cron.next_fire(last).is_some_and(|next| next <= now),
                None => true, // Never fired; fire immediately.
            }
        }
        TriggerConfig::Interval { interval_secs } => {
            let interval = IntervalSchedule::new(*interval_secs);
            match schedule.last_fire {
                Some(last) => interval.next_fire(last) <= now,
                None => true, // Never fired; fire immediately.
            }
        }
        TriggerConfig::OneTime { at } => {
            let ot = OneTimeSchedule::new(*at);
            ot.should_fire(now)
        }
        TriggerConfig::Event { .. } => {
            // Event triggers are handled externally via the EventBridge (Phase S6).
            false
        }
    };

    if should_fire {
        let trigger_type = match &schedule.trigger {
            TriggerConfig::Cron { .. } => TriggerType::Cron,
            TriggerConfig::Interval { .. } => TriggerType::Interval,
            TriggerConfig::OneTime { .. } => TriggerType::OneTime,
            TriggerConfig::Event { .. } => TriggerType::Event,
        };
        Some(FiredTrigger {
            schedule_id: schedule.id.clone(),
            fired_at: now,
            trigger_type,
        })
    } else {
        None
    }
}

/// Evaluate all schedules and collect the ones that should fire.
pub fn evaluate_all(schedules: &[Schedule], now: DateTime<Utc>) -> Vec<FiredTrigger> {
    schedules
        .iter()
        .filter_map(|s| evaluate_trigger(s, now))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_trigger_engine_cron_fires() {
        let mut schedule = Schedule::new(
            "test-cron",
            "Test Cron",
            TriggerConfig::Cron {
                expression: "* * * * *".into(),
                timezone: "UTC".into(),
            },
            "wf",
        );
        // Set last_fire to 2 minutes ago — should fire because next_fire (1 min later) is in the past.
        schedule.last_fire = Some(Utc::now() - Duration::minutes(2));

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_some());
        assert_eq!(result.unwrap().trigger_type, TriggerType::Cron);
    }

    #[test]
    fn test_trigger_engine_cron_does_not_fire_early() {
        let mut schedule = Schedule::new(
            "test-cron",
            "Test Cron",
            TriggerConfig::Cron {
                expression: "* * * * *".into(),
                timezone: "UTC".into(),
            },
            "wf",
        );
        // Set last_fire to just now — next fire should be in the future.
        schedule.last_fire = Some(Utc::now());

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn test_trigger_engine_interval_fires() {
        let mut schedule = Schedule::new(
            "test-interval",
            "Test Interval",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        );
        schedule.last_fire = Some(Utc::now() - Duration::seconds(120));

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_some());
        assert_eq!(result.unwrap().trigger_type, TriggerType::Interval);
    }

    #[test]
    fn test_trigger_engine_interval_does_not_fire_early() {
        let mut schedule = Schedule::new(
            "test-interval",
            "Test Interval",
            TriggerConfig::Interval {
                interval_secs: 3600,
            },
            "wf",
        );
        schedule.last_fire = Some(Utc::now() - Duration::seconds(10));

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn test_trigger_engine_onetime_fires() {
        let schedule = Schedule::new(
            "test-onetime",
            "Test OneTime",
            TriggerConfig::OneTime {
                at: Utc::now() - Duration::seconds(1),
            },
            "wf",
        );

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_some());
        assert_eq!(result.unwrap().trigger_type, TriggerType::OneTime);
    }

    #[test]
    fn test_trigger_engine_disabled_schedule() {
        let mut schedule = Schedule::new(
            "test-disabled",
            "Test Disabled",
            TriggerConfig::Interval { interval_secs: 1 },
            "wf",
        );
        schedule.enabled = false;

        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_none());
    }

    #[test]
    fn test_trigger_engine_never_fired() {
        let schedule = Schedule::new(
            "test-new",
            "Test New",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        );
        // last_fire is None (never fired) → should fire immediately.
        let result = evaluate_trigger(&schedule, Utc::now());
        assert!(result.is_some());
    }

    #[test]
    fn test_evaluate_all() {
        let schedules = vec![
            {
                let mut s = Schedule::new(
                    "s1",
                    "S1",
                    TriggerConfig::Interval { interval_secs: 60 },
                    "wf",
                );
                s.last_fire = Some(Utc::now() - Duration::seconds(120));
                s
            },
            {
                let mut s = Schedule::new(
                    "s2",
                    "S2",
                    TriggerConfig::Interval {
                        interval_secs: 3600,
                    },
                    "wf",
                );
                s.last_fire = Some(Utc::now() - Duration::seconds(10));
                s
            },
        ];

        let fired = evaluate_all(&schedules, Utc::now());
        // Only s1 should fire (interval 60s, last fire 120s ago)
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].schedule_id, "s1");
    }
}
