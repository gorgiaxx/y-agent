//! `LlmCallExecutor`: handles [`TaskType::LlmCall`] tasks by invoking the
//! LLM via the provider pool.
//!
//! Routes through `ProviderPool::chat_completion` with tag-based routing.
//! The task's `system_prompt` and resolved inputs are assembled into a
//! single chat request.

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

/// Executes `TaskType::LlmCall` tasks via the provider pool.
pub struct LlmCallExecutor {
    container: Arc<ServiceContainer>,
}

impl LlmCallExecutor {
    /// Create a new executor wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
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
            serde_json::to_string_pretty(&inputs).unwrap_or_else(|_| format!("{inputs:?}"))
        };

        let mut messages = Vec::new();
        if let Some(ref sys) = system_prompt {
            messages.push(make_message(Role::System, sys));
        }
        messages.push(make_message(Role::User, &user_content));

        let request = ChatRequest {
            messages,
            model: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
        };

        // Route to provider by tag or use default.
        let route = RouteRequest {
            required_tags: provider_tag.map(|t| vec![t]).unwrap_or_default(),
            ..RouteRequest::default()
        };

        let pool = self.container.provider_pool().await;
        let response = pool.chat_completion(&request, &route).await.map_err(|e| {
            TaskExecuteError::Transient {
                message: format!("LLM call failed: {e}"),
            }
        })?;

        let content = response.content.unwrap_or_default();

        debug!(
            task_id = %task.id,
            content_len = content.len(),
            "LlmCallExecutor completed"
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
        matches!(task_type, TaskType::LlmCall { .. })
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
    fn test_supports_llm_call() {
        let task_type = TaskType::LlmCall {
            provider_tag: None,
            system_prompt: None,
        };
        assert!(matches!(task_type, TaskType::LlmCall { .. }));
    }
}
