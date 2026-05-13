//! OpenAI-compatible provider backend.
//!
//! Supports `OpenAI` API and any compatible endpoints (e.g., Azure `OpenAI`,
//! vLLM, `LiteLLM`) via configurable base URL.

use async_trait::async_trait;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use std::collections::VecDeque;

use crate::config::HttpProtocol;
use crate::inter_stream::InterStreamEvent;
use crate::tool_call_accumulator::ToolCallAccumulatorSet;
use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, GeneratedImage,
    ImageContentDelta, LlmProvider, ProviderCapability, ProviderError, ProviderMetadata,
    ProviderType, RequestMode, ToolCallingMode,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

/// OpenAI-compatible LLM provider.
#[derive(Debug)]
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    custom_headers: reqwest::header::HeaderMap,
    metadata: ProviderMetadata,
    /// Send `stream_options.include_usage = true` on streaming requests.
    /// Defaults to `false` because many OpenAI-compatible backends reject
    /// the `stream_options` field. See [`crate::config::ProviderConfig`].
    include_usage: bool,
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
        capabilities: Vec<ProviderCapability>,
        max_concurrency: usize,
        context_window: usize,
        tool_calling_mode: ToolCallingMode,
    ) -> Self {
        let headers = std::collections::HashMap::new();
        Self::with_headers(
            id,
            model,
            api_key,
            base_url,
            proxy_url,
            tags,
            capabilities,
            max_concurrency,
            context_window,
            tool_calling_mode,
            &headers,
            HttpProtocol::Http1,
        )
    }

    /// Create a new `OpenAI` provider with additional HTTP headers.
    pub fn with_headers<S: std::hash::BuildHasher>(
        id: &str,
        model: &str,
        api_key: String,
        base_url: Option<String>,
        proxy_url: Option<String>,
        tags: Vec<String>,
        capabilities: Vec<ProviderCapability>,
        max_concurrency: usize,
        context_window: usize,
        tool_calling_mode: ToolCallingMode,
        headers: &std::collections::HashMap<String, String, S>,
        http_protocol: HttpProtocol,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let custom_headers = crate::http_headers::custom_header_map(headers).unwrap_or_else(
            |message| {
                tracing::warn!(provider_id = %id, error = %message, "Ignoring invalid provider custom headers");
                reqwest::header::HeaderMap::default()
            },
        );

        Self {
            client: crate::http_headers::provider_http_client(http_protocol, proxy_url)
                .unwrap_or_else(|_| Client::new()),
            api_key,
            base_url,
            custom_headers,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::OpenAi,
                model: model.to_string(),
                tags,
                capabilities,
                max_concurrency,
                context_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode,
            },
            include_usage: false,
        }
    }

    /// Builder-style setter: opt in to `stream_options.include_usage = true`
    /// on streaming requests. Pool wiring reads this from
    /// [`crate::config::ProviderConfig::include_usage`].
    #[must_use]
    pub fn with_include_usage(mut self, include_usage: bool) -> Self {
        self.include_usage = include_usage;
        self
    }

    /// Build the full API URL for a given endpoint.
    fn api_url(&self, endpoint: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint)
    }

    fn latest_user_prompt(request: &ChatRequest) -> Result<String, ProviderError> {
        request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == y_core::types::Role::User)
            .map(|message| message.content.trim().to_string())
            .filter(|prompt| !prompt.is_empty())
            .ok_or_else(|| ProviderError::Other {
                message: "image generation requires a non-empty user prompt".into(),
            })
    }

    fn extract_image_attachment(request: &ChatRequest) -> Option<String> {
        request.messages.iter().rev().find_map(|message| {
            message
                .metadata
                .get("attachments")
                .and_then(|value| value.as_array())
                .and_then(|attachments| {
                    attachments.iter().find_map(|att| {
                        let mime = att.get("mime_type")?.as_str()?;
                        if !mime.starts_with("image/") {
                            return None;
                        }
                        let b64 = att.get("base64_data")?.as_str()?;
                        Some(format!("data:{mime};base64,{b64}"))
                    })
                })
        })
    }

    fn build_image_generation_request_body(
        &self,
        request: &ChatRequest,
    ) -> Result<OpenAiImageGenerationRequest, ProviderError> {
        let model = request.model.as_deref().unwrap_or(&self.metadata.model);
        let prompt = Self::latest_user_prompt(request)?;
        let opts = request.image_generation_options.as_ref();

        let image = Self::extract_image_attachment(request);

        let watermark = opts.map(|o| o.watermark);
        let size = opts.and_then(|o| o.size.clone());
        let max_images = opts.map_or(1, |o| o.max_images);

        let (sequential, sequential_opts) = if max_images > 1 {
            (
                Some("auto".to_string()),
                Some(SequentialImageGenOptions { max_images }),
            )
        } else {
            (None, None)
        };

        Ok(OpenAiImageGenerationRequest {
            model: model.to_string(),
            prompt,
            response_format: Some("b64_json".to_string()),
            size,
            watermark,
            sequential_image_generation: sequential,
            sequential_image_generation_options: sequential_opts,
            image,
        })
    }

    async fn generate_images(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = self.build_image_generation_request_body(request)?;
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self.client.post(self.api_url("images/generations"));
        request_builder =
            crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
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
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
                .unwrap_or(60_u64);

            return Err(ProviderError::RateLimited {
                provider: self.metadata.id.to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
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
        let image_response: OpenAiImageGenerationResponse =
            serde_json::from_value(raw_response.clone()).map_err(|e| ProviderError::Other {
                message: format!("parse image generation response: {e}"),
            })?;

        let mut content_parts = Vec::new();
        let mut generated_images = Vec::new();

        for (index, item) in image_response.data.into_iter().enumerate() {
            if let Some(data) = item.b64_json.filter(|value| !value.is_empty()) {
                generated_images.push(GeneratedImage {
                    index,
                    mime_type: "image/png".into(),
                    data,
                });
            } else if let Some(url) = item.url.filter(|value| !value.is_empty()) {
                content_parts.push(url);
            }
        }

        if generated_images.is_empty() && content_parts.is_empty() {
            return Err(ProviderError::Other {
                message: "image generation response contained no images".into(),
            });
        }

        Ok(ChatResponse {
            id: String::new(),
            model: image_response
                .model
                .unwrap_or_else(|| self.metadata.model.clone()),
            content: (!content_parts.is_empty()).then(|| content_parts.join("\n")),
            reasoning_content: None,
            tool_calls: vec![],
            usage: TokenUsage::default(),
            finish_reason: FinishReason::Stop,
            raw_request,
            raw_response: Some(raw_response),
            provider_id: None,
            generated_images,
        })
    }

    async fn generate_images_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        use futures::stream;

        let response = self.generate_images(request).await?;
        let ChatResponse {
            raw_request,
            content,
            generated_images,
            finish_reason,
            usage,
            ..
        } = response;

        let mut chunks = Vec::new();
        if let Some(delta_content) = content.filter(|value| !value.is_empty()) {
            chunks.push(Ok(ChatStreamChunk {
                delta_content: Some(delta_content),
                delta_reasoning_content: None,
                delta_tool_calls: vec![],
                usage: None,
                finish_reason: None,
                delta_images: vec![],
            }));
        }
        for image in generated_images {
            chunks.push(Ok(ChatStreamChunk {
                delta_content: None,
                delta_reasoning_content: None,
                delta_tool_calls: vec![],
                usage: None,
                finish_reason: None,
                delta_images: vec![ImageContentDelta {
                    index: image.index,
                    mime_type: image.mime_type,
                    partial_data: image.data,
                    is_complete: true,
                }],
            }));
        }
        chunks.push(Ok(ChatStreamChunk {
            delta_content: None,
            delta_reasoning_content: None,
            delta_tool_calls: vec![],
            usage: Some(usage),
            finish_reason: Some(finish_reason),
            delta_images: vec![],
        }));

        Ok(ChatStreamResponse {
            stream: Box::pin(stream::iter(chunks)),
            raw_request,
            provider_id: None,
            model: self.metadata.model.clone(),
            context_window: self.metadata.context_window,
        })
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
            stream_options: if stream && self.include_usage {
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
                        ThinkingEffort::High => "high".to_string(),
                        ThinkingEffort::Max => {
                            tracing::warn!(
                                "ThinkingEffort::Max not supported by OpenAI; \
                                 downgrading to 'high'"
                            );
                            "high".to_string()
                        }
                    },
                }
            }),
            response_format: request.response_format.as_ref().map(|rf| {
                use y_core::provider::ResponseFormat;
                match rf {
                    ResponseFormat::Text => OpenAiResponseFormat::Text,
                    ResponseFormat::JsonObject => OpenAiResponseFormat::JsonObject,
                    ResponseFormat::JsonSchema { name, schema } => {
                        OpenAiResponseFormat::JsonSchema {
                            json_schema: OpenAiJsonSchema {
                                name: name.clone(),
                                schema: schema.clone(),
                                strict: true,
                            },
                        }
                    }
                }
            }),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        if request.request_mode == RequestMode::ImageGeneration {
            return self.generate_images(request).await;
        }

        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self.client.post(self.api_url("chat/completions"));
        request_builder =
            crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
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

        let (content, generated_images) = choice
            .message
            .content
            .map_or((None, vec![]), OpenAiContent::into_text_and_images);
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
            generated_images,
        })
    }

    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        if request.request_mode == RequestMode::ImageGeneration {
            return self.generate_images_stream(request).await;
        }

        let body = self.build_request_body(request, true);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self.client.post(self.api_url("chat/completions"));
        request_builder = crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
                .header("Content-Type", "application/json")
                // Explicitly opt in to SSE. Some compat relays (Cloudflare-fronted,
                // nginx with response buffering, sidecar SSE adapters) only switch
                // to chunked streaming when this header is present — without it
                // they buffer the response and return one giant blob at the end
                // or reject with 415.
                .header("Accept", "text/event-stream");

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

        Ok(ChatStreamResponse {
            stream: crate::inter_stream_adapter::into_chat_stream(Box::pin(
                build_openai_inter_stream(Box::pin(byte_stream)),
            )),
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
// (SseState and extract_sse_event are now in crate::sse)

/// Build the `OpenAI` inter-stream event stream from a raw HTTP byte stream.
///
/// Extracted as a free function so tests can drive the SSE parsing loop
/// without spinning up a real HTTP server. The closure-based version
/// previously inlined inside `chat_completion_stream` had identical
/// semantics.
fn build_openai_inter_stream(
    byte_stream: crate::sse::ByteStream,
) -> impl futures::Stream<Item = Result<crate::inter_stream::InterStreamEvent, ProviderError>> + Send
{
    futures::stream::unfold(
        (
            crate::sse::SseStreamState::new(byte_stream),
            ToolCallAccumulatorSet::default(),
            VecDeque::<InterStreamEvent>::new(),
        ),
        |mut composite| async move {
            let (ref mut state, ref mut tool_acc, ref mut pending) = composite;

            if let Some(event) = pending.pop_front() {
                return Some((Ok(event), composite));
            }

            if state.done {
                return None;
            }

            loop {
                if let Some(event) = crate::sse::extract_sse_data(&mut state.buffer) {
                    let trimmed = event.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    if trimmed == "[DONE]" {
                        state.done = true;
                        for tc in tool_acc.drain_completed() {
                            pending.push_back(InterStreamEvent::ToolCall(tc));
                        }
                        if let Some(event) = pending.pop_front() {
                            return Some((Ok(event), composite));
                        }
                        return None;
                    }

                    match serde_json::from_str::<OpenAiStreamChunk>(trimmed) {
                        Ok(chunk) => {
                            let mut events = map_to_inter_events(&chunk, tool_acc);
                            if events.is_empty() {
                                continue;
                            }
                            let first = events.remove(0);
                            pending.extend(events);
                            return Some((Ok(first), composite));
                        }
                        Err(e) => {
                            // Tolerate non-conforming events from OpenAI-compat
                            // relays (keepalive frames, proxy comments,
                            // vendor-specific control messages). Terminating on
                            // a single malformed event would kill the whole turn
                            // — the Vercel `@ai-sdk/openai-compatible` reference
                            // SDK also keeps the stream alive after such
                            // failures.
                            tracing::warn!(
                                error = %e,
                                data = %trimmed,
                                "Skipping unparseable OpenAI SSE event"
                            );
                            continue;
                        }
                    }
                }

                match state.read_next().await {
                    Ok(true) => {}
                    Ok(false) => {
                        while let Some(event) = crate::sse::extract_sse_data(&mut state.buffer) {
                            let trimmed = event.trim();
                            if trimmed.is_empty() || trimmed == "[DONE]" {
                                continue;
                            }
                            if let Ok(chunk) = serde_json::from_str::<OpenAiStreamChunk>(trimmed) {
                                for ev in map_to_inter_events(&chunk, tool_acc) {
                                    pending.push_back(ev);
                                }
                            }
                        }
                        for tc in tool_acc.drain_completed() {
                            pending.push_back(InterStreamEvent::ToolCall(tc));
                        }
                        if let Some(event) = pending.pop_front() {
                            return Some((Ok(event), composite));
                        }
                        return None;
                    }
                    Err(e) => return Some((Err(e), composite)),
                }
            }
        },
    )
}

fn map_to_inter_events(
    chunk: &OpenAiStreamChunk,
    tool_acc: &mut crate::tool_call_accumulator::ToolCallAccumulatorSet,
) -> Vec<crate::inter_stream::InterStreamEvent> {
    use crate::inter_stream::InterStreamEvent;

    let choice = chunk.choices.first();
    let mut events = Vec::new();

    if let Some(text) = choice.and_then(|c| c.delta.content.clone()) {
        if !text.is_empty() {
            events.push(InterStreamEvent::TextDelta(text));
        }
    }

    if let Some(reasoning) = choice.and_then(|c| c.delta.reasoning_content.clone()) {
        if !reasoning.is_empty() {
            events.push(InterStreamEvent::ReasoningDelta(reasoning));
        }
    }

    if let Some(choice) = choice {
        if let Some(ref tcs) = choice.delta.tool_calls {
            for tc in tcs {
                let idx = tc.index.map(|i| i as usize);
                tool_acc.process_delta(
                    idx,
                    tc.id.as_deref(),
                    tc.function.as_ref().and_then(|f| f.name.as_deref()),
                    tc.function.as_ref().and_then(|f| f.arguments.as_deref()),
                );
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

    if finish_reason.is_some() {
        for tc in tool_acc.drain_completed() {
            events.push(InterStreamEvent::ToolCall(tc));
        }
    }

    if let Some(usage) = chunk.usage.as_ref().map(|u| TokenUsage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
        cache_read_tokens: None,
        cache_write_tokens: None,
        ..Default::default()
    }) {
        events.push(InterStreamEvent::Usage(usage));
    }

    if let Some(reason) = finish_reason {
        events.push(InterStreamEvent::Finished(reason));
    }

    events
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
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAiResponseFormat>,
}

#[derive(Debug, Serialize)]
struct OpenAiReasoning {
    effort: String,
}

/// `OpenAI` `response_format` for structured output.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponseFormat {
    /// Plain text (default).
    Text,
    /// JSON output (model chooses schema).
    JsonObject,
    /// JSON output conforming to a specific schema.
    JsonSchema {
        /// Nested schema wrapper.
        json_schema: OpenAiJsonSchema,
    },
}

#[derive(Debug, Serialize)]
struct OpenAiJsonSchema {
    name: String,
    schema: serde_json::Value,
    strict: bool,
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
    /// Extract text and generated images from this content value.
    fn into_text_and_images(self) -> (Option<String>, Vec<GeneratedImage>) {
        match self {
            OpenAiContent::Text(s) => {
                let text = if s.is_empty() { None } else { Some(s) };
                (text, vec![])
            }
            OpenAiContent::Parts(parts) => {
                let mut texts = Vec::new();
                let mut images = Vec::new();
                let mut img_index = 0usize;
                for part in parts {
                    match part {
                        OpenAiContentPart::Text { text } => texts.push(text),
                        OpenAiContentPart::GeneratedImage { image } => {
                            if !image.data.is_empty() {
                                images.push(GeneratedImage {
                                    index: img_index,
                                    mime_type: "image/png".to_string(),
                                    data: image.data,
                                });
                                img_index += 1;
                            }
                        }
                        OpenAiContentPart::ImageUrl { .. } => {}
                    }
                }
                let text = if texts.is_empty() {
                    None
                } else {
                    Some(texts.join(""))
                };
                (text, images)
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
    #[serde(rename = "generated_image")]
    GeneratedImage {
        #[serde(flatten)]
        image: OpenAiGeneratedImage,
    },
}

/// Image data from an LLM-generated image in a chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiGeneratedImage {
    #[serde(default, alias = "b64_json")]
    data: String,
}

/// Image URL payload for `OpenAI` vision API. Supports both HTTP URLs
/// and inline data URIs (`data:{mime};base64,{data}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiImageUrl {
    url: String,
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

#[derive(Debug, Clone, Serialize)]
struct OpenAiImageGenerationRequest {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    watermark: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequential_image_generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequential_image_generation_options: Option<SequentialImageGenOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SequentialImageGenOptions {
    max_images: u32,
}

#[derive(Debug, Deserialize)]
struct OpenAiImageGenerationResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    data: Vec<OpenAiImageGenerationItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiImageGenerationItem {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
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
    use crate::sse::extract_sse_data;
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
            vec![],
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
            response_format: None,
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
            response_format: None,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert!(json["stream"].as_bool().unwrap());
        assert!(json["stream_options"]["include_usage"].as_bool().unwrap());
    }

    /// Regression: `stream_options` must NOT be sent unless the provider was
    /// opted into `include_usage`. Several OpenAI-compatible backends reject
    /// the field with HTTP 400.
    #[test]
    fn streaming_request_omits_stream_options_by_default() {
        let provider = OpenAiProvider::new(
            "test",
            "gpt-4o",
            "sk-test".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        );

        let request = ChatRequest {
            messages: vec![y_core::types::Message {
                message_id: "m1".into(),
                role: y_core::types::Role::User,
                content: "hi".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::json!({}),
            }],
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let body = provider.build_request_body(&request, true);
        let json = serde_json::to_value(&body).unwrap();
        assert!(json["stream"].as_bool().unwrap());
        assert!(
            json.get("stream_options").is_none(),
            "stream_options must be absent without include_usage opt-in: {json}"
        );
    }

    #[test]
    fn streaming_request_emits_stream_options_when_opted_in() {
        let provider = OpenAiProvider::new(
            "test",
            "gpt-4o",
            "sk-test".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_include_usage(true);

        let request = ChatRequest {
            messages: vec![y_core::types::Message {
                message_id: "m1".into(),
                role: y_core::types::Role::User,
                content: "hi".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::json!({}),
            }],
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let body = provider.build_request_body(&request, true);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["stream_options"]["include_usage"], true);
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
                .map(super::OpenAiContent::into_text_and_images)
                .and_then(|(text, _)| text),
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
            vec![],
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
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "{\"id\":\"123\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn test_extract_sse_event_done() {
        let mut buf = "data: [DONE]\n\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "[DONE]");
    }

    #[test]
    fn test_extract_sse_event_incomplete() {
        let mut buf = "data: partial".to_string();
        assert!(extract_sse_data(&mut buf).is_none());
    }

    #[test]
    fn test_extract_sse_event_multiple() {
        let mut buf = "data: first\n\ndata: second\n\n".to_string();
        let e1 = extract_sse_data(&mut buf).unwrap();
        assert_eq!(e1, "first");
        let e2 = extract_sse_data(&mut buf).unwrap();
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
    fn test_map_to_inter_events_content() {
        use crate::inter_stream::InterStreamEvent;

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

        let mut acc = ToolCallAccumulatorSet::default();
        let events = map_to_inter_events(&chunk, &mut acc);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], InterStreamEvent::TextDelta(t) if t == "Hello"));
    }

    #[test]
    fn test_map_to_inter_events_tool_calls_incremental() {
        use crate::inter_stream::InterStreamEvent;

        let mut acc = ToolCallAccumulatorSet::default();

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
        let events1 = map_to_inter_events(&chunk1, &mut acc);
        assert!(events1.is_empty());

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
        let events2 = map_to_inter_events(&chunk2, &mut acc);
        assert!(events2.is_empty());

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
        let events3 = map_to_inter_events(&chunk3, &mut acc);
        let tool_events: Vec<_> = events3
            .iter()
            .filter(|e| matches!(e, InterStreamEvent::ToolCall(_)))
            .collect();
        assert_eq!(tool_events.len(), 1);
        if let InterStreamEvent::ToolCall(tc) = &tool_events[0] {
            assert_eq!(tc.id, "call_abc");
            assert_eq!(tc.name, "get_weather");
            assert_eq!(tc.arguments, serde_json::json!({"city": "Paris"}));
        } else {
            panic!("expected ToolCall event");
        }
        assert!(events3
            .iter()
            .any(|e| matches!(e, InterStreamEvent::Finished(FinishReason::ToolUse))));
        assert!(events3
            .iter()
            .any(|e| matches!(e, InterStreamEvent::Usage(_))));
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
            request_mode: RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
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
            request_mode: RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let messages = OpenAiProvider::build_messages(&request);
        assert_eq!(messages.len(), 1);

        // Plain text content (no array), serialized as a string.
        let json = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn test_openai_content_into_text_and_images() {
        let text = OpenAiContent::Text("hello".into());
        let (t, imgs) = text.into_text_and_images();
        assert_eq!(t, Some("hello".into()));
        assert!(imgs.is_empty());

        let empty = OpenAiContent::Text(String::new());
        let (t, imgs) = empty.into_text_and_images();
        assert_eq!(t, None);
        assert!(imgs.is_empty());

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
        let (t, imgs) = parts.into_text_and_images();
        assert_eq!(t, Some("describe this".into()));
        assert!(imgs.is_empty());
    }

    #[test]
    fn test_openai_content_extracts_generated_images() {
        let parts = OpenAiContent::Parts(vec![
            OpenAiContentPart::Text {
                text: "Here is the image:".into(),
            },
            OpenAiContentPart::GeneratedImage {
                image: OpenAiGeneratedImage {
                    data: "iVBORw0KGgo=".into(),
                },
            },
        ]);
        let (t, imgs) = parts.into_text_and_images();
        assert_eq!(t, Some("Here is the image:".into()));
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].index, 0);
        assert_eq!(imgs[0].mime_type, "image/png");
        assert_eq!(imgs[0].data, "iVBORw0KGgo=");
    }

    // -----------------------------------------------------------------------
    // Resilient SSE parsing (regression: malformed event must not kill stream)
    // -----------------------------------------------------------------------

    /// Build an inter-stream over a fixed sequence of byte chunks.
    fn inter_stream_from_chunks(
        chunks: Vec<&'static str>,
    ) -> impl futures::Stream<Item = Result<crate::inter_stream::InterStreamEvent, ProviderError>> + Send
    {
        use bytes::Bytes;
        let stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(|s| Ok::<_, reqwest::Error>(Bytes::from_static(s.as_bytes()))),
        );
        super::build_openai_inter_stream(Box::pin(stream))
    }

    /// A garbage event in the middle of an otherwise-valid stream must be
    /// skipped without producing an error or terminating the stream.
    #[tokio::test]
    async fn stream_skips_malformed_sse_event() {
        use crate::inter_stream::InterStreamEvent;
        use futures::StreamExt as _;

        let stream = inter_stream_from_chunks(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
            "data: this is not json\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\" there\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ]);

        let events: Vec<_> = stream.collect().await;
        let texts: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Ok(InterStreamEvent::TextDelta(t)) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["hi", " there"]);

        // None of the items should be Err.
        assert!(
            events.iter().all(Result::is_ok),
            "stream produced an error: {events:?}"
        );

        // Finished must still fire.
        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(InterStreamEvent::Finished(FinishReason::Stop)))));
    }

    /// SSE comment lines / vendor keepalive frames that don't follow the
    /// OpenAI schema must be tolerated.
    #[tokio::test]
    async fn stream_tolerates_keepalive_frames() {
        use crate::inter_stream::InterStreamEvent;
        use futures::StreamExt as _;

        let stream = inter_stream_from_chunks(vec![
            ": ping\n\n",
            "data: {\"_keepalive\": true}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ]);

        let events: Vec<_> = stream.collect().await;
        let texts: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Ok(InterStreamEvent::TextDelta(t)) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["ok"]);
        assert!(events.iter().all(Result::is_ok));
    }
}
