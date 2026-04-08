//! Native and prompt-based tool call handling, dynamic tool sync.

use y_core::provider::ToolCallingMode;
use y_core::types::{Message, Role, ToolCallRequest};
use y_tools::{format_tool_result, strip_tool_call_blocks, ParseResult};

use crate::container::ServiceContainer;

use super::result;
use super::tool_dispatch;
use super::{pruning, AgentExecutionConfig, LlmIterationData, ToolExecContext, TurnEventSender};

/// Handle native (function-calling) tool calls from an LLM response.
pub(crate) async fn handle_native_tool_calls(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    response: &y_core::provider::ChatResponse,
    progress: Option<&TurnEventSender>,
    data: &LlmIterationData,
    ctx: &mut ToolExecContext,
    context_window: usize,
) {
    let tc_names: Vec<String> = response
        .tool_calls
        .iter()
        .map(|tc| tc.name.clone())
        .collect();

    result::emit_llm_response(
        progress,
        response,
        data,
        ctx.iteration,
        tc_names,
        context_window,
        &config.agent_name,
    );

    // Track new messages added in this iteration for mid-loop persistence.
    let msgs_before = ctx.new_messages.len();

    // Even with Native tool calling, some providers/models embed XML tool
    // call blocks in the text content alongside structured tool_calls.
    // Strip them so raw protocol XML never leaks into the conversation.
    let iter_content = {
        let raw = response.content.clone().unwrap_or_default();
        let stripped = strip_tool_call_blocks(&raw);
        if stripped.is_empty() {
            raw
        } else {
            stripped
        }
    };

    // Accumulate this iteration's text so the final persisted message
    // includes all iterations' content. The raw content is preserved
    // as-is -- the frontend renders it sequentially.
    let out_content = if iter_content.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", iter_content.trim())
    };
    ctx.accumulated_content.push_str(&out_content);
    // Store per-iteration text separately so the frontend can interleave
    // text and tool cards by iteration order (without character offsets).
    // Always push (even empty) to stay parallel with iteration_reasonings.
    ctx.iteration_texts.push(out_content.clone());
    ctx.iteration_tool_counts.push(response.tool_calls.len());

    let assistant_msg =
        result::build_assistant_msg(response, out_content, response.tool_calls.clone());

    ctx.working_history.push(assistant_msg.clone());
    ctx.new_messages.push(assistant_msg);

    for tc in &response.tool_calls {
        let (_success, result_content) =
            tool_dispatch::execute_and_record_tool(container, config, tc, progress, ctx).await;

        let tool_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: result_content,
            tool_call_id: Some(tc.id.clone()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        ctx.working_history.push(tool_msg.clone());
        ctx.new_messages.push(tool_msg);
    }

    // If ToolSearch was called this iteration, sync newly activated
    // tool definitions so they appear in the next ChatRequest.tools.
    if response.tool_calls.iter().any(|tc| tc.name == "ToolSearch") {
        sync_dynamic_tool_defs(container, ctx).await;
    }

    // Mid-loop pruning: truncate large tool results from previous
    // iterations so context is managed at tool-call granularity.
    if config.use_context_pipeline {
        pruning::prune_working_history_mid_loop(container, ctx, msgs_before);
    }
}

