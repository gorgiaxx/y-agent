//! Context pruning helpers for the agent execution loop.
//!
//! Includes mid-loop context pruning, tool history pruning, thinking strip,
//! and token estimation.

use y_core::types::{Message, Role};

use crate::container::ServiceContainer;

use super::{strip_think_tags, ToolExecContext};

/// Estimate token count for a message (content + role overhead).
pub(crate) fn estimate_msg_tokens(msg: &Message) -> u32 {
    estimate_msg_tokens_from_str(&msg.content) + 4 // role/separator overhead
}

/// Estimate token count from text.
///
/// Uses `chars().count()` rather than `len()` (byte count) so multi-byte
/// scripts (Chinese, Japanese, Korean) produce accurate estimates instead
/// of being inflated by UTF-8 encoding overhead.
fn estimate_msg_tokens_from_str(text: &str) -> u32 {
    u32::try_from(text.chars().count().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Mid-loop context pruning: truncates large tool result messages from
/// previous iterations when total `working_history` tokens exceed the
/// configured pruning threshold.
///
/// Operates entirely in-memory on `working_history` -- no
/// `ChatMessageStore` dependency. This is correct because the agentic
/// loop builds LLM requests from `working_history`, not from persistent
/// storage.
///
/// Only called for the root chat agent (`use_context_pipeline == true`).
///
/// Strategy:
/// 1. Estimate total tokens in `working_history`
/// 2. If total exceeds threshold, find tool/user result messages from
///    *previous* iterations (protect current iteration's messages)
/// 3. Sort candidates by token size descending
/// 4. Truncate the largest messages until total is under the threshold
pub(crate) fn prune_working_history_mid_loop(
    container: &ServiceContainer,
    ctx: &mut ToolExecContext,
    msgs_before: usize,
) {
    let config = container.pruning_engine.config();
    if !config.enabled {
        return;
    }

    // Per-message token limit: individual tool results larger than this
    // are truncated immediately. Uses the pruning token_threshold as the
    // per-message cap (default 2000 tokens = ~8K chars).
    let per_message_limit = config.token_threshold;

    // Overall context budget: when total working_history exceeds this,
    // the largest old tool results are truncated greedily.
    // Default: 10x the per-message limit = 20K tokens.
    let context_budget = per_message_limit.saturating_mul(10);

    // Estimate total tokens in working_history.
    let total_tokens: u32 = ctx.working_history.iter().map(estimate_msg_tokens).sum();

    if total_tokens < context_budget {
        // Total is under budget; skip the overall truncation pass but still
        // check individual large messages below.
    }

    // Collect IDs of messages added in the current iteration -- protected.
    let current_iteration_ids: std::collections::HashSet<String> = ctx.new_messages[msgs_before..]
        .iter()
        .map(|m| m.message_id.clone())
        .collect();

    // Build candidate list: any non-system, non-assistant message from
    // previous iterations.
    let mut candidates: Vec<(usize, u32)> = ctx
        .working_history
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            !current_iteration_ids.contains(&m.message_id)
                && m.role != Role::System
                && m.role != Role::Assistant
        })
        .map(|(idx, m)| (idx, estimate_msg_tokens(m)))
        .filter(|(_, tokens)| *tokens > 200) // Only truncate messages worth truncating
        .collect();

    // Sort by token count descending so we truncate the largest first.
    candidates.sort_by(|a, b| b.1.cmp(&a.1));

    let mut truncated_count = 0u32;
    let mut tokens_saved = 0u32;
    let over_budget = total_tokens > context_budget;

    for (idx, original_tokens) in &candidates {
        // Two conditions to truncate:
        // 1. Per-message: message exceeds per_message_limit (always truncate)
        // 2. Budget: total working_history exceeds context_budget
        if *original_tokens <= per_message_limit && !over_budget {
            continue;
        }
        // If we're in budget mode only (not per-message), stop once we've
        // reclaimed enough.
        if *original_tokens <= per_message_limit
            && tokens_saved >= total_tokens.saturating_sub(context_budget)
        {
            break;
        }

        let msg = &ctx.working_history[*idx];
        let content = &msg.content;

        // Keep first 200 and last 100 chars, replace the rest with a marker.
        let keep_head = 200.min(content.len());
        let keep_tail = 100.min(content.len().saturating_sub(keep_head));

        if content.len() <= keep_head + keep_tail + 50 {
            continue;
        }

        let head = &content[..content.floor_char_boundary(keep_head)];
        let tail_start = content.ceil_char_boundary(content.len() - keep_tail);
        let tail = &content[tail_start..];
        let truncated = format!(
            "{head}\n\n[... content truncated ({original_tokens} tokens -> ~100 tokens) ...]\n\n{tail}"
        );

        let new_tokens = estimate_msg_tokens_from_str(&truncated);
        let saved = original_tokens.saturating_sub(new_tokens);

        ctx.working_history[*idx].content = truncated;
        tokens_saved += saved;
        truncated_count += 1;
    }

    if truncated_count > 0 {
        tracing::info!(
            session_id = %ctx.session_id,
            total_tokens_before = total_tokens,
            per_message_limit,
            context_budget,
            messages_truncated = truncated_count,
            tokens_saved,
            "mid-loop pruning: truncated large tool results in working_history"
        );
    }
}

