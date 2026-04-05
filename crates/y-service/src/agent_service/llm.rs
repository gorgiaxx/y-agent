//! LLM call dispatch -- streaming and non-streaming.
//!
//! Contains request building, routing, and the streaming chunk consumer.

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use y_core::provider::{ChatRequest, ProviderPool, RouteRequest};
use y_core::types::ProviderId;

use crate::cost::CostService;

use super::{AgentExecutionConfig, LlmIterationData, ToolExecContext, TurnEvent, TurnEventSender};

pub(crate) fn build_chat_request(
    config: &AgentExecutionConfig,
    ctx: &ToolExecContext,
) -> ChatRequest {
    // Merge essential (static) + dynamically activated tool definitions.
    let tools = if ctx.dynamic_tool_defs.is_empty() {
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
        messages: ctx.working_history.clone(),
        model: None,
        max_tokens: config.max_tokens,
        temperature: config.temperature,
        top_p: None,
        tools,
        tool_calling_mode: config.tool_calling_mode,
        stop: vec![],
        extra: serde_json::Value::Null,
        thinking: config.thinking.clone(),
    }
}

pub(crate) fn build_route_request(config: &AgentExecutionConfig) -> RouteRequest {
    RouteRequest {
        preferred_provider_id: config.provider_id.as_ref().map(ProviderId::from_string),
        preferred_model: config.preferred_models.first().cloned(),
        required_tags: config.provider_tags.clone(),
        ..RouteRequest::default()
    }
}

pub(crate) fn build_iteration_data(
    response: &y_core::provider::ChatResponse,
    fallback: &str,
    llm_start: std::time::Instant,
) -> LlmIterationData {
    let resp_input_tokens = u64::from(response.usage.input_tokens);
    let resp_output_tokens = u64::from(response.usage.output_tokens);
    let cost = CostService::compute_cost(resp_input_tokens, resp_output_tokens);
    let llm_elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);

    let prompt_preview = response.raw_request.as_ref().map_or_else(
        || fallback.to_string(),
        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    );

    let response_text_raw = response.raw_response.as_ref().map_or_else(
        || {
            serde_json::json!({
                "content": response.content.clone().unwrap_or_default(),
                "model": response.model,
                "usage": {
                    "input_tokens": resp_input_tokens,
                    "output_tokens": resp_output_tokens,
                }
            })
            .to_string()
        },
        std::string::ToString::to_string,
    );

    LlmIterationData {
        resp_input_tokens,
        resp_output_tokens,
        cost,
        llm_elapsed_ms,
        prompt_preview,
        response_text_raw,
    }
}

/// Dispatch to streaming or non-streaming LLM call.
///
/// Returns `(ChatResponse, Option<reasoning_duration_ms>)`. The duration
/// is only available when streaming is active and the model produced
/// reasoning content.
pub(crate) async fn call_llm(
    pool: &dyn ProviderPool,
    request: &ChatRequest,
    route: &RouteRequest,
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError> {
    if progress.is_some() {
        call_llm_streaming(pool, request, route, progress, cancel).await
    } else {
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
    let mut usage = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: None,
        cache_write_tokens: None,
        ..Default::default()
    };
    let mut finish_reason = FinishReason::Stop;

    // Track reasoning timing: first reasoning delta -> first content delta.
    let mut reasoning_start: Option<std::time::Instant> = None;
    let mut reasoning_duration_ms: Option<u64> = None;

    loop {
        // Check cancellation between chunks.
        if let Some(tok) = cancel {
            if tok.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
        }

        let chunk_result = if let Some(tok) = cancel {
            tokio::select! {
                next = stream.next() => next,
                () = tok.cancelled() => {
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
                }

                if let Some(fr) = chunk.finish_reason {
                    finish_reason = fr;
                }
            }
            Some(Err(e)) => return Err(e),
            None => break,
        }
    }

    // Build synthetic raw response for diagnostics.
    let finish_reason_str = match finish_reason {
        FinishReason::Length => "length",
        FinishReason::ToolUse => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Unknown | FinishReason::Stop => "stop",
    };
    let raw_response = serde_json::json!({
        "id": "",
        "object": "chat.completion",
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
            },
            "finish_reason": finish_reason_str,
        }],
        "usage": {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
        }
    });

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
    };
    Ok((response, reasoning_duration_ms))
}
