//! Recovery manager: detects and handles missed schedule fires after restart.

use chrono::{DateTime, Duration, Utc};
use tracing::{debug, info};

use crate::config::MissedPolicy;
use crate::store::{Schedule, ScheduleStore, TriggerConfig};
use crate::trigger::{FiredTrigger, TriggerType};

/// Result of the recovery process.
#[derive(Debug, Default)]
pub struct RecoveryResult {
    /// Schedules that were caught up (fired once).
    pub caught_up: Vec<String>,
    /// Schedules that were skipped.
    pub skipped: Vec<String>,
    /// Schedules that were backfilled (multiple fires).
    pub backfilled: Vec<(String, usize)>,
}

/// Recover missed schedule fires.
///
/// For each enabled schedule with a `last_fire` time, compute whether fires
/// were missed (`last_fire` + interval < now). Apply the schedule's `missed_policy`.
///
/// Returns a list of `FiredTrigger` events to be dispatched and a summary.
pub fn recover_missed(
    store: &ScheduleStore,
    now: DateTime<Utc>,
) -> (Vec<FiredTrigger>, RecoveryResult) {
    let mut triggers = Vec::new();
    let mut result = RecoveryResult::default();

    for schedule in store.list_enabled() {
        let Some(last_fire) = schedule.last_fire else {
            // Never fired — treat as needing a single fire.
            debug!(schedule_id = %schedule.id, "Never fired, firing now");
            triggers.push(make_trigger(schedule, now));
            result.caught_up.push(schedule.id.clone());
            continue;
        };

        let interval = match compute_interval(schedule) {
            Some(i) => i,
            None => continue, // OneTime or Event — skip recovery.
        };

        if interval.is_zero() {
            continue;
        }

        let elapsed = now - last_fire;
        if elapsed <= interval {
            // Not missed.
            continue;
        }

        let policy = &schedule.policies.missed_policy;
        match policy {
            MissedPolicy::Skip => {
                info!(schedule_id = %schedule.id, "Missed schedule, skipping (policy=skip)");
                result.skipped.push(schedule.id.clone());
            }
            MissedPolicy::CatchUp => {
                info!(schedule_id = %schedule.id, "Missed schedule, catching up (policy=catch_up)");
                triggers.push(make_trigger(schedule, now));
                result.caught_up.push(schedule.id.clone());
            }
            MissedPolicy::Backfill => {
                let missed_count = (elapsed.num_seconds() / interval.num_seconds()).max(1) as usize;
                // Cap backfill to a reasonable limit.
                let capped = missed_count.min(100);
                info!(
                    schedule_id = %schedule.id,
                    missed_count = capped,
                    "Missed schedule, backfilling (policy=backfill)"
                );
                for i in 0..capped {
                    let fire_time = last_fire + interval * (i as i32 + 1);
                    triggers.push(make_trigger(schedule, fire_time));
                }
                result.backfilled.push((schedule.id.clone(), capped));
            }
        }
    }

    (triggers, result)
}

/// Compute the effective interval for a schedule (for recovery purposes).
fn compute_interval(schedule: &Schedule) -> Option<Duration> {
    match &schedule.trigger {
        TriggerConfig::Interval { interval_secs } => {
            Some(Duration::seconds(*interval_secs as i64))
        }
        TriggerConfig::Cron { expression, .. } => {
            // Approximate interval by computing two consecutive next-fires.
            use crate::cron::CronSchedule;
            let cron = CronSchedule::new(expression);
            let base = Utc::now() - Duration::days(1);
            let first = cron.next_fire(base)?;
            let second = cron.next_fire(first)?;
            Some(second - first)
        }
        TriggerConfig::OneTime { .. } | TriggerConfig::Event { .. } => None,
    }
}

/// Helper to create a fired trigger.
fn make_trigger(schedule: &Schedule, at: DateTime<Utc>) -> FiredTrigger {
    let trigger_type = match &schedule.trigger {
        TriggerConfig::Cron { .. } => TriggerType::Cron,
        TriggerConfig::Interval { .. } => TriggerType::Interval,
        TriggerConfig::OneTime { .. } => TriggerType::OneTime,
        TriggerConfig::Event { .. } => TriggerType::Event,
    };
    FiredTrigger {
        schedule_id: schedule.id.clone(),
        fired_at: at,
        trigger_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConcurrencyPolicy;
    use crate::store::SchedulePolicies;

    fn interval_schedule(id: &str, interval_secs: u64, policy: MissedPolicy) -> Schedule {
        Schedule::new(id, id, TriggerConfig::Interval { interval_secs }, "wf")
            .with_policies(SchedulePolicies {
                missed_policy: policy,
                concurrency_policy: ConcurrencyPolicy::default(),
                max_executions_per_hour: 0,
            })
    }

    #[test]
    fn test_recovery_skip() {
        let mut store = ScheduleStore::new();
        let mut s = interval_schedule("s1", 60, MissedPolicy::Skip);
        s.last_fire = Some(Utc::now() - Duration::minutes(5));
        store.register(s);

        let (triggers, result) = recover_missed(&store, Utc::now());
        assert!(triggers.is_empty());
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_recovery_catch_up() {
        let mut store = ScheduleStore::new();
        let mut s = interval_schedule("s1", 60, MissedPolicy::CatchUp);
        s.last_fire = Some(Utc::now() - Duration::minutes(5));
        store.register(s);

        let (triggers, result) = recover_missed(&store, Utc::now());
        assert_eq!(triggers.len(), 1);
        assert_eq!(result.caught_up.len(), 1);
    }

    #[test]
    fn test_recovery_backfill() {
        let mut store = ScheduleStore::new();
        let mut s = interval_schedule("s1", 60, MissedPolicy::Backfill);
        s.last_fire = Some(Utc::now() - Duration::minutes(5));
        store.register(s);

        let (triggers, result) = recover_missed(&store, Utc::now());
        // 5 minutes / 1 minute interval = 5 missed fires.
        assert_eq!(triggers.len(), 5);
        assert_eq!(result.backfilled.len(), 1);
        assert_eq!(result.backfilled[0].1, 5);
    }

    #[test]
    fn test_recovery_not_missed() {
        let mut store = ScheduleStore::new();
        let mut s = interval_schedule("s1", 3600, MissedPolicy::CatchUp);
        s.last_fire = Some(Utc::now() - Duration::seconds(10));
        store.register(s);

        let (triggers, result) = recover_missed(&store, Utc::now());
        assert!(triggers.is_empty());
        assert!(result.caught_up.is_empty());
    }

    #[test]
    fn test_recovery_never_fired() {
        let mut store = ScheduleStore::new();
        let s = interval_schedule("s1", 60, MissedPolicy::Skip);
        // last_fire = None → should fire once.
        store.register(s);

        let (triggers, result) = recover_missed(&store, Utc::now());
        assert_eq!(triggers.len(), 1);
        assert_eq!(result.caught_up.len(), 1);
    }

    #[test]
    fn test_recovery_disabled_schedule_skipped() {
        let mut store = ScheduleStore::new();
        let mut s = interval_schedule("s1", 60, MissedPolicy::CatchUp);
        s.last_fire = Some(Utc::now() - Duration::minutes(5));
        s.enabled = false;
        store.register(s);

        let (triggers, _result) = recover_missed(&store, Utc::now());
        // Disabled schedules not in list_enabled().
        assert!(triggers.is_empty());
    }
}
