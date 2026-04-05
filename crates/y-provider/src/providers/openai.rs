//! OpenAI-compatible provider backend.
//!
//! Supports `OpenAI` API and any compatible endpoints (e.g., Azure `OpenAI`,
//! vLLM, `LiteLLM`) via configurable base URL.

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, LlmProvider,
    ProviderError, ProviderMetadata, ProviderType, ToolCallingMode,
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
    /// Create a new `OpenAI` provider.
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
        let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());

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
                provider_type: ProviderType::OpenAi,
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

    /// Build `OpenAI` message list from a `ChatRequest`.
    fn build_messages(request: &ChatRequest) -> Vec<OpenAiMessage> {
        request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    y_core::types::Role::User => "user".to_string(),
                    y_core::types::Role::Assistant => "assistant".to_string(),
                    y_core::types::Role::System => "system".to_string(),
                    y_core::types::Role::Tool => "tool".to_string(),
                };

                // For assistant messages with tool calls, set content to None
                // and populate the tool_calls array (OpenAI API contract).
                let (content, tool_calls) =
                    if m.role == y_core::types::Role::Assistant && !m.tool_calls.is_empty() {
                        let tcs: Vec<OpenAiToolCall> = m
                            .tool_calls
                            .iter()
                            .map(|tc| OpenAiToolCall {
                                id: tc.id.clone(),
                                r#type: "function".to_string(),
                                function: OpenAiToolCallFunction {
                                    name: tc.name.clone(),
                                    arguments: match &tc.arguments {
                                        serde_json::Value::String(s) => s.clone(),
                                        other => serde_json::to_string(other)
                                            .unwrap_or_else(|_| "{}".to_string()),
                                    },
                                },
                            })
                            .collect();
                        // Content may be empty or present alongside tool calls.
                        let content = if m.content.is_empty() {
                            None
                        } else {
                            Some(OpenAiContent::Text(m.content.clone()))
                        };
                        (content, Some(tcs))
                    } else if m.role == y_core::types::Role::User {
                        // Check for image attachments in metadata (multimodal).
                        let content = if let Some(arr) =
                            m.metadata.get("attachments").and_then(|v| v.as_array())
                        {
                            if arr.is_empty() {
                                Some(OpenAiContent::Text(m.content.clone()))
                            } else {
                                let mut parts: Vec<OpenAiContentPart> = Vec::new();
                                for att in arr {
                                    if let (Some(mime), Some(data)) = (
                                        att.get("mime_type").and_then(|v| v.as_str()),
                                        att.get("base64_data").and_then(|v| v.as_str()),
                                    ) {
                                        parts.push(OpenAiContentPart::ImageUrl {
                                            image_url: OpenAiImageUrl {
                                                url: format!("data:{mime};base64,{data}"),
                                            },
                                        });
                                    }
                                }
                                if !m.content.is_empty() {
                                    parts.push(OpenAiContentPart::Text {
                                        text: m.content.clone(),
                                    });
                                }
                                Some(OpenAiContent::Parts(parts))
                            }
                        } else {
                            Some(OpenAiContent::Text(m.content.clone()))
                        };
                        (content, None)
                    } else {
                        (Some(OpenAiContent::Text(m.content.clone())), None)
                    };

                OpenAiMessage {
                    role,
                    content,
                    reasoning_content: None,
                    tool_call_id: m.tool_call_id.clone(),
                    tool_calls,
                }
            })
            .collect()
    }

    /// Build the `OpenAI` request body.
    fn build_request_body(&self, request: &ChatRequest, stream: bool) -> OpenAiRequest {
        use y_core::provider::ToolCallingMode;

        let model = request.model.as_deref().unwrap_or(&self.metadata.model);

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

        OpenAiRequest {
            model: model.to_string(),
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
            reasoning: request.thinking.as_ref().map(|tc| {
                use y_core::provider::ThinkingEffort;
                OpenAiReasoning {
                    effort: match tc.effort {
                        ThinkingEffort::Low => "low".to_string(),
                        ThinkingEffort::Medium => "medium".to_string(),
                        ThinkingEffort::High | ThinkingEffort::Max => "high".to_string(),
                    },
                }
            }),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self
            .client
            .post(self.api_url("chat/completions"))
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

        let openai_response: OpenAiResponse = serde_json::from_value(raw_response.clone())
            .map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        let choice =
            openai_response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| ProviderError::Other {
                    message: "no choices in response".into(),
                })?;

        let content = choice.message.content.and_then(OpenAiContent::into_text);
        let reasoning_content = choice.message.reasoning_content;
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
            reasoning_content,
            tool_calls,
            usage: TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: None,
                cache_write_tokens: None,
                ..Default::default()
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

        let mut request_builder = self
            .client
            .post(self.api_url("chat/completions"))
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
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            let error_body = response.text().await.unwrap_or_default();
            return Err(if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                ProviderError::RateLimited {
                    provider: self.metadata.id.to_string(),
                    retry_after_secs: retry_after.unwrap_or(60),
                }
            } else if status == reqwest::StatusCode::UNAUTHORIZED {
                ProviderError::AuthenticationFailed {
                    provider: self.metadata.id.to_string(),
                    message: error_body,
                }
            } else {
                ProviderError::ServerError {
                    provider: self.metadata.id.to_string(),
                    message: format!("HTTP {status}: {error_body}"),
                }
            });
        }

        // Parse SSE stream from the response bytes_stream.
        let byte_stream = response.bytes_stream();
        let provider_id = self.metadata.id.to_string();

        let stream = futures::stream::unfold(
            SseState {
                byte_stream: Box::pin(byte_stream),
                buffer: String::new(),
                bytes_remainder: Vec::new(),
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
                        // Try to extract a complete SSE event from the buffer.
                        if let Some(event) = extract_sse_event(&mut state.buffer) {
                            let trimmed = event.trim();
                            if trimmed.is_empty() {
                                continue;
                            }

                            // Check for [DONE] termination signal.
                            if trimmed == "[DONE]" {
                                state.done = true;
                                return None;
                            }

                            // Parse the JSON chunk.
                            match serde_json::from_str::<OpenAiStreamChunk>(trimmed) {
                                Ok(chunk) => {
                                    let mapped =
                                        map_stream_chunk(&chunk, &mut state.tool_calls_acc);
                                    return Some((Ok(mapped), state));
                                }
                                Err(e) => {
                                    return Some((
                                        Err(ProviderError::ParseError {
                                            message: format!(
                                                "SSE JSON parse error: {e}, data: {trimmed}"
                                            ),
                                        }),
                                        state,
                                    ));
                                }
                            }
                        }

                        // Need more data from the network.
                        match state.byte_stream.next().await {
                            Some(Ok(bytes)) => {
                                // Prepend any leftover bytes from a previous incomplete UTF-8 sequence.
                                let combined = if state.bytes_remainder.is_empty() {
                                    bytes.to_vec()
                                } else {
                                    let mut combined = std::mem::take(&mut state.bytes_remainder);
                                    combined.extend_from_slice(&bytes);
                                    combined
                                };
                                match std::str::from_utf8(&combined) {
                                    Ok(text) => state.buffer.push_str(text),
                                    Err(e) => {
                                        // Decode as much valid UTF-8 as possible.
                                        let valid_up_to = e.valid_up_to();
                                        if valid_up_to > 0 {
                                            // Safety: valid_up_to is guaranteed to be a valid UTF-8 boundary.
                                            let valid_text = unsafe {
                                                std::str::from_utf8_unchecked(
                                                    &combined[..valid_up_to],
                                                )
                                            };
                                            state.buffer.push_str(valid_text);
                                        }
                                        // Keep the remaining bytes for the next chunk.
                                        state.bytes_remainder = combined[valid_up_to..].to_vec();
                                    }
                                }
                            }
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
                                // Stream ended without [DONE] — this is acceptable.
                                state.done = true;

                                // Drain any remaining buffer.
                                if let Some(event) = extract_sse_event(&mut state.buffer) {
                                    let trimmed = event.trim();
                                    if !trimmed.is_empty() && trimmed != "[DONE]" {
                                        if let Ok(chunk) =
                                            serde_json::from_str::<OpenAiStreamChunk>(trimmed)
                                        {
                                            let mapped =
                                                map_stream_chunk(&chunk, &mut state.tool_calls_acc);
                                            return Some((Ok(mapped), state));
                                        }
                                    }
                                }
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
// SSE parsing helpers
// ---------------------------------------------------------------------------

/// Internal state for SSE stream parsing.
struct SseState {
    byte_stream:
        std::pin::Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>>,
    buffer: String,
    /// Leftover bytes from the previous chunk that form an incomplete UTF-8 sequence.
    bytes_remainder: Vec<u8>,
    /// Accumulated tool calls for incremental assembly.
    tool_calls_acc: Vec<ToolCallAccumulator>,
    done: bool,
}

/// Accumulates incremental tool call arguments across multiple chunks.
#[derive(Debug, Clone)]
struct ToolCallAccumulator {
    _index: usize,
    id: String,
    name: String,
    arguments: String,
}

/// Extract one SSE event `data:` payload from the buffer.
///
/// SSE events are separated by double newlines. Each event line starts with
/// `data: `. Returns `None` if no complete event is available yet.
fn extract_sse_event(buffer: &mut String) -> Option<String> {
    // Look for double newline (event boundary).
    let boundary = if let Some(pos) = buffer.find("\n\n") {
        pos
    } else if let Some(pos) = buffer.find("\r\n\r\n") {
        pos
    } else {
        return None;
    };

    let raw_event: String = buffer.drain(..boundary).collect();
    // Consume the boundary newlines.
    let trim_count = buffer
        .chars()
        .take_while(|c| *c == '\n' || *c == '\r')
        .count();
    if trim_count > 0 {
        buffer.drain(..trim_count);
    }

    // Extract data from `data: <payload>` lines.
    let mut data_parts = Vec::new();
    for line in raw_event.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            data_parts.push(data.trim().to_string());
        }
        // Ignore other SSE fields (event:, id:, retry:).
    }

    if data_parts.is_empty() {
        // Not a data event — skip.
        return Some(String::new());
    }

    Some(data_parts.join("\n"))
}

/// Map an `OpenAI` streaming chunk to our `ChatStreamChunk`, with incremental
/// `tool_calls` assembly.
fn map_stream_chunk(
    chunk: &OpenAiStreamChunk,
    tool_calls_acc: &mut Vec<ToolCallAccumulator>,
) -> ChatStreamChunk {
    let choice = chunk.choices.first();

    let delta_content = choice.and_then(|c| c.delta.content.clone());
    let delta_reasoning_content = choice.and_then(|c| c.delta.reasoning_content.clone());

    // Handle incremental tool calls.
    let mut delta_tool_calls = Vec::new();
    if let Some(choice) = choice {
        if let Some(ref tcs) = choice.delta.tool_calls {
            for tc in tcs {
                let idx = tc.index.unwrap_or(0) as usize;

                // Find or create accumulator for this index.
                while tool_calls_acc.len() <= idx {
                    tool_calls_acc.push(ToolCallAccumulator {
                        _index: tool_calls_acc.len(),
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }

                let acc = &mut tool_calls_acc[idx];

                // Update with new data.
                if let Some(ref id) = tc.id {
                    acc.id.clone_from(id);
                }
                if let Some(ref func) = tc.function {
                    if let Some(ref name) = func.name {
                        acc.name.clone_from(name);
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
        ..Default::default()
    });

    ChatStreamChunk {
        delta_content,
        delta_reasoning_content,
        delta_tool_calls,
        usage,
        finish_reason,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<OpenAiReasoning>,
}

#[derive(Debug, Serialize)]
struct OpenAiReasoning {
    effort: String,
}

#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    /// Reasoning/thinking content from thinking-mode LLMs (e.g. DeepSeek-R1).
    /// Some providers use `reasoning_content`, others use `reasoning` (vLLM).
    #[serde(skip_serializing_if = "Option::is_none", default, alias = "reasoning")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

/// `OpenAI` content -- either a plain text string or an array of content parts
/// (used for multimodal messages containing text + images).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

impl OpenAiContent {
    /// Extract the text content from this value.
    /// For `Text`, returns the string directly.
    /// For `Parts`, concatenates all text-type parts.
    fn into_text(self) -> Option<String> {
        match self {
            OpenAiContent::Text(s) => {
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            }
            OpenAiContent::Parts(parts) => {
                let texts: Vec<String> = parts
                    .into_iter()
                    .filter_map(|p| match p {
                        OpenAiContentPart::Text { text } => Some(text),
                        OpenAiContentPart::ImageUrl { .. } => None,
                    })
                    .collect();
                if texts.is_empty() {
                    None
                } else {
                    Some(texts.join(""))
                }
            }
        }
    }
}

/// A single content part within an `OpenAI` multimodal message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum OpenAiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAiImageUrl },
}

/// Image URL payload for `OpenAI` vision API. Supports both HTTP URLs
/// and inline data URIs (`data:{mime};base64,{data}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiImageUrl {
    url: String,
}

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[allow(dead_code)]
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
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(default = "default_tool_call_type")]
    r#type: String,
    function: OpenAiToolCallFunction,
}

fn default_tool_call_type() -> String {
    "function".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

// ---------------------------------------------------------------------------
// OpenAI Streaming types
// ---------------------------------------------------------------------------

/// A single chunk from the `OpenAI` streaming API.
#[derive(Debug, Deserialize)]
struct OpenAiStreamChunk {
    #[allow(dead_code)]
    id: Option<String>,
    #[allow(dead_code)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiStreamChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamDelta {
    #[serde(default)]
    content: Option<String>,
    /// Reasoning/thinking content delta.
    /// Some providers use `reasoning_content`, others use `reasoning` (vLLM).
    #[serde(default, alias = "reasoning")]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: Option<u32>,
    id: Option<String>,
    function: Option<OpenAiStreamToolCallFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCallFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::provider::ToolCallingMode;

    #[test]
    fn test_openai_provider_metadata() {
        let provider = OpenAiProvider::new(
            "test-openai",
            "gpt-4o",
            "sk-test".into(),
            None,
            None,
            vec!["reasoning".into(), "general".into()],
            5,
            128_000,
            ToolCallingMode::default(),
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
            None,
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
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
            None,
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
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
                content: Some(OpenAiContent::Text("Hello".into())),
                reasoning_content: None,
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
            reasoning: None,
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
    fn test_openai_request_with_stream_options() {
        let req = OpenAiRequest {
            model: "gpt-4o".into(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            top_p: None,
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: None,
            stop: None,
            reasoning: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert!(json["stream"].as_bool().unwrap());
        assert!(json["stream_options"]["include_usage"].as_bool().unwrap());
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
            response.choices[0]
                .message
                .content
                .clone()
                .and_then(super::OpenAiContent::into_text),
            Some("Hello!".into())
        );
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
    }

    #[test]
    fn test_openai_provider_with_proxy() {
        // Verify that passing a proxy URL does not cause panics.
        let provider = OpenAiProvider::new(
            "proxied",
            "gpt-4",
            "sk-test".into(),
            None,
            Some("socks5://127.0.0.1:1080".into()),
            vec!["general".into()],
            5,
            128_000,
            ToolCallingMode::default(),
        );
        assert_eq!(provider.metadata().id, ProviderId::from_string("proxied"));

        // Verify http proxy also works.
        let provider2 = OpenAiProvider::new(
            "proxied-http",
            "gpt-4",
            "sk-test".into(),
            None,
            Some("http://proxy.example.com:8080".into()),
            vec![],
            3,
            128_000,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider2.metadata().id,
            ProviderId::from_string("proxied-http")
        );
    }

    // -----------------------------------------------------------------------
    // SSE parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_sse_event_simple() {
        let mut buf = "data: {\"id\":\"123\"}\n\n".to_string();
        let event = extract_sse_event(&mut buf).unwrap();
        assert_eq!(event, "{\"id\":\"123\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_extract_sse_event_done() {
        let mut buf = "data: [DONE]\n\n".to_string();
        let event = extract_sse_event(&mut buf).unwrap();
        assert_eq!(event, "[DONE]");
    }

    #[test]
    fn test_extract_sse_event_incomplete() {
        let mut buf = "data: partial".to_string();
        assert!(extract_sse_event(&mut buf).is_none());
    }

    #[test]
    fn test_extract_sse_event_multiple() {
        let mut buf = "data: first\n\ndata: second\n\n".to_string();
        let e1 = extract_sse_event(&mut buf).unwrap();
        assert_eq!(e1, "first");
        let e2 = extract_sse_event(&mut buf).unwrap();
        assert_eq!(e2, "second");
    }

    #[test]
    fn test_stream_chunk_deserialization() {
        let json = r#"{"id":"chatcmpl-abc","model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk: OpenAiStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content, Some("Hello".to_string()));
        assert!(chunk.choices[0].finish_reason.is_none());
    }

    #[test]
    fn test_stream_chunk_with_tool_calls() {
        let json = r#"{"id":"chatcmpl-abc","model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_123","function":{"name":"get_weather","arguments":"{\"city\":"}}]},"finish_reason":null}]}"#;
        let chunk: OpenAiStreamChunk = serde_json::from_str(json).unwrap();
        let tcs = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id.as_deref(), Some("call_123"));
    }

    #[test]
    fn test_stream_chunk_finish_with_usage() {
        let json = r#"{"id":"chatcmpl-abc","model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let chunk: OpenAiStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
        assert!(chunk.usage.is_some());
    }

    /// Regression test: openai-compat providers (e.g. `MiniMax`) may return usage objects
    /// without `prompt_tokens`/`completion_tokens`, using `total_tokens` instead.
    /// The parser must tolerate this and default missing fields to 0.
    #[test]
    fn test_stream_chunk_compat_usage_without_prompt_tokens() {
        let json = r#"{"id":"061029ccbac41519c69f764cc06f5024","choices":[{"index":0,"delta":{"content":"","role":"assistant"},"finish_reason":null}],"created":1774253772,"model":"MiniMax-M2.7-highspeed","object":"chat.completion.chunk","usage":{"total_tokens":0,"total_characters":0}}"#;
        let chunk: OpenAiStreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 0);
        assert_eq!(usage.completion_tokens, 0);
    }

    #[test]
    fn test_map_stream_chunk_content() {
        let chunk = OpenAiStreamChunk {
            id: Some("test".into()),
            model: Some("gpt-4o".into()),
            choices: vec![OpenAiStreamChoice {
                delta: OpenAiStreamDelta {
                    content: Some("Hello".into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        };

        let mut acc = Vec::new();
        let mapped = map_stream_chunk(&chunk, &mut acc);
        assert_eq!(mapped.delta_content, Some("Hello".into()));
        assert!(mapped.delta_tool_calls.is_empty());
        assert!(mapped.finish_reason.is_none());
    }

    #[test]
    fn test_map_stream_chunk_tool_calls_incremental() {
        let mut acc = Vec::new();

        // First chunk: start of tool call.
        let chunk1 = OpenAiStreamChunk {
            id: Some("test".into()),
            model: Some("gpt-4o".into()),
            choices: vec![OpenAiStreamChoice {
                delta: OpenAiStreamDelta {
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![OpenAiStreamToolCall {
                        index: Some(0),
                        id: Some("call_abc".into()),
                        function: Some(OpenAiStreamToolCallFunction {
                            name: Some("get_weather".into()),
                            arguments: Some("{\"ci".into()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let m1 = map_stream_chunk(&chunk1, &mut acc);
        assert!(m1.delta_tool_calls.is_empty()); // Not finished yet.
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].arguments, "{\"ci");

        // Second chunk: continuation of arguments.
        let chunk2 = OpenAiStreamChunk {
            id: Some("test".into()),
            model: Some("gpt-4o".into()),
            choices: vec![OpenAiStreamChoice {
                delta: OpenAiStreamDelta {
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![OpenAiStreamToolCall {
                        index: Some(0),
                        id: None,
                        function: Some(OpenAiStreamToolCallFunction {
                            name: None,
                            arguments: Some("ty\":\"Paris\"}".into()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let m2 = map_stream_chunk(&chunk2, &mut acc);
        assert!(m2.delta_tool_calls.is_empty());
        assert_eq!(acc[0].arguments, "{\"city\":\"Paris\"}");

        // Final chunk: finish reason triggers emission.
        let chunk3 = OpenAiStreamChunk {
            id: Some("test".into()),
            model: Some("gpt-4o".into()),
            choices: vec![OpenAiStreamChoice {
                delta: OpenAiStreamDelta {
                    content: None,
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("tool_calls".into()),
            }],
            usage: Some(OpenAiUsage {
                prompt_tokens: 100,
                completion_tokens: 20,
            }),
        };
        let m3 = map_stream_chunk(&chunk3, &mut acc);
        assert_eq!(m3.delta_tool_calls.len(), 1);
        assert_eq!(m3.delta_tool_calls[0].id, "call_abc");
        assert_eq!(m3.delta_tool_calls[0].name, "get_weather");
        assert_eq!(m3.finish_reason, Some(FinishReason::ToolUse));
        assert!(m3.usage.is_some());
        assert!(acc.is_empty()); // Drained.
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
        };

        let messages = OpenAiProvider::build_messages(&request);
        assert_eq!(messages.len(), 1);

        let json = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        // First part: image_url with data URI
        assert_eq!(content[0]["type"], "image_url");
        assert_eq!(
            content[0]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo="
        );
        // Second part: text
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "What is in this image?");
    }

    #[test]
    fn test_build_messages_user_without_attachments() {
        use y_core::types::{Message, Role};

        let request = ChatRequest {
            messages: vec![Message {
                message_id: "test-1".into(),
                role: Role::User,
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::json!({}),
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
        };

        let messages = OpenAiProvider::build_messages(&request);
        assert_eq!(messages.len(), 1);

        // Plain text content (no array), serialized as a string.
        let json = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn test_openai_content_into_text() {
        let text = OpenAiContent::Text("hello".into());
        assert_eq!(text.into_text(), Some("hello".into()));

        let empty = OpenAiContent::Text(String::new());
        assert_eq!(empty.into_text(), None);

        let parts = OpenAiContent::Parts(vec![
            OpenAiContentPart::ImageUrl {
                image_url: OpenAiImageUrl {
                    url: "data:image/png;base64,abc".into(),
                },
            },
            OpenAiContentPart::Text {
                text: "describe this".into(),
            },
        ]);
        assert_eq!(parts.into_text(), Some("describe this".into()));
    }
}
