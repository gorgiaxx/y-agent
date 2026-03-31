//! Event-driven schedule trigger.

use serde::{Deserialize, Serialize};

/// An event-driven schedule trigger.
///
/// Fires when a matching event is received from the `y-hooks` event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSchedule {
    /// Event type to match (e.g., "file.changed").
    pub event_type: String,
    /// Optional payload filter (Glob pattern on a field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<EventFilter>,
    /// Debounce window in seconds (collapse rapid events).
    #[serde(default)]
    pub debounce_secs: u64,
}

/// Filter for event payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    /// Field path to match (e.g., "payload.path").
    pub field: String,
    /// Glob pattern to match against the field value.
    pub pattern: String,
}

impl EventSchedule {
    /// Create a new event schedule.
    pub fn new(event_type: &str) -> Self {
        Self {
            event_type: event_type.to_string(),
            filter: None,
            debounce_secs: 0,
        }
    }

    /// Set a debounce window.
    #[must_use]
    pub fn with_debounce(mut self, secs: u64) -> Self {
        self.debounce_secs = secs;
        self
    }

    /// Set a filter.
    #[must_use]
    pub fn with_filter(mut self, field: &str, pattern: &str) -> Self {
        self.filter = Some(EventFilter {
            field: field.to_string(),
            pattern: pattern.to_string(),
        });
        self
    }

    /// Check if an event type matches this trigger.
    pub fn matches_event_type(&self, event_type: &str) -> bool {
        self.event_type == event_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_schedule_creation() {
        let sched = EventSchedule::new("file.changed")
            .with_debounce(5)
            .with_filter("payload.path", "*.md");
        assert_eq!(sched.event_type, "file.changed");
        assert_eq!(sched.debounce_secs, 5);
        assert!(sched.filter.is_some());
    }

    #[test]
    fn test_event_matches_type() {
        let sched = EventSchedule::new("file.changed");
        assert!(sched.matches_event_type("file.changed"));
        assert!(!sched.matches_event_type("file.created"));
    }
}
