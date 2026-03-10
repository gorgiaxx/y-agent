//! `ScheduleStore`: in-memory schedule registry with CRUD operations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Trigger configuration variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerConfig {
    /// Cron expression trigger.
    Cron {
        expression: String,
        #[serde(default = "default_tz")]
        timezone: String,
    },
    /// Fixed interval trigger.
    Interval { interval_secs: u64 },
    /// Event-driven trigger.
    Event {
        event_type: String,
        #[serde(default)]
        debounce_secs: u64,
    },
    /// One-time execution at a specific time.
    OneTime { at: DateTime<Utc> },
}

fn default_tz() -> String {
    "UTC".into()
}

/// A schedule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// Unique identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the schedule is active.
    pub enabled: bool,
    /// Trigger configuration.
    pub trigger: TriggerConfig,
    /// Workflow to execute.
    pub workflow_id: String,
    /// Parameter values for the workflow.
    pub parameter_values: serde_json::Value,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last fire timestamp.
    pub last_fire: Option<DateTime<Utc>>,
}

/// In-memory schedule store.
///
/// In production, persisted to `SQLite` with WAL mode.
pub struct ScheduleStore {
    schedules: Vec<Schedule>,
}

impl ScheduleStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            schedules: Vec::new(),
        }
    }

    /// Register a schedule.
    pub fn register(&mut self, schedule: Schedule) {
        // Replace if exists.
        self.schedules.retain(|s| s.id != schedule.id);
        self.schedules.push(schedule);
    }

    /// Get a schedule by ID.
    pub fn get(&self, id: &str) -> Option<&Schedule> {
        self.schedules.iter().find(|s| s.id == id)
    }

    /// List all schedules.
    pub fn list(&self) -> &[Schedule] {
        &self.schedules
    }

    /// List enabled schedules.
    pub fn list_enabled(&self) -> Vec<&Schedule> {
        self.schedules.iter().filter(|s| s.enabled).collect()
    }

    /// Enable or disable a schedule.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(s) = self.schedules.iter_mut().find(|s| s.id == id) {
            s.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Remove a schedule.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.schedules.len();
        self.schedules.retain(|s| s.id != id);
        self.schedules.len() < before
    }

    /// Update last fire time.
    pub fn update_last_fire(&mut self, id: &str, time: DateTime<Utc>) {
        if let Some(s) = self.schedules.iter_mut().find(|s| s.id == id) {
            s.last_fire = Some(time);
        }
    }

    /// Count schedules.
    pub fn len(&self) -> usize {
        self.schedules.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty()
    }
}

impl Default for ScheduleStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_schedule(id: &str, enabled: bool) -> Schedule {
        Schedule {
            id: id.into(),
            name: format!("Schedule {id}"),
            enabled,
            trigger: TriggerConfig::Interval {
                interval_secs: 3600,
            },
            workflow_id: "test-wf".into(),
            parameter_values: serde_json::json!({}),
            created_at: Utc::now(),
            last_fire: None,
        }
    }

    #[test]
    fn test_store_register_and_get() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1", true));
        assert!(store.get("s1").is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_list_enabled() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1", true));
        store.register(make_schedule("s2", false));
        store.register(make_schedule("s3", true));
        assert_eq!(store.list_enabled().len(), 2);
    }

    #[test]
    fn test_store_remove() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1", true));
        assert!(store.remove("s1"));
        assert!(store.is_empty());
    }

    #[test]
    fn test_store_enable_disable() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1", true));
        store.set_enabled("s1", false);
        assert!(!store.get("s1").unwrap().enabled);
    }

    #[test]
    fn test_trigger_config_serialization() {
        let cron = TriggerConfig::Cron {
            expression: "0 2 * * *".into(),
            timezone: "UTC".into(),
        };
        let json = serde_json::to_string(&cron).unwrap();
        let deserialized: TriggerConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, TriggerConfig::Cron { .. }));
    }
}