/// Handle prompt-based tool calls parsed from LLM response text.
///
/// The fallback only affects how tool calls are *detected* (parsing XML from
/// text). The *result format* sent back to the LLM always follows the
/// provider's configured `tool_calling_mode`:
/// - **Native**: results are sent as `Role::Tool` messages with `tool_call_id`,
///   and the assistant message carries structured `tool_calls`.
/// - **`PromptBased`**: results are wrapped in `<tool_result>` XML and sent as a
///   single `Role::User` message.
pub(crate) async fn handle_prompt_based_tool_calls(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    response: &y_core::provider::ChatResponse,
    parse_result: &ParseResult,
    text: &str,
    progress: Option<&TurnEventSender>,
    data: &LlmIterationData,
    ctx: &mut ToolExecContext,
    context_window: usize,
) {
    let tc_names: Vec<String> = parse_result
        .tool_calls
        .iter()
        .map(|ptc| ptc.name.clone())
        .collect();

    result::emit_llm_response(
        progress,
        response,
        data,
        ctx.iteration,
        tc_names,
        context_window,
        &config.agent_name,
    );

    // Track new messages added in this iteration for mid-loop persistence.
    let msgs_before = ctx.new_messages.len();

    // The result format follows the provider config, not the detection method.
    let use_native_results = config.tool_calling_mode == ToolCallingMode::Native;

    // In Native mode, strip XML tool call blocks from the assistant content
    // so the working history stays in pure native format (the parsed calls
    // are attached via the `tool_calls` field instead). In PromptBased mode,
    // keep the raw text as-is (the XML blocks are the protocol).
    let out_content = if use_native_results {
        let stripped = strip_tool_call_blocks(text);
        if stripped.trim().is_empty() {
            String::new()
        } else {
            format!("{}\n", stripped.trim())
        }
    } else if text.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", text.trim())
    };

    ctx.accumulated_content.push_str(&out_content);
    // Always push (even empty) to stay parallel with iteration_reasonings.
    ctx.iteration_texts.push(out_content.clone());
    ctx.iteration_tool_counts
        .push(parse_result.tool_calls.len());

    // Pre-build ToolCallRequest objects with synthetic IDs.
    let tool_call_requests: Vec<ToolCallRequest> = parse_result
        .tool_calls
        .iter()
        .map(|ptc| ToolCallRequest {
            id: format!("prompt_{}", uuid::Uuid::new_v4()),
            name: ptc.name.clone(),
            arguments: ptc.arguments.clone(),
        })
        .collect();

    // In Native mode, attach parsed tool calls to the assistant message so
    // providers serialize them as structured function calls. In PromptBased
    // mode, the tool calls live in the text content.
    let assistant_tool_calls = if use_native_results {
        tool_call_requests.clone()
    } else {
        vec![]
    };

    let assistant_msg = result::build_assistant_msg(response, out_content, assistant_tool_calls);

    ctx.working_history.push(assistant_msg.clone());
    ctx.new_messages.push(assistant_msg);

    if use_native_results {
        // Native mode: execute each tool and send results as Role::Tool
        // messages with tool_call_id, matching handle_native_tool_calls.
        for tc in &tool_call_requests {
            let (_success, result_content) =
                tool_dispatch::execute_and_record_tool(container, config, tc, progress, ctx).await;

            let tool_msg = Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Tool,
                content: result_content,
                tool_call_id: Some(tc.id.clone()),
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            };
            ctx.working_history.push(tool_msg.clone());
            ctx.new_messages.push(tool_msg);
        }
    } else {
        // PromptBased mode: wrap results in XML and send as a single
        // Role::User message (original behavior).
        let mut result_blocks = Vec::new();
        for tc in &tool_call_requests {
            let (tool_success, result_content) =
                tool_dispatch::execute_and_record_tool(container, config, tc, progress, ctx).await;

            let result_value: serde_json::Value = serde_json::from_str(&result_content)
                .unwrap_or(serde_json::Value::String(result_content));
            result_blocks.push(format_tool_result(&tc.name, tool_success, &result_value));
        }

        let results_text = result_blocks.join("\n");
        let user_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: results_text,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({ "type": "tool_result" }),
        };
        ctx.working_history.push(user_msg.clone());
        ctx.new_messages.push(user_msg);
    }

    // If ToolSearch was called this iteration, sync newly activated
    // tool definitions so they appear in the next ChatRequest.tools.
    if parse_result
        .tool_calls
        .iter()
        .any(|ptc| ptc.name == "ToolSearch")
    {
        sync_dynamic_tool_defs(container, ctx).await;
    }

    // Mid-loop pruning: truncate large tool results from previous
    // iterations so context is managed at tool-call granularity.
    if config.use_context_pipeline {
        pruning::prune_working_history_mid_loop(container, ctx, msgs_before);
    }
}

/// Sync dynamically activated tool definitions from the `ToolActivationSet`
/// into `ctx.dynamic_tool_defs` so they appear in subsequent `ChatRequest.tools`.
///
/// Called after a `ToolSearch` call activates new tools. Also sets the
/// `orchestration.enabled` prompt flag when workflow/schedule tools are active.
pub(crate) async fn sync_dynamic_tool_defs(
    container: &ServiceContainer,
    ctx: &mut ToolExecContext,
) {
    use crate::container::ESSENTIAL_TOOL_NAMES;

    let essential: std::collections::HashSet<&str> = ESSENTIAL_TOOL_NAMES.iter().copied().collect();

    // When plan mode is active, also exclude blocked tools from dynamic defs.
    let plan_active = {
        let pctx = container.prompt_context.read().await;
        pctx.config_flags
            .get("plan_mode.active")
            .copied()
            .unwrap_or(false)
    };
    let blocked: std::collections::HashSet<&str> = if plan_active {
        use crate::container::PLAN_MODE_BLOCKED_TOOLS;
        PLAN_MODE_BLOCKED_TOOLS.iter().copied().collect()
    } else {
        std::collections::HashSet::new()
    };

    let set = container.tool_activation_set.read().await;
    let active = set.active_definitions();

    ctx.dynamic_tool_defs = active
        .iter()
        .filter(|def| {
            let name = def.name.as_str();
            !essential.contains(name) && !blocked.contains(name)
        })
        .map(|def| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": def.name.as_str(),
                    "description": def.description,
                    "parameters": def.parameters,
                }
            })
        })
        .collect();

    // If workflow/schedule tools were activated, set the orchestration
    // flag so the system prompt includes orchestration instructions on
    // subsequent turns.
    let has_orchestration = active.iter().any(|d| {
        let n = d.name.as_str();
        n.starts_with("workflow_") || n.starts_with("schedule_")
    });
    if has_orchestration {
        let mut pctx = container.prompt_context.write().await;
        pctx.config_flags
            .insert("orchestration.enabled".into(), true);
    }

    tracing::debug!(
        dynamic_count = ctx.dynamic_tool_defs.len(),
        "synced dynamic tool definitions from activation set"
    );
}
