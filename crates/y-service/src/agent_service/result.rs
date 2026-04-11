//! Result building, progress events, and diagnostics recording.

use uuid::Uuid;

use y_core::types::{Message, Role, ToolCallRequest};
use y_tools::strip_tool_call_blocks;

use crate::container::ServiceContainer;

use super::{
    AgentExecutionConfig, AgentExecutionError, AgentExecutionResult, FinalResultParams,
    LlmIterationData, ToolExecContext, TurnEvent, TurnEventSender,
};

pub(crate) async fn emit_loop_limit(
    progress: Option<&TurnEventSender>,
    ctx: &ToolExecContext,
    max_iterations: usize,
    container: &ServiceContainer,
    owns_trace: bool,
) {
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::LoopLimitHit {
            iterations: ctx.iteration - 1,
            max_iterations,
        });
    }
    if owns_trace {
        if let Some(tid) = ctx.trace_id {
            let _ = container
                .diagnostics
                .on_trace_end(tid, false, Some("tool loop limit exceeded"))
                .await;
        }
    }
}

/// Handle an LLM call error: emit progress event, close trace, return error.
pub(crate) async fn handle_llm_error(
    error: y_core::provider::ProviderError,
    elapsed_ms: u64,
    model: &str,
    prompt_preview: &str,
    context_window: usize,
    progress: Option<&TurnEventSender>,
    container: &ServiceContainer,
    owns_trace: bool,
    ctx: &mut ToolExecContext,
    agent_name: &str,
) -> Result<AgentExecutionResult, AgentExecutionError> {
    if matches!(error, y_core::provider::ProviderError::Cancelled) {
        return Err(AgentExecutionError::Cancelled {
            partial_messages: std::mem::take(&mut ctx.new_messages),
            accumulated_content: std::mem::take(&mut ctx.accumulated_content),
            iteration_texts: std::mem::take(&mut ctx.iteration_texts),
            iteration_reasonings: std::mem::take(&mut ctx.iteration_reasonings),
            iteration_reasoning_durations_ms: std::mem::take(
                &mut ctx.iteration_reasoning_durations_ms,
            ),
            iteration_tool_counts: std::mem::take(&mut ctx.iteration_tool_counts),
            tool_calls_executed: std::mem::take(&mut ctx.tool_calls_executed),
            iterations: ctx.iteration.saturating_sub(1),
            input_tokens: ctx.cumulative_input_tokens,
            output_tokens: ctx.cumulative_output_tokens,
            cost_usd: ctx.cumulative_cost,
            model: model.to_string(),
        });
    }

    // Emit LlmError progress event so the diagnostics panel
    // records the failed call before the progress channel closes.
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::LlmError {
            iteration: ctx.iteration,
            error: format!("{error}"),
            duration_ms: elapsed_ms,
            model: model.to_string(),
            prompt_preview: prompt_preview.to_string(),
            context_window,
            agent_name: agent_name.to_string(),
        });
    }

    if owns_trace {
        if let Some(tid) = ctx.trace_id {
            let _ = container
                .diagnostics
                .on_trace_end(tid, false, Some(&error.to_string()))
                .await;
        }
    }
    let partial = std::mem::take(&mut ctx.new_messages);
    Err(AgentExecutionError::LlmError {
        message: format!("{error}"),
        partial_messages: partial,
    })
}

/// Record a generation observation in the diagnostics subsystem.
pub(crate) async fn record_generation_diagnostics(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    response: &y_core::provider::ChatResponse,
    prompt_preview_fallback: &str,
    data: &LlmIterationData,
    last_gen_id: &mut Option<Uuid>,
    trace_id: Option<Uuid>,
) {
    let Some(tid) = trace_id else { return };

    let diag_input = response.raw_request.clone().unwrap_or_else(|| {
        serde_json::from_str(prompt_preview_fallback).unwrap_or(serde_json::Value::Null)
    });
    let diag_output = response.raw_response.clone().unwrap_or_else(|| {
        let mut output = serde_json::json!({
            "content": response.content.clone().unwrap_or_default(),
            "model": response.model,
            "usage": {
                "input_tokens": data.resp_input_tokens,
                "output_tokens": data.resp_output_tokens,
            }
        });
        if let Some(reasoning) = response.reasoning_content.as_ref() {
            output["reasoning_content"] = serde_json::Value::String(reasoning.clone());
        }
        output
    });

    let gen_id = container
        .diagnostics
        .on_generation(y_diagnostics::GenerationParams {
            trace_id: tid,
            parent_id: None,
            session_id: Some(config.session_uuid),
            model: response.model.clone(),
            input_tokens: data.resp_input_tokens,
            output_tokens: data.resp_output_tokens,
            cost_usd: data.cost,
            input: diag_input,
            output: diag_output,
            duration_ms: data.llm_elapsed_ms,
        })
        .await
        .ok();
    *last_gen_id = gen_id;

    tracing::debug!(
        trace_id = %tid,
        agent = %config.agent_name,
        model = %response.model,
        input_tokens = data.resp_input_tokens,
        output_tokens = data.resp_output_tokens,
        llm_ms = data.llm_elapsed_ms,
        "Diagnostics: agent LLM call recorded"
    );
}

