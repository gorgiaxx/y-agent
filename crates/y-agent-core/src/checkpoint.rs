//! Checkpoint manager: persists workflow state for recovery.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::dag::TaskId;

/// Snapshot of a channel's state at checkpoint time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSnapshot {
    pub name: String,
    pub value: serde_json::Value,
    pub version: u64,
}

/// Output from a completed task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutput {
    pub task_id: TaskId,
    pub output: serde_json::Value,
    pub completed_at: DateTime<Utc>,
}

/// A workflow checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    /// Workflow execution ID.
    pub execution_id: String,
    /// Committed channel state.
    pub committed_channels: HashMap<String, ChannelSnapshot>,
    /// Committed task outputs.
    pub committed_tasks: HashMap<TaskId, TaskOutput>,
    /// Pending channel writes (not yet committed).
    pub pending_channel_writes: Vec<(String, serde_json::Value)>,
    /// Pending task outputs.
    pub pending_task_outputs: Vec<TaskOutput>,
    /// Step number.
    pub step_number: u64,
    /// Checkpoint timestamp.
    pub checkpoint_time: DateTime<Utc>,
}

/// In-memory checkpoint store.
pub struct CheckpointStore {
    checkpoints: HashMap<String, Vec<WorkflowCheckpoint>>,
}

impl CheckpointStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            checkpoints: HashMap::new(),
        }
    }

    /// Save a checkpoint.
    pub fn save(&mut self, checkpoint: WorkflowCheckpoint) {
        self.checkpoints
            .entry(checkpoint.execution_id.clone())
            .or_default()
            .push(checkpoint);
    }

    /// Load the latest checkpoint for an execution.
    pub fn load_latest(&self, execution_id: &str) -> Option<&WorkflowCheckpoint> {
        self.checkpoints
            .get(execution_id)
            .and_then(|cps| cps.last())
    }

    /// Count checkpoints for an execution.
    pub fn checkpoint_count(&self, execution_id: &str) -> usize {
        self.checkpoints.get(execution_id).map_or(0, Vec::len)
    }

    /// Create a checkpoint from the current workflow state.
    pub fn create_checkpoint(
        execution_id: &str,
        step: u64,
        committed_channels: HashMap<String, ChannelSnapshot>,
        committed_tasks: HashMap<TaskId, TaskOutput>,
    ) -> WorkflowCheckpoint {
        WorkflowCheckpoint {
            execution_id: execution_id.to_string(),
            committed_channels,
            committed_tasks,
            pending_channel_writes: Vec::new(),
            pending_task_outputs: Vec::new(),
            step_number: step,
            checkpoint_time: Utc::now(),
        }
    }
}

impl Default for CheckpointStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_save_and_load() {
        let mut store = CheckpointStore::new();
        let cp = CheckpointStore::create_checkpoint("exec-1", 1, HashMap::new(), HashMap::new());
        store.save(cp);

        assert!(store.load_latest("exec-1").is_some());
        assert_eq!(store.checkpoint_count("exec-1"), 1);
        assert!(store.load_latest("nonexistent").is_none());
    }

    #[test]
    fn test_multiple_checkpoints() {
        let mut store = CheckpointStore::new();
        let cp1 = CheckpointStore::create_checkpoint("exec-1", 1, HashMap::new(), HashMap::new());
        let cp2 = CheckpointStore::create_checkpoint("exec-1", 2, HashMap::new(), HashMap::new());
        store.save(cp1);
        store.save(cp2);

        let latest = store.load_latest("exec-1").unwrap();
        assert_eq!(latest.step_number, 2);
        assert_eq!(store.checkpoint_count("exec-1"), 2);
    }

    #[test]
    fn test_checkpoint_with_data() {
        let mut channels = HashMap::new();
        channels.insert(
            "result".to_string(),
            ChannelSnapshot {
                name: "result".into(),
                value: serde_json::json!("hello"),
                version: 1,
            },
        );

        let mut tasks = HashMap::new();
        tasks.insert(
            "task-1".to_string(),
            TaskOutput {
                task_id: "task-1".into(),
                output: serde_json::json!(42),
                completed_at: Utc::now(),
            },
        );

        let cp = CheckpointStore::create_checkpoint("exec-1", 1, channels, tasks);
        assert_eq!(cp.committed_channels.len(), 1);
        assert_eq!(cp.committed_tasks.len(), 1);
    }
}
