//! LLM call dispatch -- streaming and non-streaming.
//!
//! Contains request building, routing, and the streaming chunk consumer.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use y_core::provider::{
    ChatRequest, GeneratedImage, ImageContentDelta, ProviderError, ProviderPool, RequestMode,
    RouteRequest,
};
use y_core::types::ProviderId;

use crate::cost::CostService;

use super::{AgentExecutionConfig, LlmIterationData, ToolExecContext, TurnEvent, TurnEventSender};

/// Normalize messages for optimal prompt cache prefix stability.
///
/// Trims trailing whitespace from message content and sorts JSON keys in
/// `tool_calls` arguments. This ensures the message prefix remains
/// bit-identical across turns, maximizing KV cache hits on providers that
/// support prompt caching (e.g., Anthropic `cache_control: ephemeral`).
///
/// The normalization is shallow: it only processes top-level message content
/// and `tool_call` arguments. Deep normalization of nested structures is not
/// needed because the LLM API serializes them deterministically.
fn normalize_messages(messages: &[y_core::types::Message]) -> Vec<y_core::types::Message> {
    messages
        .iter()
        .map(|msg| {
            let mut normalized = msg.clone();
            // Trim trailing whitespace from content (common source of
            // cache-busting differences from streaming reconstruction).
            normalized.content = normalized.content.trim_end().to_string();
            // Canonicalize tool_call arguments: re-serialize JSON Value
            // to get sorted keys, ensuring deterministic bytes across turns.
            for tc in &mut normalized.tool_calls {
                if let Ok(canonical) = serde_json::to_string(&tc.arguments) {
                    if let Ok(parsed) = serde_json::from_str(&canonical) {
                        tc.arguments = parsed;
                    }
                }
            }
            normalized
        })
        .collect()
}

pub(crate) fn build_chat_request(
    config: &AgentExecutionConfig,
    ctx: &ToolExecContext,
) -> ChatRequest {
    // Merge essential (static) + dynamically activated tool definitions.
    let tools = if config.request_mode == RequestMode::ImageGeneration {
        Vec::new()
    } else if ctx.dynamic_tool_defs.is_empty() {
        config.tool_definitions.clone()
    } else {
        let mut merged = config.tool_definitions.clone();
        for dyn_def in &ctx.dynamic_tool_defs {
            let dyn_name = dyn_def
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str());
            if let Some(name) = dyn_name {
                let already_present = merged.iter().any(|t| {
                    t.get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        == Some(name)
                });
                if !already_present {
                    merged.push(dyn_def.clone());
                }
            }
        }
        merged
    };

    ChatRequest {
        messages: normalize_messages(&ctx.working_history),
        model: None,
        request_mode: config.request_mode,
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        top_p: None,
        tools,
        tool_calling_mode: config.tool_calling_mode,
        tool_dialect: config.tool_dialect,
        stop: vec![],
        extra: serde_json::Value::Null,
        thinking: config.thinking.clone(),
        response_format: config.response_format.clone(),
        image_generation_options: config.image_generation_options.clone(),
    }
}

pub(crate) fn build_route_request(config: &AgentExecutionConfig) -> RouteRequest {
    build_route_request_with_tags(config, config.provider_tags.clone())
}

fn build_route_request_with_tags(
    config: &AgentExecutionConfig,
    required_tags: Vec<String>,
) -> RouteRequest {
    RouteRequest {
        preferred_provider_id: config.provider_id.as_ref().map(ProviderId::from_string),
        preferred_model: config.preferred_models.first().cloned(),
        required_tags,
        ..RouteRequest::default()
    }
}

pub(crate) fn build_route_requests(config: &AgentExecutionConfig) -> Vec<RouteRequest> {
    let mut routes = Vec::with_capacity(config.fallback_provider_tags.len() + 1);
    routes.push(build_route_request(config));
    routes.extend(
        config
            .fallback_provider_tags
            .iter()
            .cloned()
            .map(|tags| build_route_request_with_tags(config, tags)),
    );
    routes
}

/// Resolve a conservative context window across every provider a route may
/// select. Explicit provider IDs are exact; tag-routed requests use the
/// smallest matching window so failover cannot invalidate preflight.
pub(crate) fn resolve_preflight_context_window(
    metadata: &[y_core::provider::ProviderMetadata],
    routes: &[RouteRequest],
) -> usize {
    routes
        .iter()
        .filter_map(|route| {
            if let Some(provider_id) = route.preferred_provider_id.as_ref() {
                return metadata
                    .iter()
                    .find(|candidate| candidate.id == *provider_id)
                    .filter(|candidate| {
                        route
                            .required_tags
                            .iter()
                            .all(|tag| candidate.tags.contains(tag))
                    })
                    .map(|candidate| candidate.context_window);
            }
            metadata
                .iter()
                .filter(|candidate| {
                    route
                        .required_tags
                        .iter()
                        .all(|tag| candidate.tags.contains(tag))
                })
                .map(|candidate| candidate.context_window)
                .min()
        })
        .filter(|window| *window > 0)
        .min()
        .unwrap_or(0)
}

