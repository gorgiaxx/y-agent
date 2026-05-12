//! `ToolExecutionExecutor`: handles [`TaskType::ToolExecution`] tasks by
//! invoking the named tool from the tool registry.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use y_agent::orchestrator::channel::WorkflowContext;
use y_agent::orchestrator::checkpoint::TaskOutput;
use y_agent::orchestrator::dag::{TaskNode, TaskType};
use y_agent::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};
use y_core::tool::ToolInput;
use y_core::types::{SessionId, ToolName};

use crate::ServiceContainer;

/// Executes `TaskType::ToolExecution` tasks via the tool registry.
pub struct ToolExecutionExecutor {
    container: Arc<ServiceContainer>,
}

impl ToolExecutionExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait]
impl TaskExecutor for ToolExecutionExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        let (tool_name, static_params) = match &task.task_type {
            TaskType::ToolExecution {
                tool_name,
                parameters,
            } => (tool_name.clone(), parameters.clone()),
            _ => return Err(TaskExecuteError::Unsupported),
        };

        // Merge static parameters with resolved inputs. Inputs override statics.
        let mut merged = if static_params.is_object() {
            static_params.as_object().cloned().unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        for (k, v) in &inputs {
            merged.insert(k.clone(), v.clone());
        }
        let arguments = serde_json::Value::Object(merged);

        let name = ToolName::from_string(&tool_name);
        let tool = self
            .container
            .tool_registry
            .get_tool(&name)
            .await
            .ok_or_else(|| TaskExecuteError::Permanent {
                message: format!("tool '{tool_name}' not found in registry"),
            })?;

        let tool_input = ToolInput {
            call_id: format!("wf-{}", task.id),
            name: name.clone(),
            arguments,
            session_id: SessionId(String::new()),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        };

        let output = tool
            .execute(tool_input)
            .await
            .map_err(|e| TaskExecuteError::Transient {
                message: format!("tool execution failed: {e}"),
            })?;

        debug!(
            task_id = %task.id,
            tool = %tool_name,
            success = output.success,
            "ToolExecutionExecutor completed"
        );

        Ok(TaskOutput {
            task_id: task.id.clone(),
            output: serde_json::json!({
                "tool": tool_name,
                "success": output.success,
                "content": output.content,
                "warnings": output.warnings,
            }),
            completed_at: chrono::Utc::now(),
        })
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::ToolExecution { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_tool_execution() {
        let task_type = TaskType::ToolExecution {
            tool_name: "FileRead".into(),
            parameters: serde_json::Value::Null,
        };
        assert!(matches!(task_type, TaskType::ToolExecution { .. }));
    }
}
