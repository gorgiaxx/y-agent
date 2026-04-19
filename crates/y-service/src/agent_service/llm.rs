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
        response_format: config.response_format.clone(),
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
            let mut response_json = serde_json::json!({
                "content": response.content.clone().unwrap_or_default(),
                "model": response.model,
                "usage": {
                    "input_tokens": resp_input_tokens,
                    "output_tokens": resp_output_tokens,
                }
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
        cost,
        llm_elapsed_ms,
        prompt_preview,
        response_text_raw,
    }
}

fn build_streaming_raw_response(
    model_name: &str,
    content: &str,
    reasoning_content: Option<&str>,
    tool_calls: &[y_core::types::ToolCallRequest],
    finish_reason: &y_core::provider::FinishReason,
    input_tokens: u64,
    output_tokens: u64,
) -> serde_json::Value {
    let finish_reason_str = match finish_reason {
        y_core::provider::FinishReason::Length => "length",
        y_core::provider::FinishReason::ToolUse => "tool_calls",
        y_core::provider::FinishReason::ContentFilter => "content_filter",
        y_core::provider::FinishReason::Unknown | y_core::provider::FinishReason::Stop => "stop",
    };

    let tool_calls_json: Vec<serde_json::Value> = tool_calls
        .iter()
        .map(|tool_call| {
            serde_json::json!({
                "id": tool_call.id,
                "type": "function",
                "function": {
                    "name": tool_call.name,
                    "arguments": tool_call.arguments,
                }
            })
        })
        .collect();

    let mut message = serde_json::json!({
        "role": "assistant",
        "content": content,
    });
    if let Some(reasoning) = reasoning_content.filter(|value| !value.is_empty()) {
        message["reasoning_content"] = serde_json::Value::String(reasoning.to_string());
    }
    if !tool_calls_json.is_empty() {
        message["tool_calls"] = serde_json::Value::Array(tool_calls_json);
    }

    serde_json::json!({
        "id": "",
        "object": "chat.completion",
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason_str,
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
        }
    })
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
    agent_name: &str,
) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError> {
    if progress.is_some() {
        call_llm_streaming(pool, request, route, progress, cancel, agent_name).await
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
    agent_name: &str,
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
    let raw_response = build_streaming_raw_response(
        &model_name,
        &content,
        (!reasoning_content.is_empty()).then_some(reasoning_content.as_str()),
        &tool_calls,
        &finish_reason,
        u64::from(usage.input_tokens),
        u64::from(usage.output_tokens),
    );

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
        generated_images: vec![],
    };
    Ok((response, reasoning_duration_ms))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::stream;
    use tokio::sync::mpsc;
    use y_core::provider::{
        ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, ProviderError,
        ProviderMetadata, ProviderPool, ProviderStatus, ProviderType, RoutePriority, RouteRequest,
        ToolCallingMode,
    };
    use y_core::types::{Message, ProviderId, Role, TokenUsage};

    use y_core::provider::FinishReason;
    use y_core::types::ToolCallRequest;

    use super::{build_streaming_raw_response, call_llm, TurnEvent};

    struct MockStreamingPool {
        provider_id: ProviderId,
        metadata: ProviderMetadata,
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
                    max_concurrency: 1,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    tool_calling_mode: ToolCallingMode::Native,
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
            max_tokens: Some(128),
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
        }
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
            &FinishReason::ToolUse,
            123,
            45,
        );

        assert_eq!(raw["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            raw["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "Plan"
        );
        assert_eq!(
            raw["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]["request"],
            "Create a plan"
        );
    }

    #[tokio::test]
    async fn test_call_llm_streaming_preserves_reasoning_in_raw_response() {
        let pool = MockStreamingPool::new();
        let request = test_request();
        let route = RouteRequest {
            priority: RoutePriority::Normal,
            ..Default::default()
        };
        let (tx, _rx) = mpsc::unbounded_channel::<TurnEvent>();

        let (response, reasoning_duration_ms) =
            call_llm(&pool, &request, &route, Some(&tx), None, "chat-turn")
                .await
                .expect("streaming call should succeed");

        assert_eq!(response.reasoning_content.as_deref(), Some("step by step"));
        assert_eq!(response.content.as_deref(), Some("Final answer"));
        assert!(reasoning_duration_ms.is_some());

        let raw_response = response
            .raw_response
            .expect("streaming call should synthesize raw response");
        assert_eq!(
            raw_response["choices"][0]["message"]["reasoning_content"].as_str(),
            Some("step by step")
        );
        assert_eq!(
            raw_response["choices"][0]["message"]["content"].as_str(),
            Some("Final answer")
        );
    }
}