pub(crate) fn build_iteration_data(
    response: &y_core::provider::ChatResponse,
    fallback: &str,
    llm_start: std::time::Instant,
) -> LlmIterationData {
    let resp_input_tokens = u64::from(response.usage.input_tokens);
    let resp_output_tokens = u64::from(response.usage.output_tokens);
    let resp_cache_read_tokens = u64::from(response.usage.cache_read_tokens.unwrap_or(0));
    let resp_cache_write_tokens = u64::from(response.usage.cache_write_tokens.unwrap_or(0));
    let context_input_tokens = u64::from(response.usage.total_input_tokens());
    let cost = CostService::compute_cost_from_usage(&response.usage);
    let llm_elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);

    let prompt_preview = response.raw_request.as_ref().map_or_else(
        || fallback.to_string(),
        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    );

    let response_text_raw = response.raw_response.as_ref().map_or_else(
        || {
            let mut response_json = serde_json::json!({
                "content": response.content.clone().unwrap_or_default(),
                "model": response.model,
                "usage": response.usage.to_diagnostics_json(),
            });
            if let Some(reasoning) = response.reasoning_content.as_ref() {
                response_json["reasoning_content"] = serde_json::Value::String(reasoning.clone());
            }
            response_json.to_string()
        },
        std::string::ToString::to_string,
    );

    LlmIterationData {
        resp_input_tokens,
        resp_output_tokens,
        resp_cache_read_tokens,
        resp_cache_write_tokens,
        context_input_tokens,
        cost,
        llm_elapsed_ms,
        prompt_preview,
        response_text_raw,
    }
}

/// Build a provider-neutral `raw_response` for the streaming path.
///
/// The streaming path has no single raw JSON blob from the provider, so we
/// synthesize one for diagnostics. It uses the unified token field names and a
/// neutral envelope (no `object: chat.completion` / `choices` wrapper), so an
/// Anthropic stream is not mislabeled as an `OpenAI` completion. Nothing parses
/// this shape downstream; it is stored and displayed as the observation output.
fn build_streaming_raw_response(
    model_name: &str,
    content: &str,
    reasoning_content: Option<&str>,
    tool_calls: &[y_core::types::ToolCallRequest],
    generated_images: &[GeneratedImage],
    finish_reason: y_core::provider::FinishReason,
    usage: &y_core::types::TokenUsage,
) -> serde_json::Value {
    let finish_reason_str = match finish_reason {
        y_core::provider::FinishReason::Length => "length",
        y_core::provider::FinishReason::ToolUse => "tool_use",
        y_core::provider::FinishReason::ContentFilter => "content_filter",
        y_core::provider::FinishReason::Unknown | y_core::provider::FinishReason::Stop => "stop",
    };

    let tool_calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tool_call| {
            serde_json::json!({
                "id": tool_call.id,
                "name": tool_call.name,
                "arguments": tool_call.arguments,
            })
        })
        .collect();

    let mut response = serde_json::json!({
        "model": model_name,
        "content": content,
        "finish_reason": finish_reason_str,
        "usage": usage.to_diagnostics_json(),
    });
    if let Some(reasoning) = reasoning_content.filter(|value| !value.is_empty()) {
        response["reasoning_content"] = serde_json::Value::String(reasoning.to_string());
    }
    if !tool_calls_json.is_empty() {
        response["tool_calls"] = serde_json::Value::Array(tool_calls_json);
    }
    if !generated_images.is_empty() {
        response["generated_images"] =
            serde_json::to_value(generated_images).unwrap_or(serde_json::Value::Array(vec![]));
    }
    response
}

#[derive(Default)]
struct ImageAccumulator {
    images: std::collections::BTreeMap<usize, AccumulatingImage>,
}

struct AccumulatingImage {
    mime_type: String,
    data_parts: Vec<String>,
}

impl ImageAccumulator {
    fn push(&mut self, delta: &ImageContentDelta) {
        let entry = self
            .images
            .entry(delta.index)
            .or_insert_with(|| AccumulatingImage {
                mime_type: delta.mime_type.clone(),
                data_parts: Vec::new(),
            });
        if entry.mime_type.is_empty() {
            entry.mime_type.clone_from(&delta.mime_type);
        }
        if !delta.partial_data.is_empty() {
            entry.data_parts.push(delta.partial_data.clone());
        }
    }

    fn complete_image(&self, index: usize) -> Option<GeneratedImage> {
        self.images.get(&index).map(|img| GeneratedImage {
            index,
            mime_type: img.mime_type.clone(),
            data: img.data_parts.join(""),
        })
    }

