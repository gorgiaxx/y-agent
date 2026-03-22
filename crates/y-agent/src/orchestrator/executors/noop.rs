//! No-op executor for testing and placeholder tasks.
//!
//! Handles `TaskType::Noop` by echoing inputs as outputs.
//! Useful for testing the DAG execution flow without real LLM/tool calls.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::orchestrator::channel::WorkflowContext;
use crate::orchestrator::checkpoint::TaskOutput;
use crate::orchestrator::dag::{TaskNode, TaskType};
use crate::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};

/// Executor that handles `Noop` tasks by echoing inputs as outputs.
#[derive(Debug, Default)]
pub struct NoopExecutor;

impl NoopExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl TaskExecutor for NoopExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
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
    async fn test_noop_echoes_inputs() {
        let executor = NoopExecutor::new();
        let task = TaskNode {
            id: "test".into(),
            name: "Test".into(),
            task_type: TaskType::Noop,
            ..TaskNode::default()
        };

        let mut inputs = HashMap::new();
        inputs.insert("key".into(), serde_json::json!("value"));

        let ctx = WorkflowContext::new();
        let output = executor.execute(&task, inputs, &ctx).await.unwrap();
        assert_eq!(output.task_id, "test");
        assert_eq!(output.output["key"], "value");
    }

    #[test]
    fn test_noop_supports_only_noop() {
        let executor = NoopExecutor::new();
        assert!(executor.supports(&TaskType::Noop));
        assert!(!executor.supports(&TaskType::HumanApproval));
        assert!(!executor.supports(&TaskType::LlmCall {
            provider_tag: None,
            system_prompt: None,
        }));
    }
}
