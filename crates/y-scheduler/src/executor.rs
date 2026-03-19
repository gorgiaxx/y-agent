//! Schedule executor: translates triggers into workflow executions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::{Schedule, ScheduleStore};

/// Execution status for a schedule run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// Record of a schedule execution.
#[derive(Debug, Clone)]
pub struct ScheduleExecution {
    pub execution_id: String,
    pub schedule_id: String,
    pub triggered_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: ExecutionStatus,
    pub workflow_execution_id: Option<String>,
}

/// Context injected into scheduled workflow executions.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleContext {
    /// Originating schedule ID.
    pub schedule_id: String,
    /// When the trigger fired.
    pub trigger_time: DateTime<Utc>,
    /// Trigger type name.
    pub trigger_type: String,
    /// Incrementing counter for this schedule.
    pub execution_sequence: u64,
    /// Resolved parameter values.
    pub resolved_parameters: serde_json::Value,
}

/// Schedule executor that manages trigger-to-execution translation.
///
/// In Phase S3 this becomes async with `WorkflowDispatcher` trait and
/// concurrency policy enforcement.
pub struct ScheduleExecutor {
    executions: Vec<ScheduleExecution>,
    next_sequence: std::collections::HashMap<String, u64>,
}

impl ScheduleExecutor {
    /// Create a new executor.
    pub fn new() -> Self {
        Self {
            executions: Vec::new(),
            next_sequence: std::collections::HashMap::new(),
        }
    }

    /// Execute a schedule (placeholder — integrates with Orchestrator in Phase S3/S8).
    ///
    /// Returns the execution ID.
    pub fn trigger_execution(&mut self, schedule: &Schedule, store: &mut ScheduleStore) -> String {
        let seq = self.next_sequence.entry(schedule.id.clone()).or_insert(0);
        *seq += 1;

        let execution_id = format!("exec-{}-{seq}", schedule.id);
        let now = Utc::now();

        let execution = ScheduleExecution {
            execution_id: execution_id.clone(),
            schedule_id: schedule.id.clone(),
            triggered_at: now,
            started_at: Some(now),
            completed_at: Some(now), // Placeholder: instant completion.
            status: ExecutionStatus::Completed,
            workflow_execution_id: Some(format!("workflow-{execution_id}")),
        };

        self.executions.push(execution);

        // Update last-fire time in store.
        store.update_last_fire(&schedule.id, now);

        execution_id
    }

    /// Get execution history for a schedule.
    pub fn get_history(&self, schedule_id: &str) -> Vec<&ScheduleExecution> {
        self.executions
            .iter()
            .filter(|e| e.schedule_id == schedule_id)
            .collect()
    }

    /// Get total execution count.
    pub fn execution_count(&self) -> usize {
        self.executions.len()
    }
}

impl Default for ScheduleExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::store::TriggerConfig;

    use super::*;

    #[test]
    fn test_executor_trigger_execution() {
        let mut executor = ScheduleExecutor::new();
        let mut store = ScheduleStore::new();

        let schedule = Schedule::new(
            "daily-cleanup",
            "Daily Cleanup",
            TriggerConfig::Interval {
                interval_secs: 3600,
            },
            "cleanup-wf",
        );
        store.register(schedule.clone());

        let exec_id = executor.trigger_execution(&schedule, &mut store);
        assert!(exec_id.starts_with("exec-daily-cleanup-"));
        assert_eq!(executor.execution_count(), 1);

        // Verify last_fire updated.
        let s = store.get("daily-cleanup").unwrap();
        assert!(s.last_fire.is_some());
    }

    #[test]
    fn test_executor_history() {
        let mut executor = ScheduleExecutor::new();
        let mut store = ScheduleStore::new();

        let schedule = Schedule::new(
            "test",
            "Test",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        );
        store.register(schedule.clone());

        executor.trigger_execution(&schedule, &mut store);
        executor.trigger_execution(&schedule, &mut store);

        let history = executor.get_history("test");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].status, ExecutionStatus::Completed);
    }
}
