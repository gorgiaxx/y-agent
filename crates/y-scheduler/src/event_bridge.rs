//! Event bridge: connects the `y-hooks` event bus to `EventSchedule` triggers.
//!
//! The `EventBridge` receives events (e.g., from file watchers, webhooks) and
//! matches them against registered `EventSchedule` triggers, applying debounce
//! and optional payload filtering before enqueueing trigger events.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{debug, info};

use crate::queue::TriggerSender;
use crate::store::{ScheduleStore, TriggerConfig};

// `Schedule` is used in tests.
#[cfg(test)]
use crate::store::Schedule;
use crate::trigger::{FiredTrigger, TriggerType};

/// An incoming event from the hook system or external source.
#[derive(Debug, Clone)]
pub struct IncomingEvent {
    /// Event type identifier (e.g. `"file.changed"`).
    pub event_type: String,
    /// Optional payload with event details.
    pub payload: Option<Value>,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
}

/// Event bridge that evaluates incoming events against registered schedules.
pub struct EventBridge {
    /// Track last event time per schedule for debounce.
    last_event: HashMap<String, DateTime<Utc>>,
}

impl EventBridge {
    /// Create a new event bridge.
    pub fn new() -> Self {
        Self {
            last_event: HashMap::new(),
        }
    }

    /// Process an incoming event against registered schedules.
    ///
    /// Returns the list of schedule IDs that matched and were enqueued.
    pub async fn process_event(
        &mut self,
        event: &IncomingEvent,
        store: &ScheduleStore,
        tx: &TriggerSender,
    ) -> Vec<String> {
        let mut matched = Vec::new();

        for schedule in store.list_enabled() {
            if let TriggerConfig::Event {
                event_type,
                debounce_secs,
            } = &schedule.trigger
            {
                // Check event type.
                if event_type != &event.event_type {
                    continue;
                }

                // Apply debounce.
                if *debounce_secs > 0 {
                    if let Some(last) = self.last_event.get(&schedule.id) {
                        let elapsed = (event.timestamp - *last).num_seconds();
                        if elapsed < *debounce_secs as i64 {
                            debug!(
                                schedule_id = %schedule.id,
                                elapsed_secs = elapsed,
                                debounce_secs = debounce_secs,
                                "Event debounced"
                            );
                            continue;
                        }
                    }
                }

                // Match! Enqueue trigger.
                let trigger = FiredTrigger {
                    schedule_id: schedule.id.clone(),
                    fired_at: event.timestamp,
                    trigger_type: TriggerType::Event,
                };

                if tx.send(trigger).await.is_ok() {
                    info!(schedule_id = %schedule.id, event_type = %event.event_type, "Event trigger fired");
                    self.last_event.insert(schedule.id.clone(), event.timestamp);
                    matched.push(schedule.id.clone());
                }
            }
        }

        matched
    }
}

impl Default for EventBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::trigger_queue;
    use chrono::Duration;

    fn event_schedule(id: &str, event_type: &str, debounce: u64) -> Schedule {
        Schedule::new(
            id,
            id,
            TriggerConfig::Event {
                event_type: event_type.to_string(),
                debounce_secs: debounce,
            },
            "wf",
        )
    }

    #[tokio::test]
    async fn test_event_bridge_matches_event_type() {
        let mut bridge = EventBridge::new();
        let mut store = ScheduleStore::new();
        store.register(event_schedule("s1", "file.changed", 0));

        let (tx, mut rx) = trigger_queue();
        let event = IncomingEvent {
            event_type: "file.changed".into(),
            payload: None,
            timestamp: Utc::now(),
        };

        let matched = bridge.process_event(&event, &store, &tx).await;
        assert_eq!(matched, vec!["s1"]);

        let trigger = rx.recv().await.unwrap();
        assert_eq!(trigger.schedule_id, "s1");
        assert_eq!(trigger.trigger_type, TriggerType::Event);
    }

    #[tokio::test]
    async fn test_event_bridge_no_match() {
        let mut bridge = EventBridge::new();
        let mut store = ScheduleStore::new();
        store.register(event_schedule("s1", "file.changed", 0));

        let (tx, _rx) = trigger_queue();
        let event = IncomingEvent {
            event_type: "file.created".into(),
            payload: None,
            timestamp: Utc::now(),
        };

        let matched = bridge.process_event(&event, &store, &tx).await;
        assert!(matched.is_empty());
    }

    #[tokio::test]
    async fn test_event_bridge_debounce() {
        let mut bridge = EventBridge::new();
        let mut store = ScheduleStore::new();
        store.register(event_schedule("s1", "file.changed", 5)); // 5s debounce

        let (tx, mut rx) = trigger_queue();
        let now = Utc::now();

        // First event — should fire.
        let event1 = IncomingEvent {
            event_type: "file.changed".into(),
            payload: None,
            timestamp: now,
        };
        let matched1 = bridge.process_event(&event1, &store, &tx).await;
        assert_eq!(matched1.len(), 1);

        // Second event 2s later — should be debounced.
        let event2 = IncomingEvent {
            event_type: "file.changed".into(),
            payload: None,
            timestamp: now + Duration::seconds(2),
        };
        let matched2 = bridge.process_event(&event2, &store, &tx).await;
        assert!(matched2.is_empty());

        // Third event 10s later — should fire.
        let event3 = IncomingEvent {
            event_type: "file.changed".into(),
            payload: None,
            timestamp: now + Duration::seconds(10),
        };
        let matched3 = bridge.process_event(&event3, &store, &tx).await;
        assert_eq!(matched3.len(), 1);

        // Should have 2 triggers total.
        let t1 = rx.recv().await.unwrap();
        let t2 = rx.recv().await.unwrap();
        assert_eq!(t1.schedule_id, "s1");
        assert_eq!(t2.schedule_id, "s1");
    }

    #[tokio::test]
    async fn test_event_bridge_disabled_schedule() {
        let mut bridge = EventBridge::new();
        let mut store = ScheduleStore::new();
        let mut s = event_schedule("s1", "file.changed", 0);
        s.enabled = false;
        store.register(s);

        let (tx, _rx) = trigger_queue();
        let event = IncomingEvent {
            event_type: "file.changed".into(),
            payload: None,
            timestamp: Utc::now(),
        };

        let matched = bridge.process_event(&event, &store, &tx).await;
        assert!(matched.is_empty());
    }
}
