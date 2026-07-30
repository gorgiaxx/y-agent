//! Shared provider adapter for workflow task executors that invoke an LLM.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use y_agent::orchestrator::checkpoint::TaskOutput;
use y_agent::orchestrator::task_executor::TaskExecuteError;
use y_core::provider::{ChatRequest, ChatResponse, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::types::{Message, Role};

use crate::ServiceContainer;

#[async_trait]
pub(super) trait WorkflowLlmClient: Send + Sync {
    async fn complete(
        &self,
        messages: Vec<Message>,
        provider_tag: Option<String>,
    ) -> Result<ChatResponse, String>;
}

pub(super) struct ServiceWorkflowLlmClient {
    container: Arc<ServiceContainer>,
}

impl ServiceWorkflowLlmClient {
    pub(super) fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait]
impl WorkflowLlmClient for ServiceWorkflowLlmClient {
    async fn complete(
        &self,
        messages: Vec<Message>,
        provider_tag: Option<String>,
    ) -> Result<ChatResponse, String> {
        let request = ChatRequest {
            messages,
            model: None,
            request_mode: y_core::provider::RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };
        let route = RouteRequest {
            required_tags: provider_tag.into_iter().collect(),
            ..RouteRequest::default()
        };
        let pool = self.container.provider_pool().await;
        pool.chat_completion(&request, &route)
            .await
            .map_err(|error| error.to_string())
    }
}

pub(super) async fn execute_workflow_llm(
    client: &dyn WorkflowLlmClient,
    task_id: &str,
    messages: Vec<Message>,
    provider_tag: Option<String>,
    failure_context: &str,
    executor_name: &str,
) -> Result<TaskOutput, TaskExecuteError> {
    let response = client
        .complete(messages, provider_tag)
        .await
        .map_err(|message| TaskExecuteError::Transient {
            message: format!("{failure_context}: {message}"),
        })?;
    let content = response.content.unwrap_or_default();

    debug!(
        task_id,
        content_len = content.len(),
        executor_name,
        "workflow LLM executor completed"
    );

    Ok(TaskOutput {
        task_id: task_id.to_string(),
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

pub(super) fn make_message(role: Role, content: &str) -> Message {
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

pub(super) fn serialize_inputs(inputs: &HashMap<String, serde_json::Value>) -> String {
    let ordered = inputs.iter().collect::<BTreeMap<_, _>>();
    serde_json::to_string_pretty(&ordered)
        .unwrap_or_else(|error| format!("failed to serialize workflow inputs as JSON: {error}"))
}
