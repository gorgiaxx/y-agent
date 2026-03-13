//! Azure `OpenAI` provider backend.
//!
//! Implements the Azure `OpenAI` Service API format with:
//! - Azure-specific endpoint format: `{resource}.openai.azure.com/openai/deployments/{deployment}`
//! - `api-key` header authentication (Azure API key)
//! - `api-version` query parameter
//! - Same request/response format as `OpenAI` (reuses `OpenAI` wire types)
//! - SSE streaming support

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, LlmProvider,
    ProviderError, ProviderMetadata, ProviderType,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

const DEFAULT_API_VERSION: &str = "2024-10-21";

/// Azure `OpenAI` Service provider.
///
/// Uses Azure-specific endpoint format and authentication but the same
/// `OpenAI` request/response wire format internally.
#[derive(Debug)]
pub struct AzureOpenAiProvider {
    client: Client,
    api_key: String,
    /// Full Azure endpoint URL including deployment.
    /// Format: `https://{resource}.openai.azure.com/openai/deployments/{deployment}`
    endpoint: String,
    /// Azure API version query parameter.
    api_version: String,
    metadata: ProviderMetadata,
}

impl AzureOpenAiProvider {
    /// Create a new Azure `OpenAI` provider.
    ///
    /// `base_url` should be the full Azure endpoint including the deployment, e.g.:
    /// `https://myresource.openai.azure.com/openai/deployments/gpt-4o`
    pub fn new(
        id: &str,
        model: &str,
        api_key: String,
        base_url: Option<String>,
        proxy_url: Option<String>,
        tags: Vec<String>,
        max_concurrency: usize,
        context_window: usize,
    ) -> Self {
        let endpoint = base_url.unwrap_or_default();

        let mut builder = Client::builder();
        if let Some(ref proxy) = proxy_url {
            if let Ok(p) = reqwest::Proxy::all(proxy) {
                builder = builder.proxy(p);
            }
        }

        Self {
            client: builder.build().unwrap_or_else(|_| Client::new()),
            api_key,
            endpoint,
            api_version: DEFAULT_API_VERSION.to_string(),
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::Azure,
                model: model.to_string(),
                tags,
                max_concurrency,
                context_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
            },
        }
    }

    /// Build the chat completions URL with api-version query parameter.
    fn chat_url(&self) -> String {
        format!(
            "{}/chat/completions?api-version={}",
            self.endpoint.trim_end_matches('/'),
            self.api_version
        )
    }

    /// Build Azure/OpenAI message list from a `ChatRequest`.
    fn build_messages(request: &ChatRequest) -> Vec<AzureMessage> {
        request
            .messages
            .iter()
            .map(|m| AzureMessage {
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
            .collect()
    }

    /// Build the request body (same format as `OpenAI`).
    fn build_request_body(&self, request: &ChatRequest, stream: bool) -> AzureRequest {
        use y_core::provider::ToolCallingMode;

        // PromptBased mode: never send tool definitions to the provider.
        let tools = match request.tool_calling_mode {
            ToolCallingMode::PromptBased => None,
            ToolCallingMode::Native => {
                if request.tools.is_empty() {
                    None
                } else {
                    Some(request.tools.clone())
                }
            }
        };

        AzureRequest {
            messages: Self::build_messages(request),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stream,
            stream_options: if stream {
                Some(StreamOptions {
                    include_usage: true,
                })
            } else {
                None
            },
            tools,
            stop: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        }
    }
}

#[async_trait]
impl LlmProvider for AzureOpenAiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let response = self
            .client
            .post(self.chat_url())
            .header("api-key", &self.api_key)
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

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
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

        let response_text = response.text().await.map_err(|e| ProviderError::Other {
            message: format!("read response body: {e}"),
        })?;
        let raw_response: serde_json::Value =
            serde_json::from_str(&response_text).map_err(|e| ProviderError::Other {
                message: format!("parse response JSON: {e}"),
            })?;

        let azure_response: AzureResponse =
            serde_json::from_value(raw_response.clone()).map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        let choice =
            azure_response
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

        let usage = azure_response.usage.unwrap_or(AzureUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
        });

        Ok(ChatResponse {
            id: azure_response.id,
            model: azure_response
                .model
                .unwrap_or_else(|| self.metadata.model.clone()),
            content,
            tool_calls,
            usage: TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason,
            raw_request,
            raw_response: Some(raw_response),
            provider_id: None,
        })
    }

    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        let body = self.build_request_body(request, true);
        let raw_request = serde_json::to_value(&body).ok();

        let response = self
            .client
            .post(self.chat_url())
            .header("api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError {
                message: e.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            // Extract headers before consuming the response body.
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);
            let error_body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited {
                    provider: self.metadata.id.to_string(),
                    retry_after_secs: retry_after,
                });
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(ProviderError::AuthenticationFailed {
                    provider: self.metadata.id.to_string(),
                });
            }
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        // Parse SSE stream — same format as OpenAI.
        let byte_stream = response.bytes_stream();
        let provider_id = self.metadata.id.to_string();

        let stream = futures::stream::unfold(
            AzureSseState {
                byte_stream: Box::pin(byte_stream),
                buffer: String::new(),
                tool_calls_acc: Vec::new(),
                done: false,
            },
            move |mut state| {
                let _provider_id = provider_id.clone();
                async move {
                    if state.done {
                        return None;
                    }

                    loop {
                        if let Some(event) = extract_sse_event(&mut state.buffer) {
                            let trimmed = event.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if trimmed == "[DONE]" {
                                state.done = true;
                                return None;
                            }

                            match serde_json::from_str::<AzureStreamChunk>(trimmed) {
                                Ok(chunk) => {
                                    let mapped =
                                        map_stream_chunk(&chunk, &mut state.tool_calls_acc);
                                    return Some((Ok(mapped), state));
                                }
                                Err(e) => {
                                    return Some((
                                        Err(ProviderError::ParseError {
                                            message: format!(
                                                "Azure SSE parse error: {e}, data: {trimmed}"
                                            ),
                                        }),
                                        state,
                                    ));
                                }
                            }
                        }

                        match state.byte_stream.next().await {
                            Some(Ok(bytes)) => match std::str::from_utf8(&bytes) {
                                Ok(text) => state.buffer.push_str(text),
                                Err(e) => {
                                    state.done = true;
                                    return Some((
                                        Err(ProviderError::ParseError {
                                            message: format!("invalid UTF-8 in SSE stream: {e}"),
                                        }),
                                        state,
                                    ));
                                }
                            },
                            Some(Err(e)) => {
                                state.done = true;
                                return Some((
                                    Err(ProviderError::NetworkError {
                                        message: format!("stream read error: {e}"),
                                    }),
                                    state,
                                ));
                            }
                            None => {
                                state.done = true;
                                return None;
                            }
                        }
                    }
                }
            },
        );

        Ok(ChatStreamResponse {
            stream: Box::pin(stream),
            raw_request,
            provider_id: None,
            model: String::new(),
            context_window: 0,
        })
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }
}

