//! Azure `OpenAI` provider backend.
//!
//! Implements the Azure `OpenAI` Service API format with:
//! - Three endpoint modes: full-endpoint (legacy), deployment-based, and v1
//! - `api-key` or Bearer token authentication (Azure AD / Entra ID)
//! - `api-version` query parameter (configurable)
//! - Same request/response format as `OpenAI` (reuses `OpenAI` wire types)
//! - SSE streaming support

use async_trait::async_trait;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use std::collections::VecDeque;

use crate::config::{AzureAuthMode, HttpProtocol};
use crate::inter_stream::InterStreamEvent;
use crate::tool_call_accumulator::ToolCallAccumulatorSet;
use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason,
    ImageContentDelta, LlmProvider, ProviderCapability, ProviderError, ProviderMetadata,
    ProviderType, RequestMode, ToolCallingMode,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

const DEFAULT_API_VERSION: &str = "2024-10-21";
const DEFAULT_V1_API_VERSION: &str = "preview";

/// How the provider constructs Azure endpoint URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AzureEndpointMode {
    /// Legacy: `base_url` contains `/deployments/`, used as-is. Model NOT in body.
    FullEndpoint,
    /// `{prefix}/deployments/{model}{path}`. Model NOT in body.
    DeploymentBased,
    /// `{prefix}/v1{path}`. Model IN body.
    V1,
}

/// Azure `OpenAI` Service provider.
///
/// Supports three endpoint modes (full-endpoint legacy, deployment-based, v1)
/// and two authentication modes (API key, Bearer token).
#[derive(Debug)]
pub struct AzureOpenAiProvider {
    client: Client,
    api_key: String,
    endpoint_mode: AzureEndpointMode,
    endpoint_prefix: String,
    api_version: String,
    auth_mode: AzureAuthMode,
    custom_headers: reqwest::header::HeaderMap,
    metadata: ProviderMetadata,
    include_usage: bool,
    use_max_completion_tokens: bool,
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

