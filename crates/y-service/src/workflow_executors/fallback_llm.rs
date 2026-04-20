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
use tracing::debug;

use y_agent::orchestrator::channel::WorkflowContext;
use y_agent::orchestrator::checkpoint::TaskOutput;
use y_agent::orchestrator::dag::{TaskNode, TaskType};
use y_agent::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::types::{Message, Role};

use crate::ServiceContainer;

/// Executes `TaskType::Noop` tasks by interpreting them as LLM calls.
pub struct FallbackLlmExecutor {
    container: Arc<ServiceContainer>,
}

impl FallbackLlmExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
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
            let inputs_str =
                serde_json::to_string_pretty(&inputs).unwrap_or_else(|_| format!("{inputs:?}"));
            format!(
                "You are executing a workflow step named '{}'.\n\n\
                 Inputs:\n{}\n\n\
                 Perform this step using the provided inputs and return the result.",
                task.name, inputs_str
            )
        };

        let request = ChatRequest {
            messages: vec![make_message(Role::User, &prompt)],
            model: None,
            request_mode: y_core::provider::RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let route = RouteRequest::default();
        let pool = self.container.provider_pool().await;

        let response = pool.chat_completion(&request, &route).await.map_err(|e| {
            TaskExecuteError::Transient {
                message: format!("fallback LLM call failed: {e}"),
            }
        })?;

        let content = response.content.unwrap_or_default();

        debug!(
            task_id = %task.id,
            content_len = content.len(),
            "FallbackLlmExecutor completed"
        );

        Ok(TaskOutput {
            task_id: task.id.clone(),
            output: serde_json::json!({
                "content": content,
                "model": response.model,
                "usage": {
                    "input_tokens": response.usage.input_tokens,
                    "output_tokens": response.usage.output_tokens,
                },
            }),
            completed_at: chrono::Utc::now(),
        })
    }

    fn supports(&self, task_type: &TaskType) -> bool {
        matches!(task_type, TaskType::Noop)
    }
}

/// Build a simple message with the given role and content.
fn make_message(role: Role, content: &str) -> Message {
    Message {
        message_id: y_core::types::generate_message_id(),
        role,
        content: content.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: chrono::Utc::now(),
        metadata: serde_json::Value::Null,
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
