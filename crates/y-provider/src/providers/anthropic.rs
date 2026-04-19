//! Anthropic Messages API provider backend.
//!
//! Implements the Anthropic Messages API format with:
//! - Separated system message (not part of the messages array)
//! - Content blocks for structured responses
//! - `Authorization: Bearer` header authentication
//! - Streaming support via SSE (event-based format)

use async_trait::async_trait;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, GeneratedImage,
    ImageContentDelta, LlmProvider, ProviderError, ProviderMetadata, ProviderType, ToolCallingMode,
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
        tool_calling_mode: ToolCallingMode,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| ANTHROPIC_API_URL.to_string());

        let mut builder = Client::builder();
        if let Some(proxy) = proxy_url {
            if let Ok(p) = reqwest::Proxy::all(&proxy) {
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
                tool_calling_mode,
            },
        }
    }

    /// Build the full API URL for a given endpoint.
    fn api_url(&self, endpoint: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint)
    }

    /// Extract the system message from the request as an array of content blocks
    /// with `cache_control` for prompt caching.
    fn extract_system(request: &ChatRequest) -> Option<Vec<AnthropicSystemContent>> {
        request
            .messages
            .iter()
            .find(|m| m.role == y_core::types::Role::System)
            .map(|m| {
                vec![AnthropicSystemContent {
                    content_type: "text".to_string(),
                    text: m.content.clone(),
                    cache_control: Some(AnthropicCacheControl {
                        cache_type: "ephemeral".to_string(),
                    }),
                }]
            })
    }

    /// Build Anthropic messages from a `ChatRequest` (excluding system messages).
    fn build_messages(request: &ChatRequest) -> Vec<AnthropicMessage> {
        request
            .messages
            .iter()
            .filter(|m| m.role != y_core::types::Role::System)
            .map(|m| {
                let role = match m.role {
                    y_core::types::Role::User | y_core::types::Role::Tool => "user",
                    y_core::types::Role::Assistant => "assistant",
                    y_core::types::Role::System => unreachable!(),
                };

                // If this is a tool result, format it as a tool_result content block.
                if m.role == y_core::types::Role::Tool {
                    if let Some(ref tool_call_id) = m.tool_call_id {
                        return AnthropicMessage {
                            role: role.to_string(),
                            content: AnthropicContent(vec![AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content: m.content.clone(),
                            }]),
                        };
                    }
                }

                if m.role == y_core::types::Role::Assistant && !m.tool_calls.is_empty() {
                    let mut blocks = Vec::with_capacity(1 + m.tool_calls.len());
                    if !m.content.is_empty() {
                        blocks.push(AnthropicContentBlock::Text {
                            text: m.content.clone(),
                        });
                    }
                    blocks.extend(m.tool_calls.iter().map(|tool_call| {
                        AnthropicContentBlock::ToolUse {
                            id: tool_call.id.clone(),
                            name: tool_call.name.clone(),
                            input: tool_call.arguments.clone(),
                        }
                    }));
                    return AnthropicMessage {
                        role: role.to_string(),
                        content: AnthropicContent(blocks),
                    };
                }

                // Check for image attachments in metadata (multimodal).
                if m.role == y_core::types::Role::User {
                    if let Some(attachments) = m.metadata.get("attachments") {
                        if let Some(arr) = attachments.as_array() {
                            if !arr.is_empty() {
                                let mut blocks: Vec<AnthropicContentBlock> = Vec::new();
                                for att in arr {
                                    if let (Some(mime), Some(data)) = (
                                        att.get("mime_type").and_then(|v| v.as_str()),
                                        att.get("base64_data").and_then(|v| v.as_str()),
                                    ) {
                                        blocks.push(AnthropicContentBlock::Image {
                                            source: AnthropicImageSource {
                                                r#type: "base64".to_string(),
                                                media_type: mime.to_string(),
                                                data: data.to_string(),
                                            },
                                        });
                                    }
                                }
                                if !m.content.is_empty() {
                                    blocks.push(AnthropicContentBlock::Text {
                                        text: m.content.clone(),
                                    });
                                }
                                return AnthropicMessage {
                                    role: role.to_string(),
                                    content: AnthropicContent(blocks),
                                };
                            }
                        }
                    }
                }

                AnthropicMessage {
                    role: role.to_string(),
                    content: AnthropicContent(vec![AnthropicContentBlock::Text {
                        text: m.content.clone(),
                    }]),
                }
            })
            .collect()
    }

    /// Build the Anthropic request body.
    fn build_request_body(&self, request: &ChatRequest, stream: bool) -> AnthropicRequest {
        use y_core::provider::{ThinkingEffort, ToolCallingMode};

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
                                input_schema: {
                                    let mut schema = func.get("parameters").cloned().unwrap_or(
                                        serde_json::json!({"type": "object", "properties": {}}),
                                    );
                                    if let Some(obj) = schema.as_object_mut() {
                                        obj.insert(
                                            "$schema".to_string(),
                                            serde_json::json!(
                                                "https://json-schema.org/draft/2020-12/schema"
                                            ),
                                        );
                                    }
                                    schema
                                },
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

        // Map unified thinking config to Anthropic adaptive thinking.
        let (thinking, mut output_config, temperature) = if let Some(ref tc) = request.thinking {
            let effort_str = match tc.effort {
                ThinkingEffort::Low => "low",
                ThinkingEffort::Medium => "medium",
                ThinkingEffort::High => "high",
                ThinkingEffort::Max => "max",
            };
            (
                Some(AnthropicThinking {
                    thinking_type: "adaptive".to_string(),
                }),
                Some(AnthropicOutputConfig {
                    effort: Some(effort_str.to_string()),
                    format: None,
                }),
                // Anthropic requires temperature to be unset (or 1.0) when
                // thinking is enabled.
                None,
            )
        } else {
            (None, None, request.temperature)
        };

        // Map response format to Anthropic output_config.format.
        if let Some(ref rf) = request.response_format {
            use y_core::provider::ResponseFormat;
            let fmt = match rf {
                ResponseFormat::Text => Some(AnthropicOutputFormat::Text),
                ResponseFormat::JsonObject => {
                    // Anthropic does not have a separate json_object mode;
                    // use json_schema with a permissive schema.
                    None
                }
                ResponseFormat::JsonSchema { name, schema } => {
                    Some(AnthropicOutputFormat::JsonSchema {
                        name: name.clone(),
                        schema: schema.clone(),
                    })
                }
            };
            if let Some(fmt) = fmt {
                if let Some(ref mut oc) = output_config {
                    oc.format = Some(fmt);
                } else {
                    output_config = Some(AnthropicOutputConfig {
                        effort: None,
                        format: Some(fmt),
                    });
                }
            }
        }

        AnthropicRequest {
            model: model.to_string(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(32000),
            temperature,
            stream,
            tools,
            stop_sequences: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
            thinking,
            output_config,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self
            .client
            .post(self.api_url("messages"))
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response =
            request_builder
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
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::AuthenticationFailed {
                provider: self.metadata.id.to_string(),
                message: error_body,
            });
        }

        // 402 Payment Required / billing error.
        if status == reqwest::StatusCode::PAYMENT_REQUIRED {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::QuotaExhausted {
                provider: self.metadata.id.to_string(),
                message: error_body,
            });
        }

        // 529 Overloaded (Anthropic-specific) -- treat as transient server error.
        if status.as_u16() == 529 {
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: "API temporarily overloaded (529)".to_string(),
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

        let anthropic_response: AnthropicResponse = serde_json::from_value(raw_response.clone())
            .map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        // Extract text content, thinking content, and tool calls from content blocks.
        let mut text_parts = Vec::new();
        let mut thinking_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut generated_images = Vec::new();

        for block in &anthropic_response.content {
            match block {
                AnthropicContentBlock::Text { text } => {
                    text_parts.push(text.clone());
                }
                AnthropicContentBlock::Thinking { thinking, .. } => {
                    thinking_parts.push(thinking.clone());
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
                AnthropicContentBlock::Image { source } => {
                    generated_images.push(GeneratedImage {
                        index: generated_images.len(),
                        mime_type: source.media_type.clone(),
                        data: source.data.clone(),
                    });
                }
                AnthropicContentBlock::ToolResult { .. } => {}
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let reasoning_content = if thinking_parts.is_empty() {
            None
        } else {
            Some(thinking_parts.join(""))
        };

        let finish_reason = match anthropic_response.stop_reason.as_deref() {
            Some("tool_use") => FinishReason::ToolUse,
            Some("max_tokens") => FinishReason::Length,
            _ => FinishReason::Stop,
        };

        Ok(ChatResponse {
            id: anthropic_response.id,
            model: anthropic_response.model,
            content,
            reasoning_content,
            tool_calls,
            usage: TokenUsage {
                input_tokens: anthropic_response.usage.input_tokens,
                output_tokens: anthropic_response.usage.output_tokens,
                cache_read_tokens: anthropic_response.usage.cache_read_input_tokens,
                cache_write_tokens: anthropic_response.usage.cache_creation_input_tokens,
                ..Default::default()
            },
            finish_reason,
            raw_request,
            raw_response: Some(raw_response),
            provider_id: None,
            generated_images,
        })
    }

    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        let body = self.build_request_body(request, true);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self
            .client
            .post(self.api_url("messages"))
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response =
            request_builder
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
                    message: error_body,
                });
            }
            if status == reqwest::StatusCode::PAYMENT_REQUIRED {
                return Err(ProviderError::QuotaExhausted {
                    provider: self.metadata.id.to_string(),
                    message: error_body,
                });
            }
            if status.as_u16() == 529 {
                return Err(ProviderError::ServerError {
                    provider: self.metadata.id.to_string(),
                    message: "API temporarily overloaded (529)".to_string(),
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
                sse: crate::sse::SseStreamState::new(Box::pin(byte_stream)),
                current_tool_id: None,
                current_tool_name: None,
                current_tool_args: String::new(),
                current_thinking: String::new(),
                accumulated_usage: None,
                image_index: 0,
            },
            move |mut state| async move {
                if state.sse.done {
                    return None;
                }

                loop {
                    if let Some(event) = extract_anthropic_sse_event(&mut state.sse.buffer) {
                        match event {
                            AnthropicSseEvent::ContentBlockDelta { delta } => match delta {
                                AnthropicDelta::Text { text } => {
                                    return Some((
                                        Ok(ChatStreamChunk {
                                            delta_content: Some(text),
                                            delta_reasoning_content: None,
                                            delta_tool_calls: vec![],
                                            usage: None,
                                            finish_reason: None,
                                            delta_images: vec![],
                                        }),
                                        state,
                                    ));
                                }
                                AnthropicDelta::Thinking { thinking } => {
                                    state.current_thinking.push_str(&thinking);
                                    return Some((
                                        Ok(ChatStreamChunk {
                                            delta_content: None,
                                            delta_reasoning_content: Some(thinking),
                                            delta_tool_calls: vec![],
                                            usage: None,
                                            finish_reason: None,
                                            delta_images: vec![],
                                        }),
                                        state,
                                    ));
                                }
                                AnthropicDelta::InputJson { partial_json } => {
                                    state.current_tool_args.push_str(&partial_json);
                                    continue;
                                }
                                // Signature deltas are accumulated silently.
                                AnthropicDelta::Signature { .. } => {
                                    continue;
                                }
                            },
                            AnthropicSseEvent::ContentBlockStart { content_block } => {
                                if let Some(ref block) = content_block {
                                    match block {
                                        AnthropicContentBlock::ToolUse { id, name, .. } => {
                                            state.current_tool_id = Some(id.clone());
                                            state.current_tool_name = Some(name.clone());
                                            state.current_tool_args.clear();
                                        }
                                        AnthropicContentBlock::Thinking { .. } => {
                                            state.current_thinking.clear();
                                        }
                                        AnthropicContentBlock::Image { source } => {
                                            let idx = state.image_index;
                                            state.image_index += 1;
                                            return Some((
                                                Ok(ChatStreamChunk {
                                                    delta_content: None,
                                                    delta_reasoning_content: None,
                                                    delta_tool_calls: vec![],
                                                    usage: None,
                                                    finish_reason: None,
                                                    delta_images: vec![ImageContentDelta {
                                                        index: idx,
                                                        mime_type: source.media_type.clone(),
                                                        partial_data: source.data.clone(),
                                                        is_complete: true,
                                                    }],
                                                }),
                                                state,
                                            ));
                                        }
                                        _ => {}
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
                                            delta_images: vec![],
                                        }),
                                        state,
                                    ));
                                }
                                continue;
                            }
                            AnthropicSseEvent::MessageStart { usage, .. } => {
                                // Capture initial usage from message_start (Anthropic
                                // reports input_tokens only here, not in message_delta).
                                if let Some(u) = usage {
                                    state.accumulated_usage = Some(TokenUsage {
                                        input_tokens: u.input_tokens.unwrap_or(0),
                                        output_tokens: u.output_tokens.unwrap_or(0),
                                        cache_read_tokens: u.cache_read_input_tokens,
                                        cache_write_tokens: u.cache_creation_input_tokens,
                                        ..Default::default()
                                    });
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
                                // Merge message_delta usage with accumulated
                                // message_start usage.
                                let usage_info = if let Some(u) = usage {
                                    let mut merged =
                                        state.accumulated_usage.take().unwrap_or(TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: 0,
                                            cache_read_tokens: None,
                                            cache_write_tokens: None,
                                            ..Default::default()
                                        });
                                    // output_tokens is typically in message_delta.
                                    if let Some(out) = u.output_tokens {
                                        merged.output_tokens = out;
                                    }
                                    // input_tokens may be present in message_delta
                                    // on some proxies.
                                    if let Some(inp) = u.input_tokens {
                                        if inp > 0 {
                                            merged.input_tokens = inp;
                                        }
                                    }
                                    Some(merged)
                                } else {
                                    state.accumulated_usage.take()
                                };
                                return Some((
                                    Ok(ChatStreamChunk {
                                        delta_content: None,
                                        delta_reasoning_content: None,
                                        delta_tool_calls: vec![],
                                        usage: usage_info,
                                        finish_reason,
                                        delta_images: vec![],
                                    }),
                                    state,
                                ));
                            }
                            AnthropicSseEvent::MessageStop => {
                                state.sse.done = true;
                                return None;
                            }
                            AnthropicSseEvent::Ping
                            | AnthropicSseEvent::Error { .. }
                            | AnthropicSseEvent::Unknown => {
                                continue;
                            }
                        }
                    }

                    // Need more data.
                    match state.sse.read_next().await {
                        Ok(true) => {} // Data appended to buffer, loop again.
                        Ok(false) => {
                            // Stream ended.
                            state.sse.done = true;
                            return None;
                        }
                        Err(e) => {
                            return Some((Err(e), state));
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
    /// Shared SSE byte-stream decoder.
    sse: crate::sse::SseStreamState,
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    current_tool_args: String,
    current_thinking: String,
    /// Usage accumulated from `message_start` event.
    accumulated_usage: Option<TokenUsage>,
    /// Running index for generated images.
    image_index: usize,
}

/// Parsed Anthropic SSE event types.
enum AnthropicSseEvent {
    Ping,
    MessageStart {
        #[allow(dead_code)]
        message: Option<serde_json::Value>,
        usage: Option<AnthropicStreamUsage>,
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
    cache_read_input_tokens: Option<u32>,
    cache_creation_input_tokens: Option<u32>,
}

enum AnthropicDelta {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    Signature {
        #[allow(dead_code)]
        signature: String,
    },
    InputJson {
        partial_json: String,
    },
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
            let parsed = serde_json::from_str::<serde_json::Value>(&data).ok();
            let msg = parsed.as_ref().and_then(|v| v.get("message").cloned());
            let usage = parsed
                .as_ref()
                .and_then(|v| v.pointer("/message/usage"))
                .and_then(|u| serde_json::from_value::<AnthropicStreamUsage>(u.clone()).ok());
            Some(AnthropicSseEvent::MessageStart {
                message: msg,
                usage,
            })
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
                        delta: AnthropicDelta::Text { text },
                    })
                }
                "thinking_delta" => {
                    let thinking = delta_obj
                        .get("thinking")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(AnthropicSseEvent::ContentBlockDelta {
                        delta: AnthropicDelta::Thinking { thinking },
                    })
                }
                "signature_delta" => {
                    let signature = delta_obj
                        .get("signature")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(AnthropicSseEvent::ContentBlockDelta {
                        delta: AnthropicDelta::Signature { signature },
                    })
                }
                "input_json_delta" => {
                    let partial = delta_obj
                        .get("partial_json")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(AnthropicSseEvent::ContentBlockDelta {
                        delta: AnthropicDelta::InputJson {
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
    system: Option<Vec<AnthropicSystemContent>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
}

#[derive(Debug, Serialize)]
struct AnthropicThinking {
    #[serde(rename = "type")]
    thinking_type: String,
}

#[derive(Debug, Serialize)]
struct AnthropicOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<AnthropicOutputFormat>,
}

/// Structured output format for Anthropic's `output_config.format`.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicOutputFormat {
    /// Plain text (default).
    Text,
    /// JSON conforming to a specific schema.
    JsonSchema {
        /// Schema name.
        name: String,
        /// JSON Schema definition.
        schema: serde_json::Value,
    },
}

/// A content block in the `system` array, supporting `cache_control` for
/// prompt caching.
#[derive(Debug, Serialize)]
struct AnthropicSystemContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicCacheControl>,
}

/// Cache control directive for Anthropic system content blocks.
#[derive(Debug, Serialize)]
struct AnthropicCacheControl {
    #[serde(rename = "type")]
    cache_type: String,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

/// Anthropic message content is always an array of content blocks.
#[derive(Debug, Serialize)]
struct AnthropicContent(Vec<AnthropicContentBlock>);

/// A single content block in an Anthropic response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
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
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
}

/// Base64-encoded image source for Anthropic's vision API.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicImageSource {
    r#type: String,
    media_type: String,
    data: String,
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
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
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
            ToolCallingMode::default(),
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
            ToolCallingMode::default(),
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
            ToolCallingMode::default(),
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
                content: AnthropicContent(vec![AnthropicContentBlock::Text {
                    text: "Hello".into(),
                }]),
            }],
            system: Some(vec![AnthropicSystemContent {
                content_type: "text".to_string(),
                text: "You are a helpful assistant.".to_string(),
                cache_control: Some(AnthropicCacheControl {
                    cache_type: "ephemeral".to_string(),
                }),
            }]),
            max_tokens: 4096,
            temperature: Some(0.7),
            stream: false,
            tools: None,
            stop_sequences: None,
            thinking: None,
            output_config: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "claude-3-5-sonnet-20241022");
        let system = json["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are a helpful assistant.");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(json["max_tokens"], 4096);
        assert!(!json["stream"].as_bool().unwrap());
        // top_p should not be present.
        assert!(json.get("top_p").is_none());
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
                    message_id: String::new(),
                    role: Role::System,
                    content: "You are helpful.".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: String::new(),
                    role: Role::User,
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
            thinking: None,
            response_format: None,
        };

        let system = AnthropicProvider::extract_system(&request);
        assert!(system.is_some());
        let system_content = system.unwrap();
        assert_eq!(system_content.len(), 1);
        assert_eq!(system_content[0].text, "You are helpful.");
        assert!(system_content[0].cache_control.is_some());
        assert_eq!(
            system_content[0].cache_control.as_ref().unwrap().cache_type,
            "ephemeral"
        );

        let messages = AnthropicProvider::build_messages(&request);
        assert_eq!(messages.len(), 1); // System excluded.
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn test_build_messages_restores_assistant_tool_use_before_tool_result() {
        use y_core::types::{Message, Role, ToolCallRequest};

        let tool_call = ToolCallRequest {
            id: "call_123".into(),
            name: "ShellExec".into(),
            arguments: serde_json::json!({ "command": "uname -a" }),
        };
        let request = ChatRequest {
            messages: vec![
                Message {
                    message_id: String::new(),
                    role: Role::User,
                    content: "Check system info".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: String::new(),
                    role: Role::Assistant,
                    content: "I'll inspect the machine.\n".into(),
                    tool_call_id: None,
                    tool_calls: vec![tool_call.clone()],
                    timestamp: chrono::Utc::now(),
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: String::new(),
                    role: Role::Tool,
                    content: "{\"stdout\":\"Darwin\"}".into(),
                    tool_call_id: Some(tool_call.id.clone()),
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
            thinking: None,
            response_format: None,
        };

        let messages = AnthropicProvider::build_messages(&request);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[2].role, "user");

        match &messages[1].content {
            AnthropicContent(blocks) => {
                assert_eq!(blocks.len(), 2);
                assert!(matches!(
                    &blocks[0],
                    AnthropicContentBlock::Text { text } if text == "I'll inspect the machine.\n"
                ));
                assert!(matches!(
                    &blocks[1],
                    AnthropicContentBlock::ToolUse { id, name, input }
                        if id == "call_123"
                            && name == "ShellExec"
                            && input == &serde_json::json!({ "command": "uname -a" })
                ));
            }
        }

        match &messages[2].content {
            AnthropicContent(blocks) => {
                assert_eq!(blocks.len(), 1);
                assert!(matches!(
                    &blocks[0],
                    AnthropicContentBlock::ToolResult { tool_use_id, content }
                        if tool_use_id == "call_123" && content == "{\"stdout\":\"Darwin\"}"
                ));
            }
        }
    }

    #[test]
    fn test_extract_anthropic_sse_text_delta() {
        let mut buf = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(event.is_some());
        if let Some(AnthropicSseEvent::ContentBlockDelta {
            delta: AnthropicDelta::Text { text },
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
                "$schema": "https://json-schema.org/draft/2020-12/schema",
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
        assert_eq!(
            json["input_schema"]["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }

    #[test]
    fn test_anthropic_response_with_thinking() {
        let json = serde_json::json!({
            "id": "msg_03",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5-20250929",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "Let me think about this...",
                    "signature": "sig_abc123"
                },
                {"type": "text", "text": "Here is my answer."}
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 50,
                "output_tokens": 100,
                "cache_read_input_tokens": 20,
                "cache_creation_input_tokens": 10
            }
        });

        let response: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.content.len(), 2);

        // Verify thinking block is parsed correctly.
        if let AnthropicContentBlock::Thinking {
            thinking,
            signature,
        } = &response.content[0]
        {
            assert_eq!(thinking, "Let me think about this...");
            assert_eq!(signature.as_deref(), Some("sig_abc123"));
        } else {
            panic!("Expected Thinking block");
        }

        // Verify text block.
        if let AnthropicContentBlock::Text { text } = &response.content[1] {
            assert_eq!(text, "Here is my answer.");
        } else {
            panic!("Expected Text block");
        }
    }

    #[test]
    fn test_anthropic_usage_with_cache_tokens() {
        let json = serde_json::json!({
            "id": "msg_04",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [{"type": "text", "text": "cached response"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 80,
                "cache_creation_input_tokens": 15
            }
        });

        let response: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.usage.input_tokens, 100);
        assert_eq!(response.usage.output_tokens, 50);
        assert_eq!(response.usage.cache_read_input_tokens, Some(80));
        assert_eq!(response.usage.cache_creation_input_tokens, Some(15));
    }

    #[test]
    fn test_anthropic_usage_without_cache_tokens() {
        let json = serde_json::json!({
            "id": "msg_05",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-5-sonnet-20241022",
            "content": [{"type": "text", "text": "response"}],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        });

        let response: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(response.usage.cache_read_input_tokens, None);
        assert_eq!(response.usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn test_extract_anthropic_sse_thinking_delta() {
        let mut buf = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me reason...\"}}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(event.is_some());
        if let Some(AnthropicSseEvent::ContentBlockDelta {
            delta: AnthropicDelta::Thinking { thinking },
        }) = event
        {
            assert_eq!(thinking, "Let me reason...");
        } else {
            panic!("Expected ContentBlockDelta with ThinkingDelta");
        }
    }

    #[test]
    fn test_extract_anthropic_sse_signature_delta() {
        let mut buf = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig_xyz\"}}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(event.is_some());
        if let Some(AnthropicSseEvent::ContentBlockDelta {
            delta: AnthropicDelta::Signature { signature },
        }) = event
        {
            assert_eq!(signature, "sig_xyz");
        } else {
            panic!("Expected ContentBlockDelta with SignatureDelta");
        }
    }

    #[test]
    fn test_extract_anthropic_sse_message_start_usage() {
        let mut buf = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-3-5-sonnet\",\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":500,\"output_tokens\":0,\"cache_read_input_tokens\":300,\"cache_creation_input_tokens\":50}}}\n\n".to_string();
        let event = extract_anthropic_sse_event(&mut buf);
        assert!(event.is_some());
        if let Some(AnthropicSseEvent::MessageStart { usage, .. }) = event {
            let u = usage.expect("usage should be present");
            assert_eq!(u.input_tokens, Some(500));
            assert_eq!(u.output_tokens, Some(0));
            assert_eq!(u.cache_read_input_tokens, Some(300));
            assert_eq!(u.cache_creation_input_tokens, Some(50));
        } else {
            panic!("Expected MessageStart event");
        }
    }

    #[test]
    fn test_build_messages_with_image_attachments() {
        use y_core::types::{Message, Role};

        let request = ChatRequest {
            messages: vec![Message {
                message_id: "test-1".into(),
                role: Role::User,
                content: "What is in this image?".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::json!({
                    "attachments": [{
                        "id": "att-1",
                        "filename": "photo.png",
                        "mime_type": "image/png",
                        "base64_data": "iVBORw0KGgo=",
                        "size": 8
                    }]
                }),
            }],
            model: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
        };

        let messages = AnthropicProvider::build_messages(&request);
        assert_eq!(messages.len(), 1);

        let json = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[0]["source"]["data"], "iVBORw0KGgo=");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "What is in this image?");
    }

    #[test]
    fn test_anthropic_request_with_thinking_includes_output_config() {
        use y_core::provider::{ThinkingConfig, ThinkingEffort};
        use y_core::types::{Message, Role};

        let request = ChatRequest {
            messages: vec![Message {
                message_id: "test-1".into(),
                role: Role::User,
                content: "Think carefully".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            }],
            model: None,
            max_tokens: None,
            temperature: Some(0.7),
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: Some(ThinkingConfig {
                effort: ThinkingEffort::Max,
            }),
            response_format: None,
        };

        let provider = AnthropicProvider::new(
            "test-anthropic",
            "claude-3-5-sonnet-20241022",
            "sk-test".into(),
            None,
            None,
            vec![],
            3,
            200_000,
            ToolCallingMode::default(),
        );
        let body = provider.build_request_body(&request, false);
        let json = serde_json::to_value(&body).unwrap();

        // Thinking mode: adaptive + output_config.effort = "max".
        assert_eq!(json["thinking"]["type"], "adaptive");
        assert_eq!(json["output_config"]["effort"], "max");
        // Temperature must be null when thinking is enabled.
        assert!(json["temperature"].is_null());

        // Without thinking: no thinking/output_config fields, temperature preserved.
        let request_no_thinking = ChatRequest {
            thinking: None,
            temperature: Some(0.7),
            ..request.clone()
        };
        let body_no = provider.build_request_body(&request_no_thinking, false);
        let json_no = serde_json::to_value(&body_no).unwrap();
        assert!(json_no["thinking"].is_null());
        assert!(json_no["output_config"].is_null());
        assert_eq!(json_no["temperature"], 0.7);
    }
}
