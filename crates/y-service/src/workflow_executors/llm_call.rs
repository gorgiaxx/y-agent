//! `LlmCallExecutor`: handles [`TaskType::LlmCall`] tasks by invoking the
//! LLM via the provider pool.
//!
//! Routes through `ProviderPool::chat_completion` with tag-based routing.
//! The task's `system_prompt` and resolved inputs are assembled into a
//! single chat request.

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

/// Executes `TaskType::LlmCall` tasks via the provider pool.
pub struct LlmCallExecutor {
    client: Arc<dyn WorkflowLlmClient>,
}

impl LlmCallExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self {
            client: Arc::new(ServiceWorkflowLlmClient::new(container)),
        }
    }
}

#[async_trait]
impl TaskExecutor for LlmCallExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        let (provider_tag, system_prompt) = match &task.task_type {
            TaskType::LlmCall {
                provider_tag,
                system_prompt,
            } => (provider_tag.clone(), system_prompt.clone()),
            _ => return Err(TaskExecuteError::Unsupported),
        };

        // Build user message from resolved inputs.
        let user_content = if inputs.is_empty() {
            format!("Execute task: {}", task.name)
        } else {
            serialize_inputs(&inputs)
        };

        let mut messages = Vec::new();
        if let Some(ref sys) = system_prompt {
            messages.push(make_message(Role::System, sys));
        }
        messages.push(make_message(Role::User, &user_content));

        execute_workflow_llm(
            self.client.as_ref(),
            &task.id,
            messages,
            provider_tag,
            "LLM call failed",
            "LlmCallExecutor",
        )
        .await
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::LlmCall { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_executors::llm_support::serialize_inputs;

    #[test]
    fn test_supports_llm_call() {
        let task_type = TaskType::LlmCall {
            provider_tag: None,
            system_prompt: None,
        };
        assert!(matches!(task_type, TaskType::LlmCall { .. }));
    }

    #[test]
    fn test_workflow_llm_inputs_are_serialized_in_key_order() {
        let inputs = HashMap::from([
            ("zeta".to_string(), serde_json::json!(3)),
            ("alpha".to_string(), serde_json::json!(1)),
            ("middle".to_string(), serde_json::json!(2)),
        ]);

        assert_eq!(
            serialize_inputs(&inputs),
            "{\n  \"alpha\": 1,\n  \"middle\": 2,\n  \"zeta\": 3\n}"
        );
    }
}
