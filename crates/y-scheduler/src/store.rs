//! `ScheduleStore`: in-memory schedule registry with CRUD operations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{ConcurrencyPolicy, MissedPolicy};

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

/// Per-schedule execution policies.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulePolicies {
    /// How to handle missed fires (e.g., during downtime).
    #[serde(default)]
    pub missed_policy: MissedPolicy,
    /// Behaviour when a trigger fires while the previous execution is still running.
    #[serde(default)]
    pub concurrency_policy: ConcurrencyPolicy,
    /// Maximum number of executions allowed per hour (0 = unlimited).
    #[serde(default)]
    pub max_executions_per_hour: u32,
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
    /// Per-schedule execution policies.
    #[serde(default)]
    pub policies: SchedulePolicies,
    /// Human-readable description of the schedule's purpose.
    #[serde(default)]
    pub description: String,
    /// Arbitrary tags for filtering / grouping.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Last fire timestamp.
    pub last_fire: Option<DateTime<Utc>>,
}

impl Schedule {
    /// Create a new schedule with sensible defaults.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        trigger: TriggerConfig,
        workflow_id: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            name: name.into(),
            enabled: true,
            trigger,
            workflow_id: workflow_id.into(),
            parameter_values: serde_json::json!({}),
            policies: SchedulePolicies::default(),
            description: String::new(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            last_fire: None,
        }
    }

    /// Builder: set parameter values.
    #[must_use]
    pub fn with_params(mut self, params: serde_json::Value) -> Self {
        self.parameter_values = params;
        self
    }

    /// Builder: set policies.
    #[must_use]
    pub fn with_policies(mut self, policies: SchedulePolicies) -> Self {
        self.policies = policies;
        self
    }

    /// Builder: set description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Builder: set tags.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// In-memory schedule store.
///
/// In production, backed by `SQLite` with WAL mode (see `SqliteScheduleStore`).
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

    /// Register a schedule (inserts or replaces by ID).
    pub fn register(&mut self, schedule: Schedule) {
        self.schedules.retain(|s| s.id != schedule.id);
        self.schedules.push(schedule);
    }

    /// Update an existing schedule. Returns `false` if not found.
    pub fn update(&mut self, schedule: Schedule) -> bool {
        if let Some(existing) = self.schedules.iter_mut().find(|s| s.id == schedule.id) {
            *existing = schedule;
            true
        } else {
            false
        }
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

    /// Filter schedules by tag.
    pub fn list_by_tag(&self, tag: &str) -> Vec<&Schedule> {
        self.schedules
            .iter()
            .filter(|s| s.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// Enable or disable a schedule.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> bool {
        if let Some(s) = self.schedules.iter_mut().find(|s| s.id == id) {
            s.enabled = enabled;
            s.updated_at = Utc::now();
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
    use crate::config::{ConcurrencyPolicy, MissedPolicy};

    fn make_schedule(id: &str) -> Schedule {
        Schedule::new(
            id,
            format!("Schedule {id}"),
            TriggerConfig::Interval {
                interval_secs: 3600,
            },
            "test-wf",
        )
        .with_tags(vec!["maintenance".into()])
    }

    #[test]
    fn test_store_register_and_get() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1"));
        assert!(store.get("s1").is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_list_enabled() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1"));
        let mut s2 = make_schedule("s2");
        s2.enabled = false;
        store.register(s2);
        store.register(make_schedule("s3"));
        assert_eq!(store.list_enabled().len(), 2);
    }

    #[test]
    fn test_store_remove() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1"));
        assert!(store.remove("s1"));
        assert!(store.is_empty());
    }

    #[test]
    fn test_store_enable_disable() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1"));
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

    #[test]
    fn test_schedule_with_policies() {
        let schedule = Schedule::new(
            "s1",
            "Test",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        )
        .with_policies(SchedulePolicies {
            missed_policy: MissedPolicy::CatchUp,
            concurrency_policy: ConcurrencyPolicy::Queue,
            max_executions_per_hour: 5,
        })
        .with_description("A test schedule")
        .with_tags(vec!["test".into(), "daily".into()]);

        assert_eq!(schedule.policies.missed_policy, MissedPolicy::CatchUp);
        assert_eq!(
            schedule.policies.concurrency_policy,
            ConcurrencyPolicy::Queue
        );
        assert_eq!(schedule.policies.max_executions_per_hour, 5);
        assert_eq!(schedule.description, "A test schedule");
        assert_eq!(schedule.tags.len(), 2);
    }

    #[test]
    fn test_store_update() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1"));
        let mut updated = store.get("s1").unwrap().clone();
        updated.name = "Updated Name".into();
        assert!(store.update(updated));
        assert_eq!(store.get("s1").unwrap().name, "Updated Name");
    }

    #[test]
    fn test_store_list_by_tag() {
        let mut store = ScheduleStore::new();
        store.register(make_schedule("s1")); // has "maintenance" tag
        store.register(Schedule::new(
            "s2",
            "No tag",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        ));
        assert_eq!(store.list_by_tag("maintenance").len(), 1);
        assert_eq!(store.list_by_tag("nonexistent").len(), 0);
    }
}
