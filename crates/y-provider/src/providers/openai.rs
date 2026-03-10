//! OpenAI-compatible provider backend.
//!
//! Supports OpenAI API and any compatible endpoints (e.g., Azure OpenAI,
//! vLLM, LiteLLM) via configurable base URL.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStream, FinishReason, LlmProvider, ProviderError,
    ProviderMetadata, ProviderType,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

/// OpenAI-compatible LLM provider.
#[derive(Debug)]
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    metadata: ProviderMetadata,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider.
    pub fn new(
        id: &str,
        model: &str,
        api_key: String,
        base_url: Option<String>,
        tags: Vec<String>,
        max_concurrency: usize,
        context_window: usize,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());

        Self {
            client: Client::new(),
            api_key,
            base_url,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::OpenAi,
                model: model.to_string(),
                tags,
                max_concurrency,
                context_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
            },
        }
    }

    /// Build the full API URL for a given endpoint.
    fn api_url(&self, endpoint: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let model = request
            .model
            .as_deref()
            .unwrap_or(&self.metadata.model);

        let openai_messages: Vec<OpenAiMessage> = request
            .messages
            .iter()
            .map(|m| OpenAiMessage {
                role: match m.role {
                    y_core::types::Role::User => "user".to_string(),
                    y_core::types::Role::Assistant => "assistant".to_string(),
                    y_core::types::Role::System => "system".to_string(),
                    y_core::types::Role::Tool => "tool".to_string(),
                },
                content: Some(m.content.clone()),
                tool_call_id: m.tool_call_id.clone(),
                tool_calls: None,
            })
            .collect();

        let body = OpenAiRequest {
            model: model.to_string(),
            messages: openai_messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            stream: false,
            tools: if request.tools.is_empty() {
                None
            } else {
                // Tools are already serde_json::Value in y-core, pass through directly.
                Some(request.tools.clone())
            },
            stop: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        let response = self
            .client
            .post(self.api_url("chat/completions"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError {
                message: e.to_string(),
            })?;

        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(60u64);

            return Err(ProviderError::RateLimited {
                provider: self.metadata.id.to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ProviderError::AuthenticationFailed {
                provider: self.metadata.id.to_string(),
            });
        }

        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        let openai_response: OpenAiResponse =
            response
                .json()
                .await
                .map_err(|e| ProviderError::Other {
                    message: format!("parse response: {e}"),
                })?;

        let choice = openai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Other {
                message: "no choices in response".into(),
            })?;

        let content = choice.message.content;
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCallRequest {
                id: tc.id,
                name: tc.function.name,
                arguments: serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::Value::String(tc.function.arguments)),
            })
            .collect();

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("length") => FinishReason::Length,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        };

        let usage = openai_response.usage.unwrap_or(OpenAiUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
        });

        Ok(ChatResponse {
            id: openai_response.id,
            model: openai_response.model,
            content,
            tool_calls,
            usage: TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason,
        })
    }

    async fn chat_completion_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<ChatStream, ProviderError> {
        // Streaming implementation deferred — requires SSE parsing.
        Err(ProviderError::Other {
            message: "streaming not yet implemented for OpenAI provider".into(),
        })
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }
}

// ---------------------------------------------------------------------------
// OpenAI API types (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    id: String,
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_provider_metadata() {
        let provider = OpenAiProvider::new(
            "test-openai",
            "gpt-4o",
            "sk-test".into(),
            None,
            vec!["reasoning".into(), "general".into()],
            5,
            128_000,
        );

        let meta = provider.metadata();
        assert_eq!(meta.id, ProviderId::from_string("test-openai"));
        assert_eq!(meta.model, "gpt-4o");
        assert_eq!(meta.tags, vec!["reasoning", "general"]);
        assert_eq!(meta.max_concurrency, 5);
    }

    #[test]
    fn test_openai_api_url_construction() {
        let provider = OpenAiProvider::new(
            "test",
            "gpt-4",
            "sk-test".into(),
            None,
            vec![],
            5,
            128_000,
        );

        assert_eq!(
            provider.api_url("chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn test_openai_custom_base_url() {
        let provider = OpenAiProvider::new(
            "test",
            "gpt-4",
            "sk-test".into(),
            Some("http://localhost:8080/v1".into()),
            vec![],
            5,
            128_000,
        );

        assert_eq!(
            provider.api_url("chat/completions"),
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn test_openai_request_serialization() {
        let req = OpenAiRequest {
            model: "gpt-4o".into(),
            messages: vec![OpenAiMessage {
                role: "user".into(),
                content: Some("Hello".into()),
                tool_call_id: None,
                tool_calls: None,
            }],
            max_tokens: Some(100),
            temperature: Some(0.7),
            stream: false,
            tools: None,
            stop: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
        assert!(!json["stream"].as_bool().unwrap());
        // tools and stop should be absent (skip_serializing_if)
        assert!(json.get("tools").is_none());
        assert!(json.get("stop").is_none());
    }

    #[test]
    fn test_openai_response_deserialization() {
        let json = serde_json::json!({
            "id": "chatcmpl-123",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5
            }
        });

        let response: OpenAiResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.id, "chatcmpl-123");
        assert_eq!(response.choices.len(), 1);
        assert_eq!(
            response.choices[0].message.content,
            Some("Hello!".into())
        );
        assert_eq!(response.usage.unwrap().prompt_tokens, 10);
    }
}