/// Emit `LlmResponse` progress event with the given tool call names.
pub(crate) fn emit_llm_response(
    progress: Option<&TurnEventSender>,
    response: &y_core::provider::ChatResponse,
    data: &LlmIterationData,
    iteration: usize,
    tool_call_names: Vec<String>,
    context_window: usize,
    agent_name: &str,
) {
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::LlmResponse {
            iteration,
            model: response.model.clone(),
            input_tokens: data.resp_input_tokens,
            output_tokens: data.resp_output_tokens,
            duration_ms: data.llm_elapsed_ms,
            cost_usd: data.cost,
            tool_calls_requested: tool_call_names,
            prompt_preview: data.prompt_preview.clone(),
            response_text: data.response_text_raw.clone(),
            context_window,
            agent_name: agent_name.to_string(),
        });
    }
}

/// Build an assistant `Message` with reasoning metadata.
pub(crate) fn build_assistant_msg(
    response: &y_core::provider::ChatResponse,
    content: String,
    tool_calls: Vec<ToolCallRequest>,
) -> Message {
    let mut meta = serde_json::json!({ "model": response.model });
    if let Some(ref rc) = response.reasoning_content {
        meta["reasoning_content"] = serde_json::Value::String(rc.clone());
    }
    Message {
        message_id: y_core::types::generate_message_id(),
        role: Role::Assistant,
        content,
        tool_call_id: None,
        tool_calls,
        timestamp: y_core::types::now(),
        metadata: meta,
    }
}

/// Build the final result when no tool calls are requested.
pub(crate) async fn build_final_result(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    response: &y_core::provider::ChatResponse,
    progress: Option<&TurnEventSender>,
    data: &LlmIterationData,
    ctx: ToolExecContext,
    params: FinalResultParams,
) -> Result<AgentExecutionResult, AgentExecutionError> {
    let FinalResultParams {
        final_model,
        final_provider_id,
        owns_trace,
        context_window: ctx_window,
        reasoning_duration_ms,
    } = params;
    let raw_content = response
        .content
        .clone()
        .unwrap_or_else(|| "(no content)".to_string());

    emit_llm_response(
        progress,
        response,
        data,
        ctx.iteration,
        vec![],
        ctx_window,
        &config.agent_name,
    );

    // Always strip XML tool call blocks regardless of tool calling mode.
    let content = {
        let stripped = strip_tool_call_blocks(&raw_content);
        if stripped.is_empty() {
            raw_content
        } else {
            stripped
        }
    };

    if owns_trace {
        if let Some(tid) = ctx.trace_id {
            let _ = container
                .diagnostics
                .on_trace_end(tid, true, Some(&content))
                .await;
        }
    }

    let final_content = if ctx.accumulated_content.is_empty() {
        content.clone()
    } else {
        format!("{}{content}", ctx.accumulated_content)
    };

    Ok(AgentExecutionResult {
        content: final_content,
        model: final_model,
        provider_id: final_provider_id,
        input_tokens: ctx.cumulative_input_tokens,
        output_tokens: ctx.cumulative_output_tokens,
        last_input_tokens: ctx.last_input_tokens,
        context_window: ctx_window,
        cost_usd: ctx.cumulative_cost,
        tool_calls_executed: ctx.tool_calls_executed,
        iterations: ctx.iteration,
        new_messages: ctx.new_messages,
        final_response: content.clone(),
        iteration_texts: ctx.iteration_texts,
        iteration_reasonings: ctx.iteration_reasonings,
        iteration_reasoning_durations_ms: ctx.iteration_reasoning_durations_ms,
        iteration_tool_counts: ctx.iteration_tool_counts,
        reasoning_content: response.reasoning_content.clone(),
        reasoning_duration_ms,
    })
}