    /// Create a new Azure `OpenAI` provider with additional HTTP headers.
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
        let endpoint_prefix = base_url.unwrap_or_default();
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
            endpoint_mode: AzureEndpointMode::FullEndpoint,
            endpoint_prefix,
            api_version: DEFAULT_API_VERSION.to_string(),
            auth_mode: AzureAuthMode::ApiKey,
            custom_headers,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::Azure,
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
            use_max_completion_tokens: false,
        }
    }

    /// Builder-style setter: opt in to `stream_options.include_usage = true`
    /// on streaming requests.
    #[must_use]
    pub fn with_include_usage(mut self, include_usage: bool) -> Self {
        self.include_usage = include_usage;
        self
    }

    /// Builder-style setter: opt in to sending the output-token limit as
    /// `max_completion_tokens` instead of `max_tokens`.
    #[must_use]
    pub fn with_use_max_completion_tokens(mut self, use_max_completion_tokens: bool) -> Self {
        self.use_max_completion_tokens = use_max_completion_tokens;
        self
    }

    /// Builder-style setter: configure Azure-specific endpoint resolution,
    /// API version, and authentication mode.
    #[must_use]
    pub fn with_azure_config(
        mut self,
        resource_name: Option<&str>,
        use_deployment_urls: bool,
        api_version: Option<&str>,
        auth_mode: AzureAuthMode,
    ) -> Self {
        let (mode, prefix) =
            if !self.endpoint_prefix.is_empty() && self.endpoint_prefix.contains("/deployments/") {
                (
                    AzureEndpointMode::FullEndpoint,
                    self.endpoint_prefix.clone(),
                )
            } else if let Some(name) = resource_name {
                let prefix = if self.endpoint_prefix.is_empty() {
                    format!("https://{name}.openai.azure.com/openai")
                } else {
                    self.endpoint_prefix.clone()
                };
                if use_deployment_urls {
                    (AzureEndpointMode::DeploymentBased, prefix)
                } else {
                    (AzureEndpointMode::V1, prefix)
                }
            } else if !self.endpoint_prefix.is_empty() {
                if use_deployment_urls {
                    (
                        AzureEndpointMode::DeploymentBased,
                        self.endpoint_prefix.clone(),
                    )
                } else {
                    (AzureEndpointMode::V1, self.endpoint_prefix.clone())
                }
            } else {
                (
                    AzureEndpointMode::FullEndpoint,
                    self.endpoint_prefix.clone(),
                )
            };

        let default_version = match mode {
            AzureEndpointMode::V1 => DEFAULT_V1_API_VERSION,
            _ => DEFAULT_API_VERSION,
        };

        self.endpoint_mode = mode;
        self.endpoint_prefix = prefix;
        self.api_version = api_version.unwrap_or(default_version).to_string();
        self.auth_mode = auth_mode;
        self
    }

    /// Construct endpoint URL for the given path suffix.
    fn azure_endpoint(&self, path: &str) -> String {
        let base = self.endpoint_prefix.trim_end_matches('/');
        let url = match self.endpoint_mode {
            AzureEndpointMode::FullEndpoint => format!("{base}{path}"),
            AzureEndpointMode::DeploymentBased => {
                format!("{base}/deployments/{}{path}", self.metadata.model)
            }
            AzureEndpointMode::V1 => format!("{base}/v1{path}"),
        };
        format!("{url}?api-version={}", self.api_version)
    }

    /// Apply authentication headers to a request builder.
    fn apply_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.is_empty() {
            return builder;
        }
        match self.auth_mode {
            AzureAuthMode::ApiKey => builder.header("api-key", &self.api_key),
            AzureAuthMode::Bearer => {
                builder.header("Authorization", format!("Bearer {}", self.api_key))
            }
        }
    }

    fn chat_url(&self) -> String {
        self.azure_endpoint("/chat/completions")
    }

    fn image_generation_url(&self) -> String {
        self.azure_endpoint("/images/generations")
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
        request: &ChatRequest,
        model: &str,
    ) -> Result<AzureImageGenerationRequest, ProviderError> {
        let opts = request.image_generation_options.as_ref();
        let image = Self::extract_image_attachment(request);
        let watermark = opts.map(|o| o.watermark);
        let size = opts.and_then(|o| o.size.clone());
        let max_images = opts.map_or(1, |o| o.max_images);
        let (sequential, sequential_opts) = if max_images > 1 {
            (
                Some("auto".to_string()),
                Some(AzureSequentialImageGenOptions { max_images }),
            )
        } else {
            (None, None)
        };
        Ok(AzureImageGenerationRequest {
            model: model.to_string(),
            prompt: Self::latest_user_prompt(request)?,
            response_format: Some("b64_json".to_string()),
            size,
            watermark,
            sequential_image_generation: sequential,
            sequential_image_generation_options: sequential_opts,
            image,
        })
    }

    async fn generate_images(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let model = request.model.as_deref().unwrap_or(&self.metadata.model);
        let body = Self::build_image_generation_request_body(request, model)?;
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self.client.post(self.image_generation_url());
        request_builder =
            crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
                .header("Content-Type", "application/json");
        request_builder = self.apply_auth(request_builder);

        let response = request_builder
            .json(&body)
            .send()
            .await
            .map_err(|e| crate::net_error::network_error_from_reqwest(&e))?;

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
        let image_response: AzureImageGenerationResponse =
            serde_json::from_value(raw_response.clone()).map_err(|e| ProviderError::Other {
                message: format!("parse image generation response: {e}"),
            })?;

        let generated_images: Vec<_> = image_response
            .data
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                item.b64_json.map(|data| y_core::provider::GeneratedImage {
                    index,
                    mime_type: "image/png".into(),
                    data,
                })
            })
            .collect();

        if generated_images.is_empty() {
            return Err(ProviderError::Other {
                message: "image generation response contained no images".into(),
            });
        }

        Ok(ChatResponse {
            id: String::new(),
            model: image_response
                .model
                .unwrap_or_else(|| self.metadata.model.clone()),
            content: None,
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
            finish_reason,
            usage,
            generated_images,
            ..
        } = response;

        let mut chunks = Vec::new();
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

        let model = match self.endpoint_mode {
            AzureEndpointMode::V1 => Some(
                request
                    .model
                    .clone()
                    .unwrap_or_else(|| self.metadata.model.clone()),
            ),
            _ => None,
        };

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

        let (max_tokens, max_completion_tokens) = if self.use_max_completion_tokens {
            (None, request.max_tokens)
        } else {
            (request.max_tokens, None)
        };

        AzureRequest {
            model,
            messages: Self::build_messages(request),
            max_tokens,
            max_completion_tokens,
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
        }
    }
}