    fn into_images(self) -> Vec<GeneratedImage> {
        self.images
            .into_iter()
            .map(|(index, img)| GeneratedImage {
                index,
                mime_type: img.mime_type,
                data: img.data_parts.join(""),
            })
            .collect()
    }
}

/// Partial content accumulated during streaming before cancellation or error.
#[derive(Default)]
pub(crate) struct PartialStreamingContent {
    pub content: String,
    pub reasoning: String,
}

/// Dispatch to streaming or non-streaming LLM call.
///
/// Returns `(ChatResponse, Option<reasoning_duration_ms>)`. The duration
/// is only available when streaming is active and the model produced
/// reasoning content.
///
/// `partial_out` captures text streamed to the frontend before cancellation
/// so callers can persist partial content that would otherwise be lost.
pub(crate) async fn call_llm(
    pool: &dyn ProviderPool,
    request: &ChatRequest,
    routes: &[RouteRequest],
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
    agent_name: &str,
    partial_out: &mut PartialStreamingContent,
) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError> {
    let [primary_route, fallback_routes @ ..] = routes else {
        return Err(y_core::provider::ProviderError::NoProviderAvailable { tags: vec![] });
    };

    let mut last_no_provider_error = None;
    let route_iter = std::iter::once(primary_route).chain(fallback_routes.iter());
    for route in route_iter {
        let result = if progress.is_some() {
            call_llm_streaming(
                pool,
                request,
                route,
                progress,
                cancel,
                agent_name,
                partial_out,
            )
            .await
        } else {
            call_llm_non_streaming(pool, request, route, cancel).await
        };

        match result {
            Ok(response) => return Ok(response),
            Err(error @ ProviderError::NoProviderAvailable { .. }) => {
                last_no_provider_error = Some(error);
                partial_out.content.clear();
                partial_out.reasoning.clear();
            }
            Err(error) => return Err(error),
        }
    }

    Err(
        last_no_provider_error.unwrap_or_else(|| ProviderError::NoProviderAvailable {
            tags: primary_route.required_tags.clone(),
        }),
    )
}

async fn call_llm_non_streaming(
    pool: &dyn ProviderPool,
    request: &ChatRequest,
    route: &RouteRequest,
    cancel: Option<&CancellationToken>,
) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError> {
    let llm_future = pool.chat_completion(request, route);
    let response = if let Some(tok) = cancel {
        tokio::select! {
            res = llm_future => res?,
            () = tok.cancelled() => {
                return Err(y_core::provider::ProviderError::Cancelled);
            }
        }
    } else {
        llm_future.await?
    };
    // Non-streaming: no reasoning duration tracking.
    Ok((response, None))
}

