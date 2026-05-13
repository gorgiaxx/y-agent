//! Schedule executor: translates triggers into workflow executions.
//!
//! Provides [`ScheduleExecution`] records with request/response capture for
//! diagnostics-style display, and [`ExecutionStore`] for in-memory storage.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::store::{Schedule, ScheduleStore};

/// Execution status for a schedule run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// Record of a schedule execution with request/response capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleExecution {
    /// Unique execution identifier.
    pub execution_id: String,
    /// Originating schedule ID.
    pub schedule_id: String,
    /// When the trigger fired.
    pub triggered_at: DateTime<Utc>,
    /// When execution started.
    pub started_at: Option<DateTime<Utc>>,
    /// When execution completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Current status of this execution.
    pub status: ExecutionStatus,
    /// Linked workflow execution ID (if any).
    pub workflow_execution_id: Option<String>,
    /// Request/input summary (JSON): parameters, trigger context, workflow info.
    pub request_summary: serde_json::Value,
    /// Response/output summary (JSON): execution result, output content.
    pub response_summary: serde_json::Value,
    /// Human-readable error message when status is `Failed`.
    pub error_message: Option<String>,
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

// ---------------------------------------------------------------------------
// ExecutionStore
// ---------------------------------------------------------------------------

/// In-memory store for schedule execution records.
///
/// Analogous to `ScheduleStore` but for execution history. Keeps records
/// ordered by `triggered_at` descending (most recent first).
pub struct ExecutionStore {
    records: Vec<ScheduleExecution>,
    /// Maximum records to retain (per schedule). 0 = unlimited.
    max_per_schedule: usize,
}

