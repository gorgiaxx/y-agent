//! Task executor trait: type-dispatched async task execution.
//!
//! Design reference: orchestrator-design.md, Execution Engine
//!
//! Each `TaskType` variant is handled by a concrete executor that implements
//! this trait. The workflow executor dispatches tasks to the appropriate
//! executor based on `TaskExecutor::supports()`.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::orchestrator::channel::WorkflowContext;
use crate::orchestrator::checkpoint::TaskOutput;
use crate::orchestrator::dag::{TaskNode, TaskType};

/// Error from task execution.
#[derive(Debug, thiserror::Error)]
pub enum TaskExecuteError {
    /// The task type is not supported by this executor.
    #[error("unsupported task type for executor")]
    Unsupported,

    /// A transient error that may be retried.
    #[error("transient error: {message}")]
    Transient { message: String },

    /// A permanent error that should not be retried.
    #[error("permanent error: {message}")]
    Permanent { message: String },

    /// Task was cancelled.
    #[error("task cancelled")]
    Cancelled,

    /// Task timed out.
    #[error("task timed out after {elapsed_ms}ms")]
    Timeout { elapsed_ms: u64 },
}

impl TaskExecuteError {
    /// Whether this error is transient (eligible for retry).
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient { .. } | Self::Timeout { .. })
    }
}

/// Async trait for executing a specific type of workflow task.
///
/// Implementations handle one or more `TaskType` variants. The workflow
/// executor calls `supports()` to find the right executor, then `execute()`
/// to run the task.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute a task with the given resolved inputs.
    ///
    /// The executor should:
    /// 1. Read any needed state from `ctx`
    /// 2. Perform the actual work (LLM call, tool invocation, etc.)
    /// 3. Return a `TaskOutput` with the result
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError>;

    /// Whether this executor can handle the given task type.
    fn supports(&self, task_type: &TaskType) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that transient errors are classified correctly.
    #[test]
    fn test_transient_error_classification() {
        let transient = TaskExecuteError::Transient {
            message: "connection reset".into(),
        };
        assert!(transient.is_transient());

        let timeout = TaskExecuteError::Timeout { elapsed_ms: 30000 };
        assert!(timeout.is_transient());

        let permanent = TaskExecuteError::Permanent {
            message: "invalid config".into(),
        };
        assert!(!permanent.is_transient());

        let cancelled = TaskExecuteError::Cancelled;
        assert!(!cancelled.is_transient());

        let unsupported = TaskExecuteError::Unsupported;
        assert!(!unsupported.is_transient());
    }
}
