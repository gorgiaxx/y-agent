//! Failing executor for testing retry and failure strategies.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use crate::orchestrator::channel::WorkflowContext;
use crate::orchestrator::checkpoint::TaskOutput;
use crate::orchestrator::dag::{TaskNode, TaskType};
use crate::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};

/// Executor that fails a configurable number of times before succeeding.
///
/// Used for testing retry logic and failure strategy enforcement.
#[derive(Debug)]
pub struct FailingExecutor {
    /// Number of times to fail before succeeding.
    pub fail_count: u32,
    /// Whether failures are transient (retryable) or permanent.
    pub transient: bool,
    /// Current attempt counter (shared across all tasks).
    attempts: Arc<AtomicU32>,
}

impl FailingExecutor {
    /// Create an executor that fails `fail_count` times, then succeeds.
    pub fn new(fail_count: u32, transient: bool) -> Self {
        Self {
            fail_count,
            transient,
            attempts: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Create an executor that always fails.
    pub fn always_fail(transient: bool) -> Self {
        Self {
            fail_count: u32::MAX,
            transient,
            attempts: Arc::new(AtomicU32::new(0)),
        }
    }

    /// How many attempts have been made.
    pub fn attempt_count(&self) -> u32 {
        self.attempts.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl TaskExecutor for FailingExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        let attempt = self.attempts.fetch_add(1, Ordering::Relaxed);

        if attempt < self.fail_count {
            if self.transient {
                return Err(TaskExecuteError::Transient {
                    message: format!("transient failure {}/{}", attempt + 1, self.fail_count),
                });
            }
            return Err(TaskExecuteError::Permanent {
                message: format!("permanent failure {}/{}", attempt + 1, self.fail_count),
            });
        }

        Ok(TaskOutput {
            task_id: task.id.clone(),
            output: serde_json::to_value(&inputs).unwrap_or_default(),
            completed_at: chrono::Utc::now(),
        })
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::Noop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_failing_executor_succeeds_after_n_failures() {
        let executor = FailingExecutor::new(2, true);
        let task = TaskNode {
            id: "t".into(),
            name: "T".into(),
            task_type: TaskType::Noop,
            ..TaskNode::default()
        };
        let ctx = WorkflowContext::new();

        // First 2 calls fail
        assert!(executor.execute(&task, HashMap::new(), &ctx).await.is_err());
        assert!(executor.execute(&task, HashMap::new(), &ctx).await.is_err());
        // Third call succeeds
        assert!(executor.execute(&task, HashMap::new(), &ctx).await.is_ok());
        assert_eq!(executor.attempt_count(), 3);
    }

    #[tokio::test]
    async fn test_always_fail_executor() {
        let executor = FailingExecutor::always_fail(false);
        let task = TaskNode {
            id: "t".into(),
            name: "T".into(),
            task_type: TaskType::Noop,
            ..TaskNode::default()
        };
        let ctx = WorkflowContext::new();

        for _ in 0..5 {
            let err = executor
                .execute(&task, HashMap::new(), &ctx)
                .await
                .unwrap_err();
            assert!(!err.is_transient());
        }
    }
}
