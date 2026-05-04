//! Context pruning helpers for the agent execution loop.
//!
//! Includes mid-loop context pruning, tool history pruning, thinking strip,
//! and token estimation.

use y_core::types::{Message, Role};

use crate::container::ServiceContainer;

use super::{strip_think_tags, ToolExecContext};

/// Legacy mid-loop hard truncation hook.
///
/// This path used to rewrite large historical tool/user result payloads in
/// place with a `"... content truncated"` marker. In practice that damaged
/// useful context and conflicted with the designed intra-turn pruning model,
/// which should only remove retry noise (failed/empty/repeated branches).
///
/// The call sites are retained so the execution flow stays stable, but the
/// hard-truncation behavior is intentionally disabled.
pub(crate) fn prune_working_history_mid_loop(
    _container: &ServiceContainer,
    _ctx: &mut ToolExecContext,
    _msgs_before: usize,
) {
    // Intentionally disabled. The designed `IntraTurnPruner` already runs
    // before each iteration and handles retry-noise without rewriting
    // potentially useful historical content.
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
///
/// Safety rule: if any historical tool-calling assistant message does not
/// contain a non-empty rolling summary, pruning is skipped. Deleting tool
/// results without a surviving summary would destroy context.
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
                if trimmed.is_empty() {
                    return 0;
                }
                old_summaries.push(trimmed.to_string());
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::agent_service::ToolExecContext;
    use crate::chat::{PendingInteractions, PendingPermissions};
    use crate::config::ServiceConfig;
    use crate::container::ServiceContainer;
    use tempfile::TempDir;
    use y_core::types::{Role, SessionId, ToolCallRequest};

    async fn make_test_container() -> (ServiceContainer, TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("tempdir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    fn pending_interactions() -> PendingInteractions {
        Arc::new(Mutex::new(std::collections::HashMap::new()))
    }

    fn pending_permissions() -> PendingPermissions {
        Arc::new(Mutex::new(std::collections::HashMap::new()))
    }

    fn make_msg(role: Role, content: impl Into<String>) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_tool_msg(content: impl Into<String>) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some("tc_1".to_string()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_assistant_tool_msg(content: impl Into<String>, tool_call_id: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![ToolCallRequest {
                id: tool_call_id.to_string(),
                name: "FileRead".to_string(),
                arguments: serde_json::json!({ "path": "/tmp/test.txt" }),
            }],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_ctx(working_history: Vec<Message>) -> ToolExecContext {
        ToolExecContext {
            iteration: 1,
            last_gen_id: None,
            tool_calls_executed: vec![],
            new_messages: vec![],
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cost: 0.0,
            last_input_tokens: 0,
            trace_id: None,
            session_id: SessionId("test-session".into()),
            working_directory: None,
            working_history,
            accumulated_content: String::new(),
            iteration_texts: vec![],
            iteration_reasonings: vec![],
            iteration_reasoning_durations_ms: vec![],
            iteration_tool_counts: vec![],
            dynamic_tool_defs: vec![],
            pending_interactions: pending_interactions(),
            pending_permissions: pending_permissions(),
            cancel_token: None,
        }
    }

    #[tokio::test]
    async fn mid_loop_pruning_does_not_truncate_large_historical_tool_results() {
        let (container, _tmpdir) = make_test_container().await;
        let large_tool_output = "A".repeat(9_000);
        let mut original_history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "user asks for a large read"),
            make_tool_msg(large_tool_output.clone()),
        ];
        let mut ctx = make_ctx(original_history.clone());

        prune_working_history_mid_loop(&container, &mut ctx, 0);

        assert_eq!(ctx.working_history.len(), original_history.len());
        for (actual, expected) in ctx.working_history.iter().zip(original_history.drain(..)) {
            assert_eq!(actual.role, expected.role);
            assert_eq!(actual.content, expected.content);
            assert_eq!(actual.tool_call_id, expected.tool_call_id);
        }
    }

    #[test]
    fn prune_old_tool_results_merges_nonempty_historical_summaries() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "read two chunks"),
            make_assistant_tool_msg("CHUNK [0-99] | notes: first chunk", "tc_1"),
            make_tool_msg("first chunk raw output"),
            make_assistant_tool_msg("CHUNK [100-199] | notes: second chunk", "tc_2"),
            make_tool_msg("second chunk raw output"),
        ];

        let removed = prune_old_tool_results(&mut history);

        assert_eq!(removed, 2);
        assert_eq!(history.len(), 4);
        assert_eq!(history[0].role, Role::System);
        assert_eq!(history[1].role, Role::User);
        assert_eq!(history[2].role, Role::Assistant);
        assert_eq!(history[3].role, Role::Tool);
        assert!(history[2]
            .content
            .contains("CHUNK [0-99] | notes: first chunk"));
        assert!(history[2]
            .content
            .contains("CHUNK [100-199] | notes: second chunk"));
        assert_eq!(history[3].content, "second chunk raw output");
    }

    #[test]
    fn prune_old_tool_results_skips_when_historical_summary_is_missing() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "inspect files"),
            make_assistant_tool_msg("", "tc_1"),
            make_tool_msg("first raw output"),
            make_assistant_tool_msg("current rolling note", "tc_2"),
            make_tool_msg("second raw output"),
        ];
        let original = history.clone();

        let removed = prune_old_tool_results(&mut history);

        assert_eq!(removed, 0);
        assert_eq!(history.len(), original.len());
        for (actual, expected) in history.iter().zip(original.iter()) {
            assert_eq!(actual.role, expected.role);
            assert_eq!(actual.content, expected.content);
            assert_eq!(actual.tool_calls.len(), expected.tool_calls.len());
            assert_eq!(actual.tool_call_id, expected.tool_call_id);
        }
    }
}