/// Call the LLM via streaming and emit `TurnEvent::StreamDelta` events.
///
/// Returns `(ChatResponse, Option<reasoning_duration_ms>)` -- the assembled
/// response plus the wall-clock reasoning duration if thinking content was
/// produced. Supports mid-stream cancellation via `CancellationToken`.
async fn call_llm_streaming(
    pool: &dyn ProviderPool,
    request: &ChatRequest,
    route: &RouteRequest,
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
    agent_name: &str,
    partial_out: &mut PartialStreamingContent,
) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError> {
    use y_core::provider::{ChatResponse, FinishReason, ProviderError};
    use y_core::types::TokenUsage;

    let stream_response = pool.chat_completion_stream(request, route).await?;
    let raw_request = stream_response.raw_request;
    let provider_id = stream_response.provider_id;
    let model_name = stream_response.model;
    let mut stream = stream_response.stream;

    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut tool_calls = Vec::new();
    let mut image_accumulator = ImageAccumulator::default();
    let mut usage = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: None,
        cache_write_tokens: None,
        ..Default::default()
    };
    let mut finish_reason = FinishReason::Stop;
    let mut finish_observed = false;
    let mut usage_observed = false;

    // Track reasoning timing: first reasoning delta -> first content delta.
    let mut reasoning_start: Option<std::time::Instant> = None;
    let mut reasoning_duration_ms: Option<u64> = None;

    // Whether the stream emitted any substance: content, reasoning, tool
    // calls, generated images, or a usage/finish event. A stream that
    // produced nothing of these is an "empty stream" -- a known failure
    // mode of Anthropic-compatible gateways under load (HTTP 200 with an
    // SSE body that contains no events). Such a stream must not be treated
    // as a valid empty response; it is surfaced as a transient ServerError
    // so the existing retry/resume machinery (provider pool retry,
    // plan-phase retry, user-initiated resend) can recover.

    loop {
        // Check cancellation between chunks.
        if let Some(tok) = cancel {
            if tok.is_cancelled() {
                partial_out.content = std::mem::take(&mut content);
                partial_out.reasoning = std::mem::take(&mut reasoning_content);
                return Err(ProviderError::Cancelled);
            }
        }

        let chunk_result = if let Some(tok) = cancel {
            tokio::select! {
                next = stream.next() => next,
                () = tok.cancelled() => {
                    partial_out.content = std::mem::take(&mut content);
                    partial_out.reasoning = std::mem::take(&mut reasoning_content);
                    return Err(ProviderError::Cancelled);
                }
            }
        } else {
            stream.next().await
        };

        match chunk_result {
            Some(Ok(chunk)) => {
                // Emit text delta to presentation layers.
                if let Some(ref delta) = chunk.delta_content {
                    if !delta.is_empty() {
                        // Mark end of reasoning on first content delta.
                        if let Some(start) = reasoning_start.take() {
                            reasoning_duration_ms =
                                Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0));
                        }
                        content.push_str(delta);
                        if let Some(tx) = progress {
                            let _ = tx.send(TurnEvent::StreamDelta {
                                content: delta.clone(),
                                agent_name: agent_name.to_string(),
                            });
                        }
                    }
                }

                // Emit reasoning/thinking delta.
                if let Some(ref reasoning) = chunk.delta_reasoning_content {
                    if !reasoning.is_empty() {
                        // Mark start of reasoning on first delta.
                        if reasoning_start.is_none() {
                            reasoning_start = Some(std::time::Instant::now());
                        }
                        reasoning_content.push_str(reasoning);
                        if let Some(tx) = progress {
                            let _ = tx.send(TurnEvent::StreamReasoningDelta {
                                content: reasoning.clone(),
                                agent_name: agent_name.to_string(),
                            });
                        }
                    }
                }

                for img_delta in &chunk.delta_images {
                    image_accumulator.push(img_delta);
                    if let Some(tx) = progress {
                        if img_delta.is_complete {
                            if let Some(image) = image_accumulator.complete_image(img_delta.index) {
                                let _ = tx.send(TurnEvent::StreamImageComplete {
                                    index: image.index,
                                    mime_type: image.mime_type,
                                    data: image.data,
                                    agent_name: agent_name.to_string(),
                                });
                            }
                        } else {
                            let _ = tx.send(TurnEvent::StreamImageDelta {
                                index: img_delta.index,
                                mime_type: img_delta.mime_type.clone(),
                                partial_data: img_delta.partial_data.clone(),
                                agent_name: agent_name.to_string(),
                            });
                        }
                    }
                }

                // Collect tool calls on finish.
                if !chunk.delta_tool_calls.is_empty() {
                    tool_calls.extend(chunk.delta_tool_calls);
                }

                // Capture usage from the final chunk.
                if let Some(u) = chunk.usage {
                    usage = u;
                    usage_observed = true;
                }

                if let Some(fr) = chunk.finish_reason {
                    finish_reason = fr;
                    finish_observed = true;
                }
            }
            Some(Err(e)) => return Err(e),
            None => break,
        }
    }

    // Build synthetic raw response for diagnostics.
    let generated_images = image_accumulator.into_images();
    let raw_response = build_streaming_raw_response(
        &model_name,
        &content,
        (!reasoning_content.is_empty()).then_some(reasoning_content.as_str()),
        &tool_calls,
        &generated_images,
        finish_reason,
        &usage,
    );

    // Detect an empty stream (200 OK but no SSE data). This happens under
    // provider overload: the connection succeeds, the SSE body is empty, and
    // the stream terminates without any content, tool calls, images, usage,
    // or finish event. Treating this as success yields a "(no content)"
    // response with zero usage -- the exact symptom reported. Classify it as
    // a transient ServerError so retry logic engages.
    let stream_was_empty = content.is_empty()
        && reasoning_content.is_empty()
        && tool_calls.is_empty()
        && generated_images.is_empty()
        && !usage_observed
        && !finish_observed;
    if stream_was_empty {
        let provider_name = provider_id
            .as_ref()
            .map_or_else(|| "unknown".to_string(), std::string::ToString::to_string);
        tracing::warn!(
            provider_id = ?provider_id,
            model = %model_name,
            "LLM stream produced no events (empty SSE body); treating as transient server error"
        );
        return Err(ProviderError::ServerError {
            provider: provider_name,
            message: "LLM stream produced no content (empty SSE body; likely server overload)"
                .to_string(),
        });
    }

    // If reasoning ended without any content delta (e.g. model produced
    // only reasoning), finalize the duration now.
    if let Some(start) = reasoning_start.take() {
        reasoning_duration_ms = Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0));
    }

    let response = ChatResponse {
        id: String::new(),
        content: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        reasoning_content: if reasoning_content.is_empty() {
            None
        } else {
            Some(reasoning_content)
        },
        model: model_name,
        tool_calls,
        finish_reason,
        usage,
        raw_request,
        raw_response: Some(raw_response),
        provider_id,
        generated_images,
    };
    Ok((response, reasoning_duration_ms))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::stream;
    use y_core::provider::{
        ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, GeneratedImage,
        ImageContentDelta, ProviderCapability, ProviderError, ProviderMetadata, ProviderPool,
        ProviderStatus, ProviderType, RequestMode, RoutePriority, RouteRequest, ToolCallingMode,
    };
    use y_core::types::{Message, ProviderId, Role, TokenUsage};

    use y_core::provider::FinishReason;
    use y_core::types::ToolCallRequest;

    use super::{
        build_route_requests, build_streaming_raw_response, call_llm,
        resolve_preflight_context_window, PartialStreamingContent, TurnEvent,
    };
    use crate::agent_service::AgentExecutionConfig;

    struct MockStreamingPool {
        provider_id: ProviderId,
        metadata: ProviderMetadata,
    }

    #[test]
    fn preflight_context_window_uses_smallest_tag_routed_candidate() {
        let metadata = [
            ProviderMetadata {
                id: ProviderId::from_string("large"),
                provider_type: ProviderType::OpenAi,
                model: "large-model".to_string(),
                tags: vec!["general".to_string()],
                capabilities: vec![ProviderCapability::Text],
                max_concurrency: 1,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode: ToolCallingMode::Native,
                tool_dialect: y_core::provider::ToolDialect::default(),
            },
            ProviderMetadata {
                id: ProviderId::from_string("small"),
                provider_type: ProviderType::OpenAi,
                model: "small-model".to_string(),
                tags: vec!["general".to_string()],
                capabilities: vec![ProviderCapability::Text],
                max_concurrency: 1,
                context_window: 32_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode: ToolCallingMode::Native,
                tool_dialect: y_core::provider::ToolDialect::default(),
            },
        ];
        let routes = [RouteRequest {
            preferred_model: Some("large-model".to_string()),
            required_tags: vec!["general".to_string()],
            ..RouteRequest::default()
        }];

        assert_eq!(resolve_preflight_context_window(&metadata, &routes), 32_000);
    }

    impl MockStreamingPool {
        fn new() -> Self {
            let provider_id = ProviderId::from_string("mock-stream");
            Self {
                provider_id: provider_id.clone(),
                metadata: ProviderMetadata {
                    id: provider_id,
                    provider_type: ProviderType::OpenAi,
                    model: "gpt-test".into(),
                    tags: vec!["reasoning".into()],
                    capabilities: vec![ProviderCapability::Text],
                    max_concurrency: 1,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    tool_calling_mode: ToolCallingMode::Native,
                    tool_dialect: y_core::provider::ToolDialect::default(),
                },
            }
        }
    }

    #[async_trait]
    impl ProviderPool for MockStreamingPool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            panic!("chat_completion should not be called in streaming tests");
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            let chunks = vec![
                Ok(ChatStreamChunk {
                    delta_content: None,
                    delta_reasoning_content: Some("step by step".into()),
                    delta_tool_calls: vec![],
                    usage: None,
                    finish_reason: None,
                    delta_images: vec![],
                }),
                Ok(ChatStreamChunk {
                    delta_content: Some("Final answer".into()),
                    delta_reasoning_content: None,
                    delta_tool_calls: vec![],
                    usage: Some(TokenUsage {
                        input_tokens: 12,
                        output_tokens: 5,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        ..Default::default()
                    }),
                    finish_reason: Some(FinishReason::Stop),
                    delta_images: vec![],
                }),
            ];

            Ok(ChatStreamResponse {
                stream: Box::pin(stream::iter(chunks)),
                raw_request: Some(serde_json::json!({ "messages": [] })),
                provider_id: Some(self.provider_id.clone()),
                model: self.metadata.model.clone(),
                context_window: self.metadata.context_window,
            })
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            vec![]
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingPool {
        routes: std::sync::Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl ProviderPool for RecordingPool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            let required_tags = route.required_tags.clone();
            self.routes
                .lock()
                .expect("routes mutex poisoned")
                .push(required_tags.clone());

            if required_tags == ["translation"] {
                return Err(ProviderError::NoProviderAvailable {
                    tags: required_tags,
                });
            }

            Ok(ChatResponse {
                id: "response-1".into(),
                model: "fallback-model".into(),
                content: Some("translated text".into()),
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 6,
                    output_tokens: 3,
                    ..Default::default()
                },
                finish_reason: FinishReason::Stop,
                raw_request: None,
                raw_response: None,
                provider_id: None,
                generated_images: vec![],
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            panic!("chat_completion_stream should not be called in non-streaming tests")
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            vec![]
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn test_request() -> ChatRequest {
        ChatRequest {
            messages: vec![Message {
                message_id: "msg-1".into(),
                role: Role::User,
                content: "Explain it".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            }],
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: Some(128),
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
        }
    }

    fn test_execution_config() -> AgentExecutionConfig {
        AgentExecutionConfig {
            agent_name: "translator".into(),
            system_prompt: "Translate".into(),
            max_iterations: 1,
            max_tool_calls: 0,
            tool_definitions: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: test_request().messages,
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec!["translation".into()],
            fallback_provider_tags: vec![vec!["general".into()]],
            request_mode: RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: Some(0.3),
            max_tokens: None,
            thinking: None,
            session_id: None,
            session_uuid: uuid::Uuid::nil(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: "hello".into(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_call_llm_falls_back_to_general_tags_when_translation_provider_missing() {
        let pool = RecordingPool::default();
        let request = test_request();
        let routes = build_route_requests(&test_execution_config());

        let (response, reasoning_duration_ms) = call_llm(
            &pool,
            &request,
            &routes,
            None,
            None,
            "translator",
            &mut PartialStreamingContent::default(),
        )
        .await
        .expect("fallback route should succeed");

        assert_eq!(response.content.as_deref(), Some("translated text"));
        assert_eq!(reasoning_duration_ms, None);
        let routes = pool.routes.lock().expect("routes mutex poisoned");
        assert_eq!(
            routes.as_slice(),
            &[vec!["translation".to_string()], vec!["general".to_string()]]
        );
    }

    #[test]
    fn test_build_streaming_raw_response_includes_tool_calls() {
        let raw = build_streaming_raw_response(
            "gpt-test",
            "working",
            None,
            &[ToolCallRequest {
                id: "call_123".into(),
                name: "Plan".into(),
                arguments: serde_json::json!({
                    "request": "Create a plan",
                }),
            }],
            &[],
            FinishReason::ToolUse,
            &TokenUsage {
                input_tokens: 123,
                output_tokens: 45,
                ..TokenUsage::default()
            },
        );

        // Provider-neutral envelope: no OpenAI `object`/`choices` wrapper.
        assert!(raw.get("object").is_none());
        assert!(raw.get("choices").is_none());
        assert_eq!(raw["finish_reason"], "tool_use");
        assert_eq!(raw["tool_calls"][0]["name"], "Plan");
        assert_eq!(
            raw["tool_calls"][0]["arguments"]["request"],
            "Create a plan"
        );
        // Unified token field names (not prompt_tokens/completion_tokens).
        assert_eq!(raw["usage"]["input_tokens"], 123);
        assert_eq!(raw["usage"]["output_tokens"], 45);
    }

    #[test]
    fn test_build_streaming_raw_response_includes_cache_tokens() {
        let raw = build_streaming_raw_response(
            "claude-test",
            "answer",
            None,
            &[],
            &[],
            FinishReason::Stop,
            &TokenUsage {
                input_tokens: 491,
                output_tokens: 145,
                cache_read_tokens: Some(80_384),
                cache_write_tokens: Some(0),
                ..TokenUsage::default()
            },
        );

        assert_eq!(raw["usage"]["input_tokens"], 491);
        assert_eq!(raw["usage"]["output_tokens"], 145);
        assert_eq!(raw["usage"]["cache_read_tokens"], 80_384);
        assert_eq!(raw["usage"]["cache_write_tokens"], 0);
    }

    #[tokio::test]
    async fn test_call_llm_streaming_preserves_reasoning_in_raw_response() {
        let pool = MockStreamingPool::new();
        let request = test_request();
        let route = RouteRequest {
            priority: RoutePriority::Normal,
            ..Default::default()
        };
        let (tx, _rx) = crate::chat::TurnEventSender::channel();

        let (response, reasoning_duration_ms) = call_llm(
            &pool,
            &request,
            &[route],
            Some(&tx),
            None,
            "chat-turn",
            &mut PartialStreamingContent::default(),
        )
        .await
        .expect("streaming call should succeed");

        assert_eq!(response.reasoning_content.as_deref(), Some("step by step"));
        assert_eq!(response.content.as_deref(), Some("Final answer"));
        assert!(reasoning_duration_ms.is_some());

        let raw_response = response
            .raw_response
            .expect("streaming call should synthesize raw response");
        assert_eq!(
            raw_response["reasoning_content"].as_str(),
            Some("step by step")
        );
        assert_eq!(raw_response["content"].as_str(), Some("Final answer"));
    }

    struct MockStreamingImagePool {
        provider_id: ProviderId,
        metadata: ProviderMetadata,
    }

    impl MockStreamingImagePool {
        fn new() -> Self {
            let provider_id = ProviderId::from_string("mock-image-stream");
            Self {
                provider_id: provider_id.clone(),
                metadata: ProviderMetadata {
                    id: provider_id,
                    provider_type: ProviderType::OpenAi,
                    model: "gpt-image-test".into(),
                    tags: vec!["image".into()],
                    capabilities: vec![ProviderCapability::ImageGeneration],
                    max_concurrency: 1,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    tool_calling_mode: ToolCallingMode::Native,
                    tool_dialect: y_core::provider::ToolDialect::default(),
                },
            }
        }
    }

    #[async_trait]
    impl ProviderPool for MockStreamingImagePool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            panic!("chat_completion should not be called in streaming image tests");
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            let chunks = vec![
                Ok(ChatStreamChunk {
                    delta_content: None,
                    delta_reasoning_content: None,
                    delta_tool_calls: vec![],
                    usage: None,
                    finish_reason: None,
                    delta_images: vec![ImageContentDelta {
                        index: 0,
                        mime_type: "image/png".into(),
                        partial_data: "iVBOR".into(),
                        is_complete: false,
                    }],
                }),
                Ok(ChatStreamChunk {
                    delta_content: Some("Here is the generated image.".into()),
                    delta_reasoning_content: None,
                    delta_tool_calls: vec![],
                    usage: None,
                    finish_reason: None,
                    delta_images: vec![ImageContentDelta {
                        index: 0,
                        mime_type: "image/png".into(),
                        partial_data: "w0KGgo=".into(),
                        is_complete: false,
                    }],
                }),
                Ok(ChatStreamChunk {
                    delta_content: None,
                    delta_reasoning_content: None,
                    delta_tool_calls: vec![],
                    usage: Some(TokenUsage {
                        input_tokens: 30,
                        output_tokens: 10,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        ..Default::default()
                    }),
                    finish_reason: Some(FinishReason::Stop),
                    delta_images: vec![ImageContentDelta {
                        index: 0,
                        mime_type: "image/png".into(),
                        partial_data: String::new(),
                        is_complete: true,
                    }],
                }),
            ];

            Ok(ChatStreamResponse {
                stream: Box::pin(stream::iter(chunks)),
                raw_request: Some(serde_json::json!({ "messages": [] })),
                provider_id: Some(self.provider_id.clone()),
                model: self.metadata.model.clone(),
                context_window: self.metadata.context_window,
            })
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            vec![]
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_call_llm_streaming_accumulates_generated_images_and_emits_image_events() {
        let pool = MockStreamingImagePool::new();
        let request = test_request();
        let route = RouteRequest {
            priority: RoutePriority::Normal,
            ..Default::default()
        };
        let (tx, mut rx) = crate::chat::TurnEventSender::channel();

        let (response, _reasoning_duration_ms) = call_llm(
            &pool,
            &request,
            &[route],
            Some(&tx),
            None,
            "chat-turn",
            &mut PartialStreamingContent::default(),
        )
        .await
        .expect("streaming image call should succeed");

        assert_eq!(
            response.content.as_deref(),
            Some("Here is the generated image.")
        );
        assert_eq!(
            response.generated_images,
            vec![GeneratedImage {
                index: 0,
                mime_type: "image/png".into(),
                data: "iVBORw0KGgo=".into(),
            }]
        );

        let mut saw_partial = false;
        let mut saw_complete = false;
        while let Ok((event, _session_id)) = rx.try_recv() {
            match event {
                TurnEvent::StreamImageDelta {
                    index,
                    mime_type,
                    partial_data,
                    agent_name,
                } => {
                    saw_partial = true;
                    assert_eq!(index, 0);
                    assert_eq!(mime_type, "image/png");
                    assert!(!partial_data.is_empty());
                    assert_eq!(agent_name, "chat-turn");
                }
                TurnEvent::StreamImageComplete {
                    index,
                    mime_type,
                    data,
                    agent_name,
                } => {
                    saw_complete = true;
                    assert_eq!(index, 0);
                    assert_eq!(mime_type, "image/png");
                    assert_eq!(data, "iVBORw0KGgo=");
                    assert_eq!(agent_name, "chat-turn");
                }
                TurnEvent::StreamDelta { .. }
                | TurnEvent::StreamReasoningDelta { .. }
                | TurnEvent::LlmResponse { .. }
                | TurnEvent::ToolStart { .. }
                | TurnEvent::ToolResult { .. }
                | TurnEvent::LoopLimitHit { .. }
                | TurnEvent::LlmError { .. }
                | TurnEvent::UserInteractionRequest { .. }
                | TurnEvent::PermissionRequest { .. }
                | TurnEvent::PlanReviewRequest { .. }
                | TurnEvent::SteerInjected { .. }
                | TurnEvent::FollowUpInjected { .. }
                | TurnEvent::Heartbeat { .. } => {}
            }
        }

        assert!(saw_partial);
        assert!(saw_complete);
    }

    /// A pool whose stream completes without emitting any chunks, content,
    /// tool calls, usage, or finish event. This is the exact failure mode
    /// observed under Anthropic-compatible gateway overload: HTTP 200 OK
    /// with an empty SSE body.
    struct EmptyStreamPool {
        provider_id: ProviderId,
        metadata: ProviderMetadata,
    }

    impl EmptyStreamPool {
        fn new() -> Self {
            let provider_id = ProviderId::from_string("empty-stream");
            Self {
                provider_id: provider_id.clone(),
                metadata: ProviderMetadata {
                    id: provider_id,
                    provider_type: ProviderType::Anthropic,
                    model: "claude-test".into(),
                    tags: vec!["general".into()],
                    capabilities: vec![ProviderCapability::Text],
                    max_concurrency: 1,
                    context_window: 1_000_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    tool_calling_mode: ToolCallingMode::Native,
                    tool_dialect: y_core::provider::ToolDialect::default(),
                },
            }
        }
    }

    #[async_trait]
    impl ProviderPool for EmptyStreamPool {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            panic!("chat_completion should not be called in streaming tests");
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            // No chunks at all -- the stream terminates immediately.
            let chunks: Vec<Result<ChatStreamChunk, ProviderError>> = vec![];
            Ok(ChatStreamResponse {
                stream: Box::pin(stream::iter(chunks)),
                raw_request: Some(serde_json::json!({ "messages": [] })),
                provider_id: Some(self.provider_id.clone()),
                model: self.metadata.model.clone(),
                context_window: self.metadata.context_window,
            })
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            vec![]
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_call_llm_streaming_empty_stream_returns_server_error() {
        // Regression: an Anthropic-compatible gateway under load can return
        // 200 OK with an empty SSE body (no events). Previously this was
        // silently treated as a valid empty response, producing a "(no
        // content)" turn with zero usage. It must now surface as a
        // transient ServerError so retry machinery can engage.
        let pool = EmptyStreamPool::new();
        let request = test_request();
        let route = RouteRequest {
            priority: RoutePriority::Normal,
            ..Default::default()
        };
        let (tx, _rx) = crate::chat::TurnEventSender::channel();

        let result = call_llm(
            &pool,
            &request,
            &[route],
            Some(&tx),
            None,
            "chat-turn",
            &mut PartialStreamingContent::default(),
        )
        .await;

        let err = result.expect_err("empty stream should surface as an error");
        match err {
            ProviderError::ServerError { provider, message } => {
                assert_eq!(provider, "empty-stream");
                assert!(
                    message.contains("empty SSE body"),
                    "error message should explain the empty SSE body: {message}"
                );
            }
            other => panic!("expected ServerError for empty stream, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_call_llm_streaming_finish_only_stream_is_not_empty() {
        // A stream that emits only a finish event (no content/usage) is
        // NOT an empty stream: the provider explicitly signalled
        // completion, so the empty-stream guard must not fire.
        struct FinishOnlyPool(EmptyStreamPool);
        #[async_trait]
        impl ProviderPool for FinishOnlyPool {
            async fn chat_completion(
                &self,
                _r: &ChatRequest,
                _route: &RouteRequest,
            ) -> Result<ChatResponse, ProviderError> {
                panic!()
            }
            async fn chat_completion_stream(
                &self,
                _r: &ChatRequest,
                _route: &RouteRequest,
            ) -> Result<ChatStreamResponse, ProviderError> {
                let chunks = vec![Ok(ChatStreamChunk {
                    delta_content: None,
                    delta_reasoning_content: None,
                    delta_tool_calls: vec![],
                    usage: None,
                    finish_reason: Some(FinishReason::Stop),
                    delta_images: vec![],
                })];
                Ok(ChatStreamResponse {
                    stream: Box::pin(stream::iter(chunks)),
                    raw_request: Some(serde_json::json!({})),
                    provider_id: Some(self.0.provider_id.clone()),
                    model: self.0.metadata.model.clone(),
                    context_window: self.0.metadata.context_window,
                })
            }
            fn report_error(&self, _p: &ProviderId, _e: &ProviderError) {}
            async fn provider_statuses(&self) -> Vec<ProviderStatus> {
                vec![]
            }
            async fn freeze(&self, _p: &ProviderId, _r: String) {}
            async fn thaw(&self, _p: &ProviderId) -> Result<(), ProviderError> {
                Ok(())
            }
        }
        let pool = FinishOnlyPool(EmptyStreamPool::new());
        let request = test_request();
        let route = RouteRequest {
            priority: RoutePriority::Normal,
            ..Default::default()
        };
        let (tx, _rx) = crate::chat::TurnEventSender::channel();

        let (response, _) = call_llm(
            &pool,
            &request,
            &[route],
            Some(&tx),
            None,
            "chat-turn",
            &mut PartialStreamingContent::default(),
        )
        .await
        .expect("finish-only stream should not be treated as empty");

        assert_eq!(response.content, None);
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }
}