impl ExecutionStore {
    /// Create a new execution store with default retention (100 per schedule).
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_per_schedule: 100,
        }
    }

    /// Record a new execution.
    pub fn record(&mut self, execution: ScheduleExecution) {
        let schedule_id = execution.schedule_id.clone();
        self.records.push(execution);

        // Enforce per-schedule retention limit.
        if self.max_per_schedule > 0 {
            let count = self
                .records
                .iter()
                .filter(|e| e.schedule_id == schedule_id)
                .count();
            if count > self.max_per_schedule {
                // Remove the oldest entries for this schedule.
                let excess = count - self.max_per_schedule;
                let mut removed = 0;
                self.records.retain(|e| {
                    if e.schedule_id == schedule_id && removed < excess {
                        removed += 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
    }

    /// Update an existing execution record by ID.
    pub fn update(&mut self, execution_id: &str, updater: impl FnOnce(&mut ScheduleExecution)) {
        if let Some(rec) = self
            .records
            .iter_mut()
            .find(|e| e.execution_id == execution_id)
        {
            updater(rec);
        }
    }

    /// Get execution history for a schedule (most recent first).
    pub fn get_history(&self, schedule_id: &str) -> Vec<&ScheduleExecution> {
        let mut results: Vec<&ScheduleExecution> = self
            .records
            .iter()
            .filter(|e| e.schedule_id == schedule_id)
            .collect();
        results.sort_by(|a, b| b.triggered_at.cmp(&a.triggered_at));
        results
    }

    /// Get a single execution record by ID.
    pub fn get(&self, execution_id: &str) -> Option<&ScheduleExecution> {
        self.records.iter().find(|e| e.execution_id == execution_id)
    }

    /// List the most recent executions across all schedules.
    pub fn list_recent(&self, limit: usize) -> Vec<&ScheduleExecution> {
        let mut results: Vec<&ScheduleExecution> = self.records.iter().collect();
        results.sort_by(|a, b| b.triggered_at.cmp(&a.triggered_at));
        results.truncate(limit);
        results
    }

    /// Total number of execution records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for ExecutionStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ScheduleExecutor
// ---------------------------------------------------------------------------

/// Schedule executor that manages trigger-to-execution translation.
///
/// In Phase S3 this becomes async with `WorkflowDispatcher` trait and
/// concurrency policy enforcement.
pub struct ScheduleExecutor {
    next_sequence: std::collections::HashMap<String, u64>,
}

impl ScheduleExecutor {
    /// Create a new executor.
    pub fn new() -> Self {
        Self {
            next_sequence: std::collections::HashMap::new(),
        }
    }

    /// Execute a schedule (placeholder -- integrates with Orchestrator in Phase S3/S8).
    ///
    /// Records the execution in the provided `ExecutionStore` and returns the
    /// execution ID.
    pub fn trigger_execution(
        &mut self,
        schedule: &Schedule,
        store: &mut ScheduleStore,
        execution_store: &mut ExecutionStore,
    ) -> String {
        let seq = self.next_sequence.entry(schedule.id.clone()).or_insert(0);
        *seq += 1;

        let execution_id = format!("exec-{}-{seq}", schedule.id);
        let now = Utc::now();

        // Build request summary from schedule context.
        let request_summary = serde_json::json!({
            "schedule_id": schedule.id,
            "schedule_name": schedule.name,
            "workflow_id": schedule.workflow_id,
            "trigger": serde_json::to_value(&schedule.trigger).unwrap_or_default(),
            "parameter_values": schedule.parameter_values,
            "execution_sequence": *seq,
            "trigger_time": now.to_rfc3339(),
        });

        // Placeholder response summary (instant completion).
        let response_summary = serde_json::json!({
            "status": "completed",
            "message": "Workflow execution completed (placeholder)",
            "workflow_execution_id": format!("workflow-{execution_id}"),
        });

        let execution = ScheduleExecution {
            execution_id: execution_id.clone(),
            schedule_id: schedule.id.clone(),
            triggered_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            status: ExecutionStatus::Completed,
            workflow_execution_id: Some(format!("workflow-{execution_id}")),
            request_summary,
            response_summary,
            error_message: None,
        };

        execution_store.record(execution);

        // Update last-fire time in schedule store.
        store.update_last_fire(&schedule.id, now);

        execution_id
    }

    /// Get next sequence number for a schedule (for external callers).
    pub fn next_sequence_for(&mut self, schedule_id: &str) -> u64 {
        let seq = self
            .next_sequence
            .entry(schedule_id.to_string())
            .or_insert(0);
        *seq += 1;
        *seq
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
        let mut exec_store = ExecutionStore::new();

        let schedule = Schedule::new(
            "daily-cleanup",
            "Daily Cleanup",
            TriggerConfig::Interval {
                interval_secs: 3600,
            },
            "cleanup-wf",
        );
        store.register(schedule.clone());

        let exec_id = executor.trigger_execution(&schedule, &mut store, &mut exec_store);
        assert!(exec_id.starts_with("exec-daily-cleanup-"));
        assert_eq!(exec_store.len(), 1);

        // Verify last_fire updated.
        let s = store.get("daily-cleanup").unwrap();
        assert!(s.last_fire.is_some());

        // Verify request/response summaries captured.
        let record = exec_store.get(&exec_id).unwrap();
        assert_eq!(record.status, ExecutionStatus::Completed);
        assert!(record.request_summary.get("workflow_id").is_some());
        assert!(record.response_summary.get("status").is_some());
        assert!(record.error_message.is_none());
    }

    #[test]
    fn test_executor_history() {
        let mut executor = ScheduleExecutor::new();
        let mut store = ScheduleStore::new();
        let mut exec_store = ExecutionStore::new();

        let schedule = Schedule::new(
            "test",
            "Test",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf",
        );
        store.register(schedule.clone());

        executor.trigger_execution(&schedule, &mut store, &mut exec_store);
        executor.trigger_execution(&schedule, &mut store, &mut exec_store);

        let history = exec_store.get_history("test");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].status, ExecutionStatus::Completed);
    }

    #[test]
    fn test_execution_store_retention() {
        let mut exec_store = ExecutionStore {
            records: Vec::new(),
            max_per_schedule: 2,
        };

        for i in 0..5 {
            exec_store.record(ScheduleExecution {
                execution_id: format!("exec-{i}"),
                schedule_id: "s1".to_string(),
                triggered_at: Utc::now(),
                started_at: None,
                completed_at: None,
                status: ExecutionStatus::Completed,
                workflow_execution_id: None,
                request_summary: serde_json::json!({}),
                response_summary: serde_json::json!({}),
                error_message: None,
            });
        }

        // Should retain only the 2 most recent.
        assert_eq!(exec_store.len(), 2);
    }

    #[test]
    fn test_execution_store_list_recent() {
        let mut exec_store = ExecutionStore::new();

        for i in 0..5 {
            exec_store.record(ScheduleExecution {
                execution_id: format!("exec-{i}"),
                schedule_id: format!("s{}", i % 2),
                triggered_at: Utc::now(),
                started_at: None,
                completed_at: None,
                status: ExecutionStatus::Completed,
                workflow_execution_id: None,
                request_summary: serde_json::json!({}),
                response_summary: serde_json::json!({}),
                error_message: None,
            });
        }

        let recent = exec_store.list_recent(3);
        assert_eq!(recent.len(), 3);
    }
}
