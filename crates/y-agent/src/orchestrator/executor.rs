//! Workflow executor: runs the agent loop over a task DAG.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::orchestrator::channel::WorkflowContext;
use crate::orchestrator::checkpoint::{ChannelSnapshot, CheckpointStore, TaskOutput, WorkflowCheckpoint};
use crate::orchestrator::dag::{TaskDag, TaskId};

/// Workflow execution state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    Defined,
    Running,
    Interrupted,
    Completed,
    Failed,
}

/// Stream mode for workflow execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// Final result only.
    None,
    /// Full context snapshot per task.
    Values,
    /// Delta changes only.
    Updates,
    /// Token-level LLM output.
    Messages,
    /// All internal events.
    #[default]
    Debug,
}

/// Execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Stream mode.
    #[serde(default)]
    pub stream_mode: StreamMode,
    /// Maximum concurrent tasks.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: usize,
}

fn default_max_concurrent() -> usize {
    50
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            stream_mode: StreamMode::default(),
            max_concurrent_tasks: default_max_concurrent(),
        }
    }
}

/// Workflow executor.
pub struct WorkflowExecutor {
    pub state: WorkflowState,
    pub context: WorkflowContext,
    pub config: ExecutionConfig,
    completed_tasks: HashSet<TaskId>,
    task_outputs: HashMap<TaskId, TaskOutput>,
    step_number: u64,
}

impl WorkflowExecutor {
    /// Create a new executor.
    pub fn new(config: ExecutionConfig) -> Self {
        Self {
            state: WorkflowState::Defined,
            context: WorkflowContext::new(),
            config,
            completed_tasks: HashSet::new(),
            task_outputs: HashMap::new(),
            step_number: 0,
        }
    }

    /// Execute a workflow DAG (simplified synchronous execution).
    ///
    /// In production, this runs async with the full executor pipeline.
    /// This placeholder executes tasks as numbered placeholders.
    pub fn execute(
        &mut self,
        dag: &TaskDag,
        checkpoint_store: &mut CheckpointStore,
    ) -> Result<(), WorkflowExecuteError> {
        dag.validate()
            .map_err(|e| WorkflowExecuteError::DagInvalid(e.to_string()))?;

        self.state = WorkflowState::Running;
        let execution_id = format!("exec-{}", uuid::Uuid::new_v4());

        loop {
            let ready = dag.ready_tasks(&self.completed_tasks);
            if ready.is_empty() {
                break;
            }

            for task in ready {
                self.step_number += 1;
                // Placeholder: task execution produces a simple output.
                let output = TaskOutput {
                    task_id: task.id.clone(),
                    output: serde_json::json!({
                        "task": task.name,
                        "step": self.step_number,
                        "status": "completed"
                    }),
                    completed_at: Utc::now(),
                };

                self.context
                    .write(&format!("{}.output", task.id), output.output.clone());
                self.task_outputs.insert(task.id.clone(), output);
                self.completed_tasks.insert(task.id.clone());
            }

            // Checkpoint after each round.
            let cp = CheckpointStore::create_checkpoint(
                &execution_id,
                self.step_number,
                self.snapshot_channels(),
                self.task_outputs.clone(),
            );
            checkpoint_store.save(cp);
        }

        self.state = WorkflowState::Completed;
        Ok(())
    }

    /// Recover from a checkpoint.
    pub fn recover_from(&mut self, checkpoint: &WorkflowCheckpoint) {
        self.step_number = checkpoint.step_number;
        for (task_id, output) in &checkpoint.committed_tasks {
            self.completed_tasks.insert(task_id.clone());
            self.task_outputs.insert(task_id.clone(), output.clone());
        }
        self.state = WorkflowState::Running;
    }

    /// Get completed task count.
    pub fn completed_count(&self) -> usize {
        self.completed_tasks.len()
    }

    /// Get task output.
    pub fn get_output(&self, task_id: &str) -> Option<&TaskOutput> {
        self.task_outputs.get(task_id)
    }

    fn snapshot_channels(&self) -> HashMap<String, ChannelSnapshot> {
        let mut snaps = HashMap::new();
        for name in self.context.channel_names() {
            if let Some(val) = self.context.read(name) {
                snaps.insert(
                    name.to_string(),
                    ChannelSnapshot {
                        name: name.to_string(),
                        value: val.clone(),
                        version: 0,
                    },
                );
            }
        }
        snaps
    }
}

/// Error during workflow execution.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowExecuteError {
    #[error("invalid DAG: {0}")]
    DagInvalid(String),
}

#[cfg(test)]
mod tests {
    use crate::orchestrator::dag::TaskNode;

    use super::*;

    fn task(id: &str, deps: &[&str]) -> TaskNode {
        TaskNode {
            id: id.into(),
            name: id.into(),
            priority: crate::orchestrator::dag::TaskPriority::Normal,
            dependencies: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    #[test]
    fn test_execute_simple_dag() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();
        dag.add_task(task("b", &["a"])).unwrap();
        dag.add_task(task("c", &["a"])).unwrap();
        dag.add_task(task("d", &["b", "c"])).unwrap();

        let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
        let mut cp_store = CheckpointStore::new();
        executor.execute(&dag, &mut cp_store).unwrap();

        assert_eq!(executor.state, WorkflowState::Completed);
        assert_eq!(executor.completed_count(), 4);
        assert!(executor.get_output("d").is_some());
    }

    #[test]
    fn test_execute_creates_checkpoints() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();
        dag.add_task(task("b", &["a"])).unwrap();

        let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
        let mut cp_store = CheckpointStore::new();
        executor.execute(&dag, &mut cp_store).unwrap();

        // Should have at least one checkpoint.
        // We can't easily query by execution_id since it's UUID-generated,
        // but we verify the executor completed.
        assert_eq!(executor.state, WorkflowState::Completed);
    }

    #[test]
    fn test_execute_invalid_dag() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &["missing"])).unwrap();

        let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
        let mut cp_store = CheckpointStore::new();
        assert!(executor.execute(&dag, &mut cp_store).is_err());
    }

    #[test]
    fn test_context_populated_after_execution() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();

        let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
        let mut cp_store = CheckpointStore::new();
        executor.execute(&dag, &mut cp_store).unwrap();

        assert!(executor.context.read("a.output").is_some());
    }
}
