//! Anthropic Messages API provider backend.
//!
//! Implements the Anthropic Messages API format with:
//! - Separated system message (not part of the messages array)
//! - Content blocks for structured responses
//! - `x-api-key` header authentication
//! - Streaming support via SSE (event-based format)

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

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Anthropic Messages API provider.
#[derive(Debug)]
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    metadata: ProviderMetadata,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
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
        let base_url = base_url.unwrap_or_else(|| ANTHROPIC_API_URL.to_string());

        let mut builder = Client::builder();
        if let Some(ref proxy) = proxy_url {
            if let Ok(p) = reqwest::Proxy::all(proxy) {
                builder = builder.proxy(p);
            }
        }

        Self {
            client: builder.build().unwrap_or_else(|_| Client::new()),
            api_key,
            base_url,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::Anthropic,
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

    /// Extract the system message from the request, if any.
    fn extract_system(request: &ChatRequest) -> Option<String> {
        request
            .messages
            .iter()
            .find(|m| m.role == y_core::types::Role::System)
            .map(|m| m.content.clone())
    }

    /// Build Anthropic messages from a `ChatRequest` (excluding system messages).
    fn build_messages(request: &ChatRequest) -> Vec<AnthropicMessage> {
        request
            .messages
            .iter()
            .filter(|m| m.role != y_core::types::Role::System)
            .map(|m| {
                let role = match m.role {
                    y_core::types::Role::User => "user",
                    y_core::types::Role::Assistant => "assistant",
                    y_core::types::Role::Tool => "user", // Tool results sent as user messages.
                    y_core::types::Role::System => unreachable!(),
                };

                // If this is a tool result, format it as a tool_result content block.
                if m.role == y_core::types::Role::Tool {
                    if let Some(ref tool_call_id) = m.tool_call_id {
                        return AnthropicMessage {
                            role: role.to_string(),
                            content: AnthropicContent::Blocks(vec![
                                AnthropicContentBlock::ToolResult {
                                    tool_use_id: tool_call_id.clone(),
                                    content: m.content.clone(),
                                },
                            ]),
                        };
                    }
                }

                AnthropicMessage {
                    role: role.to_string(),
                    content: AnthropicContent::Text(m.content.clone()),
                }
            })
            .collect()
    }

    /// Build the Anthropic request body.
    fn build_request_body(&self, request: &ChatRequest, stream: bool) -> AnthropicRequest {
        use y_core::provider::ToolCallingMode;

        let model = request.model.as_deref().unwrap_or(&self.metadata.model);
        let system = Self::extract_system(request);
        let messages = Self::build_messages(request);

        // PromptBased mode: never send tool definitions to the provider.
        let tools: Option<Vec<AnthropicToolDef>> = match request.tool_calling_mode {
            ToolCallingMode::PromptBased => None,
            ToolCallingMode::Native => {
                if request.tools.is_empty() {
                    None
                } else {
                    let tools: Vec<AnthropicToolDef> = request
                        .tools
                        .iter()
                        .filter_map(|t| {
                            let func = t.get("function")?;
                            Some(AnthropicToolDef {
                                name: func.get("name")?.as_str()?.to_string(),
                                description: func
                                    .get("description")
                                    .and_then(|d| d.as_str())
                                    .map(String::from),
                                input_schema: func
                                    .get("parameters")
                                    .cloned()
                                    .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
                            })
                        })
                        .collect();
                    if tools.is_empty() {
                        None
                    } else {
                        Some(tools)
                    }
                }
            }
        };

        AnthropicRequest {
            model: model.to_string(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            top_p: request.top_p,
            stream,
            tools,
            stop_sequences: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let response = self
            .client
            .post(self.api_url("messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
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

        let response_text = response.text().await.map_err(|e| ProviderError::Other {
            message: format!("read response body: {e}"),
        })?;
        let raw_response: serde_json::Value =
            serde_json::from_str(&response_text).map_err(|e| ProviderError::Other {
                message: format!("parse response JSON: {e}"),
            })?;

        let anthropic_response: AnthropicResponse =
            serde_json::from_value(raw_response.clone()).map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        // Extract text content and tool calls from content blocks.
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in &anthropic_response.content {
            match block {
                AnthropicContentBlock::Text { text } => {
                    text_parts.push(text.clone());
                }
                AnthropicContentBlock::ToolUse {
                    id, name, input, ..
                } => {
                    tool_calls.push(ToolCallRequest {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: input.clone(),
                    });
                }
                _ => {} // Ignore other block types.
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let finish_reason = match anthropic_response.stop_reason.as_deref() {
            Some("end_turn") => FinishReason::Stop,
            Some("stop_sequence") => FinishReason::Stop,
            Some("tool_use") => FinishReason::ToolUse,
            Some("max_tokens") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        Ok(ChatResponse {
            id: anthropic_response.id,
            model: anthropic_response.model,
            content,
            reasoning_content: None,
            tool_calls,
            usage: TokenUsage {
                input_tokens: anthropic_response.usage.input_tokens,
                output_tokens: anthropic_response.usage.output_tokens,
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
            .post(self.api_url("messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError {
                message: e.to_string(),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited {
                    provider: self.metadata.id.to_string(),
                    retry_after_secs: 60,
                });
            }
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ProviderError::AuthenticationFailed {
                    provider: self.metadata.id.to_string(),
                });
            }
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        let byte_stream = response.bytes_stream();

        let stream = futures::stream::unfold(
            AnthropicSseState {
                byte_stream: Box::pin(byte_stream),
                buffer: String::new(),
                current_tool_id: None,
                current_tool_name: None,
                current_tool_args: String::new(),
                done: false,
            },
            move |mut state| async move {
                if state.done {
                    return None;
                }

                loop {
                    if let Some(event) = extract_anthropic_sse_event(&mut state.buffer) {
                        match event {
                            AnthropicSseEvent::ContentBlockDelta { delta } => match delta {
                                AnthropicDelta::TextDelta { text } => {
                                    return Some((
                                        Ok(ChatStreamChunk {
                                            delta_content: Some(text),
                                            delta_reasoning_content: None,
                                            delta_tool_calls: vec![],
                                            usage: None,
                                            finish_reason: None,
                                        }),
                                        state,
                                    ));
                                }
                                AnthropicDelta::InputJsonDelta { partial_json } => {
                                    state.current_tool_args.push_str(&partial_json);
                                    continue;
                                }
                            },
                            AnthropicSseEvent::ContentBlockStart { content_block } => {
                                if let Some(block) = content_block {
                                    if let AnthropicContentBlock::ToolUse { id, name, .. } = block {
                                        state.current_tool_id = Some(id);
                                        state.current_tool_name = Some(name);
                                        state.current_tool_args.clear();
                                    }
                                }
                                continue;
                            }
                            AnthropicSseEvent::ContentBlockStop => {
                                // If we were accumulating a tool call, emit it.
                                if let (Some(id), Some(name)) =
                                    (state.current_tool_id.take(), state.current_tool_name.take())
                                {
                                    let args = std::mem::take(&mut state.current_tool_args);
                                    let arguments = serde_json::from_str(&args)
                                        .unwrap_or(serde_json::Value::String(args));
                                    return Some((
                                        Ok(ChatStreamChunk {
                                            delta_content: None,
                                            delta_reasoning_content: None,
                                            delta_tool_calls: vec![ToolCallRequest {
                                                id,
                                                name,
                                                arguments,
                                            }],
                                            usage: None,
                                            finish_reason: None,
                                        }),
                                        state,
                                    ));
                                }
                                continue;
                            }
                            AnthropicSseEvent::MessageDelta { delta, usage } => {
                                let finish_reason =
                                    delta.and_then(|d| d.stop_reason).map(|r| match r.as_str() {
                                        "end_turn" | "stop_sequence" => FinishReason::Stop,
                                        "tool_use" => FinishReason::ToolUse,
                                        "max_tokens" => FinishReason::Length,
                                        _ => FinishReason::Unknown,
                                    });
                                let usage_info = usage.map(|u| TokenUsage {
                                    input_tokens: u.input_tokens.unwrap_or(0),
                                    output_tokens: u.output_tokens.unwrap_or(0),
                                    cache_read_tokens: None,
                                    cache_write_tokens: None,
                                });
                                return Some((
                                    Ok(ChatStreamChunk {
                                        delta_content: None,
                                        delta_reasoning_content: None,
                                        delta_tool_calls: vec![],
                                        usage: usage_info,
                                        finish_reason,
                                    }),
                                    state,
                                ));
                            }
                            AnthropicSseEvent::MessageStop => {
                                state.done = true;
                                return None;
                            }
                            AnthropicSseEvent::Ping
                            | AnthropicSseEvent::MessageStart { .. }
                            | AnthropicSseEvent::Error { .. }
                            | AnthropicSseEvent::Unknown => {
                                continue;
                            }
                        }
                    }

                    // Need more data.
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
// SSE parsing for Anthropic events
// ---------------------------------------------------------------------------

struct AnthropicSseState {
    byte_stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buffer: String,
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    current_tool_args: String,
    done: bool,
}

/// Parsed Anthropic SSE event types.
enum AnthropicSseEvent {
    Ping,
    MessageStart {
        #[allow(dead_code)]
        message: Option<serde_json::Value>,
    },
    ContentBlockStart {
        content_block: Option<AnthropicContentBlock>,
    },
    ContentBlockDelta {
        delta: AnthropicDelta,
    },
    ContentBlockStop,
    MessageDelta {
        delta: Option<AnthropicMessageDelta>,
        usage: Option<AnthropicStreamUsage>,
    },
    MessageStop,
    Error {
        #[allow(dead_code)]
        message: String,
    },
    Unknown,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicStreamUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

enum AnthropicDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

/// Extract one Anthropic SSE event from the buffer.
fn extract_anthropic_sse_event(buffer: &mut String) -> Option<AnthropicSseEvent> {
    // Find event boundary.
    let boundary = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))?;

    let raw_event: String = buffer.drain(..boundary).collect();
    while buffer.starts_with('\n') || buffer.starts_with('\r') {
        buffer.remove(0);
    }

    let mut event_type = String::new();
    let mut data_parts = Vec::new();

    for line in raw_event.lines() {
        let line = line.trim();
        if let Some(et) = line.strip_prefix("event:") {
            event_type = et.trim().to_string();
        } else if let Some(data) = line.strip_prefix("data:") {
            data_parts.push(data.trim().to_string());
        }
    }

    let data = data_parts.join("\n");

    match event_type.as_str() {
        "ping" => Some(AnthropicSseEvent::Ping),
        "message_start" => {
            let msg = serde_json::from_str::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| v.get("message").cloned());
            Some(AnthropicSseEvent::MessageStart { message: msg })
        }
        "content_block_start" => {
            let block = serde_json::from_str::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| {
                    let cb = v.get("content_block")?;
                    serde_json::from_value::<AnthropicContentBlock>(cb.clone()).ok()
                });
            Some(AnthropicSseEvent::ContentBlockStart {
                content_block: block,
            })
        }
        "content_block_delta" => {
            let delta = serde_json::from_str::<serde_json::Value>(&data).ok()?;
            let delta_obj = delta.get("delta")?;
            let delta_type = delta_obj.get("type")?.as_str()?;

            match delta_type {
                "text_delta" => {
                    let text = delta_obj
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(AnthropicSseEvent::ContentBlockDelta {
                        delta: AnthropicDelta::TextDelta { text },
                    })
                }
                "input_json_delta" => {
                    let partial = delta_obj
                        .get("partial_json")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(AnthropicSseEvent::ContentBlockDelta {
                        delta: AnthropicDelta::InputJsonDelta {
                            partial_json: partial,
                        },
                    })
                }
                _ => Some(AnthropicSseEvent::Unknown),
            }
        }
        "content_block_stop" => Some(AnthropicSseEvent::ContentBlockStop),
        "message_delta" => {
            let parsed = serde_json::from_str::<serde_json::Value>(&data).ok();
            let delta = parsed
                .as_ref()
                .and_then(|v| v.get("delta"))
                .and_then(|d| serde_json::from_value::<AnthropicMessageDelta>(d.clone()).ok());
            let usage = parsed
                .as_ref()
                .and_then(|v| v.get("usage"))
                .and_then(|u| serde_json::from_value::<AnthropicStreamUsage>(u.clone()).ok());
            Some(AnthropicSseEvent::MessageDelta { delta, usage })
        }
        "message_stop" => Some(AnthropicSseEvent::MessageStop),
        "error" => {
            let msg = serde_json::from_str::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| v.get("error")?.get("message")?.as_str().map(String::from))
                .unwrap_or_else(|| data.clone());
            Some(AnthropicSseEvent::Error { message: msg })
        }
        _ => {
            if data_parts.is_empty() && event_type.is_empty() {
                // Empty event, skip.
                Some(AnthropicSseEvent::Unknown)
            } else {
                Some(AnthropicSseEvent::Unknown)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Anthropic API types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

/// Anthropic content can be a string or an array of content blocks.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

/// A single content block in an Anthropic response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicToolDef {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::provider::ToolCallingMode;

    #[test]
    fn test_anthropic_provider_metadata() {
        let provider = AnthropicProvider::new(
            "anthropic-main",
            "claude-3-5-sonnet-20241022",
            "sk-ant-test".into(),
            None,
            None,
            vec!["reasoning".into(), "code".into()],
            3,
            200_000,
        );

        let meta = provider.metadata();
        assert_eq!(meta.id, ProviderId::from_string("anthropic-main"));
        assert_eq!(meta.model, "claude-3-5-sonnet-20241022");
        assert_eq!(meta.provider_type, ProviderType::Anthropic);
        assert_eq!(meta.tags, vec!["reasoning", "code"]);
    }

    #[test]
    fn test_anthropic_api_url() {
        let provider = AnthropicProvider::new(
            "test",
            "claude-3",
            "sk-test".into(),
            None,
            None,
            vec![],
            3,
            200_000,
        );
        assert_eq!(
            provider.api_url("messages"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn test_anthropic_custom_base_url() {
        let provider = AnthropicProvider::new(
            "test",
            "claude-3",
            "sk-test".into(),
            Some("http://localhost:8080/v1".into()),
            None,
            vec![],
            3,
            200_000,
        );
        assert_eq!(
            provider.api_url("messages"),
            "http://localhost:8080/v1/messages"
        );
    }

    #[test]
    fn test_anthropic_request_serialization() {
        let req = AnthropicRequest {
            model: "claude-3-5-sonnet-20241022".into(),
            messages: vec![AnthropicMessage {
                role: "user".into(),
                content: AnthropicContent::Text("Hello".into()),
            }],
            system: Some("You are a helpful assistant.".into()),
            max_tokens: 4096,
            temperature: Some(0.7),
            top_p: None,
            stream: false,
            tools: None,
            stop_sequences: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "claude-3-5-sonnet-20241022");
        assert_eq!(json["system"], "You are a helpful assistant.");
        assert_eq!(json["max_tokens"], 4096);
        assert!(!json["stream"].as_bool().unwrap());
    }

    #[test]
    fn test_anthropic_response_deserialization() {
        let json = serde_json::json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {"type": "text", "text": "Hello!"}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        });

        let response: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.id, "msg_01");
        assert_eq!(response.content.len(), 1);
        assert_eq!(response.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(response.usage.input_tokens, 10);
    }

    #[test]
    fn test_anthropic_response_with_tool_use() {
        let json = serde_json::json!({
            "id": "msg_02",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [
                {"type": "text", "text": "I'll check the weather."},
                {
                    "type": "tool_use",
                    "id": "toolu_01",
                    "name": "get_weather",
                    "input": {"city": "Paris"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 30,
                "output_tokens": 20
            }
        });

        let response: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.content.len(), 2);
        assert_eq!(response.stop_reason.as_deref(), Some("tool_use"));

        // Verify tool use block.
        if let AnthropicContentBlock::ToolUse { id, name, input } = &response.content[1] {
            assert_eq!(id, "toolu_01");
            assert_eq!(name, "get_weather");
            assert_eq!(input["city"], "Paris");
        } else {
            panic!("Expected ToolUse block");
        }
    }

    #[test]
    fn test_anthropic_system_extraction() {
        use y_core::types::{Message, Role};

        let request = ChatRequest {
            messages: vec![
                Message {
                    message_id: String::new(),                    role: Role::System,
                    content: "You are helpful.".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: String::new(),                    role: Role::User,
                    content: "Hello".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                },
            ],
            model: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
        };

        let system = AnthropicProvider::extract_system(&request);
        assert_eq!(system, Some("You are helpful.".into()));

        let messages = AnthropicProvider::build_messages(&request);
        assert_eq!(messages.len(), 1); // System excluded.
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn test_extract_anthropic_sse_text_delta() {
        let mut buf = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(event.is_some());
        if let Some(AnthropicSseEvent::ContentBlockDelta {
            delta: AnthropicDelta::TextDelta { text },
        }) = event
        {
            assert_eq!(text, "Hello");
        } else {
            panic!("Expected ContentBlockDelta with TextDelta");
        }
    }

    #[test]
    fn test_extract_anthropic_sse_message_stop() {
        let mut buf = "event: message_stop\ndata: {}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(matches!(event, Some(AnthropicSseEvent::MessageStop)));
    }

    #[test]
    fn test_extract_anthropic_sse_ping() {
        let mut buf = "event: ping\ndata: {}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(matches!(event, Some(AnthropicSseEvent::Ping)));
    }

    #[test]
    fn test_anthropic_tool_def_serialization() {
        let tool = AnthropicToolDef {
            name: "get_weather".into(),
            description: Some("Get the weather".into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                },
                "required": ["city"]
            }),
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["name"], "get_weather");
        assert_eq!(json["input_schema"]["type"], "object");
    }
}
