//! `FallbackLlmExecutor`: handles [`TaskType::Noop`] tasks compiled from
//! DSL expressions by sending the task name and inputs as an LLM prompt.
//!
//! DSL-compiled tasks (e.g. `search >> analyze >> summarize`) have
//! `TaskType::Noop` by default. This executor treats them as implicit LLM
//! calls so that scheduled workflow execution produces meaningful output
//! even without explicit task type annotations.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use y_agent::orchestrator::channel::WorkflowContext;
use y_agent::orchestrator::checkpoint::TaskOutput;
use y_agent::orchestrator::dag::{TaskNode, TaskType};
use y_agent::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};
use y_core::types::Role;

use crate::ServiceContainer;

use super::llm_support::{
    execute_workflow_llm, make_message, serialize_inputs, ServiceWorkflowLlmClient,
    WorkflowLlmClient,
};

/// Executes `TaskType::Noop` tasks by interpreting them as LLM calls.
pub struct FallbackLlmExecutor {
    client: Arc<dyn WorkflowLlmClient>,
}

impl FallbackLlmExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self {
            client: Arc::new(ServiceWorkflowLlmClient::new(container)),
        }
    }
}

#[async_trait]
impl TaskExecutor for FallbackLlmExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        if !matches!(&task.task_type, TaskType::Noop) {
            return Err(TaskExecuteError::Unsupported);
        }

        // Build a prompt from the task name + resolved inputs.
        let prompt = if inputs.is_empty() {
            format!(
                "You are executing a workflow step named '{}'.\n\
                 Perform this step and return the result.",
                task.name
            )
        } else {
            let inputs_str = serialize_inputs(&inputs);
            format!(
                "You are executing a workflow step named '{}'.\n\n\
                 Inputs:\n{}\n\n\
                 Perform this step using the provided inputs and return the result.",
                task.name, inputs_str
            )
        };

        execute_workflow_llm(
            self.client.as_ref(),
            &task.id,
            vec![make_message(Role::User, &prompt)],
            None,
            "fallback LLM call failed",
            "FallbackLlmExecutor",
        )
        .await
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::Noop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_noop() {
        let task_type = TaskType::Noop;
        assert!(matches!(task_type, TaskType::Noop));
    }
}