// -----------------------------------------------------------------------
// Working history pruning helpers
// -----------------------------------------------------------------------

/// Merge-and-prune historical tool call pairs from `working_history`.
///
/// For agents that build incremental summaries (e.g. `knowledge-summarizer`),
/// each assistant response contains a summary of the tool result it just
/// processed. This function:
///
/// 1. **Collects** text content (stripped of `<think>` tags) from all
///    assistant messages with `tool_calls` that appear *before* the latest
///    assistant message.
/// 2. **Merges** those summaries into the latest assistant message by
///    prepending them, so the accumulated context is preserved in a single
///    assistant message.
/// 3. **Removes** the old assistant+tool message pairs.
///
/// The net effect: the LLM request always contains at most **one**
/// assistant message (with the accumulated rolling summary) and **one**
/// tool result (the most recent chunk). System and User messages are
/// never removed.
pub(crate) fn prune_old_tool_results(working_history: &mut Vec<Message>) -> usize {
    let last_assistant_idx = working_history
        .iter()
        .rposition(|m| m.role == Role::Assistant);

    let Some(last_idx) = last_assistant_idx else {
        return 0;
    };

    // Pass 1: collect old summaries and mark indices for removal.
    let mut old_summaries: Vec<String> = Vec::new();
    let mut indices_to_remove: Vec<usize> = Vec::new();

    for (i, msg) in working_history.iter().enumerate() {
        if i >= last_idx {
            break;
        }
        match msg.role {
            Role::Assistant if !msg.tool_calls.is_empty() => {
                let stripped = strip_think_tags(&msg.content);
                let trimmed = stripped.trim();
                if !trimmed.is_empty() {
                    old_summaries.push(trimmed.to_string());
                }
                indices_to_remove.push(i);
            }
            Role::Tool => {
                indices_to_remove.push(i);
            }
            _ => {}
        }
    }

    if indices_to_remove.is_empty() {
        return 0;
    }

    // Pass 2: merge old summaries into the latest assistant message.
    if !old_summaries.is_empty() {
        let current_content = &working_history[last_idx].content;
        let merged = format!("{}\n\n{}", old_summaries.join("\n\n"), current_content);
        working_history[last_idx].content = merged;
    }

    // Pass 3: remove old messages (reverse order to preserve indices).
    let removed = indices_to_remove.len();
    for &idx in indices_to_remove.iter().rev() {
        working_history.remove(idx);
    }

    removed
}

/// Strip thinking/reasoning content from historical assistant messages.
///
/// Two forms are handled:
/// 1. `<think>...</think>` tags in `message.content` -- stripped
/// 2. `metadata.reasoning_content` field -- removed from metadata JSON
///
/// Only processes assistant messages that are NOT the most recent one.
/// The latest assistant message's thinking is preserved because the
/// current iteration result should not be altered.
pub(crate) fn strip_historical_thinking(working_history: &mut [Message]) {
    let last_assistant_idx = working_history
        .iter()
        .rposition(|m| m.role == Role::Assistant);

    for (i, msg) in working_history.iter_mut().enumerate() {
        if msg.role != Role::Assistant {
            continue;
        }
        // Protect the most recent assistant message.
        if Some(i) == last_assistant_idx {
            continue;
        }

        // 1. Strip <think>...</think> from content.
        if msg.content.contains("<think>") {
            msg.content = strip_think_tags(&msg.content);
        }

        // 2. Remove reasoning_content from metadata.
        if let Some(obj) = msg.metadata.as_object_mut() {
            obj.remove("reasoning_content");
        }
    }
}