// ---------------------------------------------------------------------------
// SSE parsing helpers (same structure as OpenAI)
// ---------------------------------------------------------------------------

struct AzureSseState {
    byte_stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buffer: String,
    tool_calls_acc: Vec<ToolCallAccumulator>,
    done: bool,
}

#[derive(Debug, Clone)]
struct ToolCallAccumulator {
    _index: usize,
    id: String,
    name: String,
    arguments: String,
}

/// Extract one SSE `data:` payload from the buffer.
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    let boundary = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))?;

    let raw_event: String = buffer.drain(..boundary).collect();
    while buffer.starts_with('\n') || buffer.starts_with('\r') {
        buffer.remove(0);
    }

    let mut data_parts = Vec::new();
    for line in raw_event.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            data_parts.push(data.trim().to_string());
        }
    }

    if data_parts.is_empty() {
        return Some(String::new());
    }

    Some(data_parts.join("\n"))
}

/// Map an Azure/OpenAI streaming chunk to `ChatStreamChunk` with incremental
/// `tool_calls` assembly.
fn map_stream_chunk(
    chunk: &AzureStreamChunk,
    tool_calls_acc: &mut Vec<ToolCallAccumulator>,
) -> ChatStreamChunk {
    let choice = chunk.choices.first();
    let delta_content = choice.and_then(|c| c.delta.content.clone());

    let mut delta_tool_calls = Vec::new();
    if let Some(choice) = choice {
        if let Some(ref tcs) = choice.delta.tool_calls {
            for tc in tcs {
                let idx = tc.index.unwrap_or(0) as usize;
                while tool_calls_acc.len() <= idx {
                    tool_calls_acc.push(ToolCallAccumulator {
                        _index: tool_calls_acc.len(),
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }
                let acc = &mut tool_calls_acc[idx];
                if let Some(ref id) = tc.id {
                    acc.id = id.clone();
                }
                if let Some(ref func) = tc.function {
                    if let Some(ref name) = func.name {
                        acc.name = name.clone();
                    }
                    if let Some(ref args) = func.arguments {
                        acc.arguments.push_str(args);
                    }
                }
            }
        }
    }

    let finish_reason = choice.and_then(|c| {
        c.finish_reason.as_deref().map(|r| match r {
            "stop" => FinishReason::Stop,
            "tool_calls" => FinishReason::ToolUse,
            "length" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        })
    });

    // On finish, emit accumulated tool calls.
    if finish_reason.is_some() {
        for acc in tool_calls_acc.drain(..) {
            if !acc.id.is_empty() {
                delta_tool_calls.push(ToolCallRequest {
                    id: acc.id,
                    name: acc.name,
                    arguments: serde_json::from_str(&acc.arguments)
                        .unwrap_or(serde_json::Value::String(acc.arguments)),
                });
            }
        }
    }

    let usage = chunk.usage.as_ref().map(|u| TokenUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        cache_read_tokens: None,
        cache_write_tokens: None,
    });

    ChatStreamChunk {
        delta_content,
        delta_tool_calls,
        usage,
        finish_reason,
    }
}

// ---------------------------------------------------------------------------
// Azure OpenAI API types (same wire format as OpenAI)
// ---------------------------------------------------------------------------

/// Note: Azure does NOT include `model` in the request body; the model is
/// specified as the deployment name in the URL.
#[derive(Debug, Serialize)]
struct AzureRequest {
    messages: Vec<AzureMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AzureMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<AzureToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AzureToolCall {
    id: String,
    function: AzureToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct AzureToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct AzureResponse {
    id: String,
    model: Option<String>,
    choices: Vec<AzureChoice>,
    usage: Option<AzureUsage>,
}

#[derive(Debug, Deserialize)]
struct AzureChoice {
    message: AzureMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AzureUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// Streaming types
#[derive(Debug, Deserialize)]
struct AzureStreamChunk {
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<AzureStreamChoice>,
    usage: Option<AzureUsage>,
}

#[derive(Debug, Deserialize)]
struct AzureStreamChoice {
    delta: AzureStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AzureStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<AzureStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct AzureStreamToolCall {
    index: Option<u32>,
    id: Option<String>,
    function: Option<AzureStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct AzureStreamToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_provider_metadata() {
        let provider = AzureOpenAiProvider::new(
            "azure-gpt4o",
            "gpt-4o",
            "azure-key-test".into(),
            Some("https://myresource.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec!["reasoning".into(), "general".into()],
            5,
            128_000,
        );

        let meta = provider.metadata();
        assert_eq!(meta.id, ProviderId::from_string("azure-gpt4o"));
        assert_eq!(meta.model, "gpt-4o");
        assert_eq!(meta.provider_type, ProviderType::Azure);
        assert_eq!(meta.tags, vec!["reasoning", "general"]);
    }

    #[test]
    fn test_azure_chat_url() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            Some("https://myresource.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            5,
            128_000,
        );
        assert_eq!(
            provider.chat_url(),
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn test_azure_request_serialization_no_model() {
        let req = AzureRequest {
            messages: vec![AzureMessage {
                role: "user".into(),
                content: Some("Hello".into()),
                tool_call_id: None,
                tool_calls: None,
            }],
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: None,
            stream: false,
            stream_options: None,
            tools: None,
            stop: None,
        };

        let json = serde_json::to_value(&req).expect("serialize");
        // Azure does NOT include `model` in the body.
        assert!(json.get("model").is_none());
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_azure_response_deserialization() {
        let json = serde_json::json!({
            "id": "chatcmpl-azure-123",
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

        let response: AzureResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(response.id, "chatcmpl-azure-123");
        assert_eq!(response.model, Some("gpt-4o".into()));
        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.choices[0].message.content, Some("Hello!".into()));
    }

    #[test]
    fn test_azure_sse_event_extraction() {
        let mut buffer = String::from("data: {\"id\":\"x\",\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n");
        let event = extract_sse_event(&mut buffer);
        assert!(event.is_some());
        let data = event.unwrap();
        assert!(data.contains("Hi"));
    }

    #[test]
    fn test_azure_sse_done_signal() {
        let mut buffer = String::from("data: [DONE]\n\n");
        let event = extract_sse_event(&mut buffer);
        assert!(event.is_some());
        assert_eq!(event.unwrap().trim(), "[DONE]");
    }

    #[test]
    fn test_azure_stream_chunk_deserialization() {
        let json = serde_json::json!({
            "id": "chatcmpl-123",
            "choices": [{
                "delta": {"content": "Hello"},
                "finish_reason": null
            }]
        });

        let chunk: AzureStreamChunk = serde_json::from_value(json).expect("deserialize");
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content, Some("Hello".into()));
    }

    #[test]
    fn test_azure_response_without_model() {
        // Azure sometimes doesn't return model field.
        let json = serde_json::json!({
            "id": "chatcmpl-azure-456",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello from Azure!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 15,
                "completion_tokens": 8
            }
        });

        let response: AzureResponse = serde_json::from_value(json).expect("deserialize");
        assert!(response.model.is_none());
    }
}