#[async_trait]
impl LlmProvider for AzureOpenAiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        if request.request_mode == RequestMode::ImageGeneration {
            return self.generate_images(request).await;
        }

        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self.client.post(self.chat_url());
        request_builder =
            crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
                .header("Content-Type", "application/json");
        request_builder = self.apply_auth(request_builder);

        let response = request_builder
            .json(&body)
            .send()
            .await
            .map_err(|e| crate::net_error::network_error_from_reqwest(&e))?;

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
            reasoning_content: None,
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
            generated_images: vec![],
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

        let mut request_builder = self.client.post(self.chat_url());
        request_builder = crate::http_headers::apply_custom_headers(request_builder, &self.custom_headers)
                .header("Content-Type", "application/json")
                // Opt in to SSE — see openai.rs for the rationale.
                .header("Accept", "text/event-stream");
        request_builder = self.apply_auth(request_builder);

        let response = request_builder
            .json(&body)
            .send()
            .await
            .map_err(|e| crate::net_error::network_error_from_reqwest(&e))?;

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
                    message: error_body,
                });
            }
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        // Parse SSE stream -- same format as OpenAI.
        let byte_stream = response.bytes_stream();
        let provider_id = self.metadata.id.to_string();

        let inter_stream = futures::stream::unfold(
            (
                crate::sse::SseStreamState::with_status(
                    Box::pin(byte_stream),
                    Some(status.as_u16()),
                ),
                ToolCallAccumulatorSet::default(),
                VecDeque::<InterStreamEvent>::new(),
            ),
            move |mut composite| {
                let _provider_id = provider_id.clone();
                async move {
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

                            match serde_json::from_str::<AzureStreamChunk>(trimmed) {
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
                                    // Tolerate non-conforming events from
                                    // OpenAI-compat relays — see openai.rs
                                    // for the rationale.
                                    tracing::warn!(
                                        error = %e,
                                        data = %trimmed,
                                        "Skipping unparseable Azure SSE event"
                                    );
                                    continue;
                                }
                            }
                        }

                        match state.read_next().await {
                            Ok(true) => {}
                            Ok(false) => {
                                while let Some(event) =
                                    crate::sse::extract_sse_data(&mut state.buffer)
                                {
                                    let trimmed = event.trim();
                                    if trimmed.is_empty() || trimmed == "[DONE]" {
                                        continue;
                                    }
                                    if let Ok(chunk) =
                                        serde_json::from_str::<AzureStreamChunk>(trimmed)
                                    {
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
                }
            },
        );

        Ok(ChatStreamResponse {
            stream: crate::inter_stream_adapter::into_chat_stream(Box::pin(inter_stream)),
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

// (SseState and extract_sse_event are now in crate::sse)

fn map_to_inter_events(
    chunk: &AzureStreamChunk,
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
// Azure OpenAI API types (same wire format as OpenAI)
// ---------------------------------------------------------------------------

/// In V1 endpoint mode, `model` is included in the request body.
/// In `FullEndpoint` and `DeploymentBased` modes, `model` is `None` (deployment
/// is specified in the URL).
#[derive(Debug, Serialize)]
struct AzureRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    messages: Vec<AzureMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// Newer Azure-hosted `OpenAI` reasoning models (o1, o3, gpt-5) reject
    /// `max_tokens` and require this field instead. Mutually exclusive with
    /// `max_tokens` -- providers populate exactly one based on
    /// `use_max_completion_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
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
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
struct AzureImageGenerationRequest {
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
    sequential_image_generation_options: Option<AzureSequentialImageGenOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AzureSequentialImageGenOptions {
    max_images: u32,
}

#[derive(Debug, Deserialize)]
struct AzureImageGenerationResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    data: Vec<AzureImageGenerationItem>,
}

#[derive(Debug, Deserialize)]
struct AzureImageGenerationItem {
    #[serde(default)]
    b64_json: Option<String>,
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
    use crate::sse::extract_sse_data;
    use y_core::provider::ToolCallingMode;

    #[test]
    fn test_azure_provider_metadata() {
        let provider = AzureOpenAiProvider::new(
            "azure-gpt4o",
            "gpt-4o",
            "azure-key-test".into(),
            Some("https://myresource.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec!["reasoning".into(), "general".into()],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
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
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.chat_url(),
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn test_azure_request_serialization_no_model() {
        let req = AzureRequest {
            model: None,
            messages: vec![AzureMessage {
                role: "user".into(),
                content: Some("Hello".into()),
                tool_call_id: None,
                tool_calls: None,
            }],
            max_tokens: Some(100),
            max_completion_tokens: None,
            temperature: Some(0.7),
            top_p: None,
            stream: false,
            stream_options: None,
            tools: None,
            stop: None,
        };

        let json = serde_json::to_value(&req).expect("serialize");
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
        let event = extract_sse_data(&mut buffer);
        assert!(event.is_some());
        let data = event.unwrap();
        assert!(data.contains("Hi"));
    }

    #[test]
    fn test_azure_sse_done_signal() {
        let mut buffer = String::from("data: [DONE]\n\n");
        let event = extract_sse_data(&mut buffer);
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

    fn azure_chat_request(max_tokens: Option<u32>) -> ChatRequest {
        ChatRequest {
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
            max_tokens,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        }
    }

    /// Default Azure behavior: send `max_tokens`, omit `max_completion_tokens`.
    #[test]
    fn azure_request_body_uses_max_tokens_by_default() {
        let provider = AzureOpenAiProvider::new(
            "azure-test",
            "gpt-4o",
            "key".into(),
            Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        );

        let json = serde_json::to_value(
            provider.build_request_body(&azure_chat_request(Some(256)), false),
        )
        .unwrap();
        assert_eq!(json["max_tokens"], 256);
        assert!(json.get("max_completion_tokens").is_none(), "{json}");
    }

    /// Opt-in Azure behavior: send `max_completion_tokens`, omit `max_tokens`.
    /// Required by newer Azure-hosted OpenAI reasoning deployments.
    #[test]
    fn azure_request_body_uses_max_completion_tokens_when_opted_in() {
        let provider = AzureOpenAiProvider::new(
            "azure-o3",
            "o3",
            "key".into(),
            Some("https://res.openai.azure.com/openai/deployments/o3".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_use_max_completion_tokens(true);

        let json = serde_json::to_value(
            provider.build_request_body(&azure_chat_request(Some(256)), false),
        )
        .unwrap();
        assert_eq!(json["max_completion_tokens"], 256);
        assert!(json.get("max_tokens").is_none(), "{json}");
    }

    // -------------------------------------------------------------------
    // Endpoint resolution tests
    // -------------------------------------------------------------------

    #[test]
    fn full_endpoint_legacy_url_preserved() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            Some("https://myresource.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, None, AzureAuthMode::ApiKey);

        assert_eq!(provider.endpoint_mode, AzureEndpointMode::FullEndpoint);
        assert_eq!(
            provider.chat_url(),
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o\
             /chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn resource_name_v1_mode_default() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(Some("myresource"), false, None, AzureAuthMode::ApiKey);

        assert_eq!(provider.endpoint_mode, AzureEndpointMode::V1);
        assert_eq!(
            provider.chat_url(),
            "https://myresource.openai.azure.com/openai\
             /v1/chat/completions?api-version=preview"
        );
    }

    #[test]
    fn resource_name_deployment_based_mode() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(Some("myresource"), true, None, AzureAuthMode::ApiKey);

        assert_eq!(provider.endpoint_mode, AzureEndpointMode::DeploymentBased);
        assert_eq!(
            provider.chat_url(),
            "https://myresource.openai.azure.com/openai\
             /deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn bare_prefix_base_url_v1_mode() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            Some("https://custom-proxy.example.com/openai".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, None, AzureAuthMode::ApiKey);

        assert_eq!(provider.endpoint_mode, AzureEndpointMode::V1);
        assert_eq!(
            provider.chat_url(),
            "https://custom-proxy.example.com/openai\
             /v1/chat/completions?api-version=preview"
        );
    }

    #[test]
    fn configurable_api_version() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, Some("2025-01-01"), AzureAuthMode::ApiKey);

        assert_eq!(
            provider.chat_url(),
            "https://res.openai.azure.com/openai/deployments/gpt-4o\
             /chat/completions?api-version=2025-01-01"
        );
    }

    #[test]
    fn v1_mode_includes_model_in_body() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(Some("myresource"), false, None, AzureAuthMode::ApiKey);

        let json = serde_json::to_value(
            provider.build_request_body(&azure_chat_request(Some(100)), false),
        )
        .unwrap();
        assert_eq!(json["model"], "gpt-4o");
    }

    #[test]
    fn full_endpoint_omits_model_in_body() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, None, AzureAuthMode::ApiKey);

        let json = serde_json::to_value(
            provider.build_request_body(&azure_chat_request(Some(100)), false),
        )
        .unwrap();
        assert!(json.get("model").is_none(), "{json}");
    }

    #[test]
    fn deployment_based_omits_model_in_body() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "key".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(Some("res"), true, None, AzureAuthMode::ApiKey);

        let json = serde_json::to_value(
            provider.build_request_body(&azure_chat_request(Some(100)), false),
        )
        .unwrap();
        assert!(json.get("model").is_none(), "{json}");
    }

    #[test]
    fn image_generation_url_uses_endpoint_resolution() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "dall-e-3",
            "key".into(),
            None,
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(Some("myresource"), false, None, AzureAuthMode::ApiKey);

        assert_eq!(
            provider.image_generation_url(),
            "https://myresource.openai.azure.com/openai\
             /v1/images/generations?api-version=preview"
        );
    }

    // -------------------------------------------------------------------
    // Auth mode tests
    // -------------------------------------------------------------------

    #[test]
    fn bearer_auth_sets_authorization_header() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "my-entra-token".into(),
            Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, None, AzureAuthMode::Bearer);

        let req = provider.client.post("http://test.example.com");
        let req = provider.apply_auth(req);
        let built = req.build().expect("build request");

        assert_eq!(
            built
                .headers()
                .get("Authorization")
                .map(|v| v.to_str().unwrap()),
            Some("Bearer my-entra-token"),
        );
        assert!(built.headers().get("api-key").is_none());
    }

    #[test]
    fn api_key_auth_sets_api_key_header() {
        let provider = AzureOpenAiProvider::new(
            "test",
            "gpt-4o",
            "azure-key-123".into(),
            Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
            None,
            vec![],
            vec![],
            5,
            128_000,
            ToolCallingMode::default(),
        )
        .with_azure_config(None, false, None, AzureAuthMode::ApiKey);

        let req = provider.client.post("http://test.example.com");
        let req = provider.apply_auth(req);
        let built = req.build().expect("build request");

        assert_eq!(
            built.headers().get("api-key").map(|v| v.to_str().unwrap()),
            Some("azure-key-123"),
        );
        assert!(built.headers().get("Authorization").is_none());
    }
}
