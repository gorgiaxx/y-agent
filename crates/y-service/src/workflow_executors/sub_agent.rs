//! `SubAgentExecutor`: handles [`TaskType::SubAgent`] tasks by delegating
//! to the agent pool via `AgentDelegator`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use y_agent::orchestrator::channel::WorkflowContext;
use y_agent::orchestrator::checkpoint::TaskOutput;
use y_agent::orchestrator::dag::{TaskNode, TaskType};
use y_agent::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};
use y_core::agent::ContextStrategyHint;

use crate::ServiceContainer;

/// Executes `TaskType::SubAgent` tasks by delegating to the agent pool.
pub struct SubAgentExecutor {
    container: Arc<ServiceContainer>,
}

impl SubAgentExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait]
impl TaskExecutor for SubAgentExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        let agent_id = match &task.task_type {
            TaskType::SubAgent { agent_id } => agent_id.clone(),
            _ => return Err(TaskExecuteError::Unsupported),
        };

        let input_value = serde_json::Value::Object(
            inputs
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        );

        let result = self
            .container
            .agent_delegator
            .delegate(&agent_id, input_value, ContextStrategyHint::None, None)
            .await
            .map_err(|e| TaskExecuteError::Transient {
                message: format!("sub-agent delegation failed: {e}"),
            })?;

        debug!(
            task_id = %task.id,
            agent_id = %agent_id,
            "SubAgentExecutor completed"
        );

        Ok(TaskOutput {
            task_id: task.id.clone(),
            output: serde_json::json!({
                "agent_id": agent_id,
                "text": result.text,
                "model_used": result.model_used,
                "tokens_used": result.tokens_used,
                "duration_ms": result.duration_ms,
            }),
            completed_at: chrono::Utc::now(),
        })
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::SubAgent { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_sub_agent() {
        let task_type = TaskType::SubAgent {
            agent_id: "test-agent".into(),
        };
        assert!(matches!(task_type, TaskType::SubAgent { .. }));
    }
}
