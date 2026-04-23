//! Intra-turn pruning: removes failed tool call branches from the in-memory
//! `working_history` between tool call iterations.
//!
//! Design reference: context-pruning-design.md, Intra-Turn Pruning section.
//!
//! Operates on `Vec<Message>` (no persistence). Uses the same detection
//! heuristics as post-turn `RetryPruning` via the shared `patterns` module.
//! Only retry-style pruning runs intra-turn (zero LLM cost).

use std::collections::HashSet;

use y_core::types::{Message, Role};

use super::config::IntraTurnPruningConfig;
use super::patterns::{
    content_similarity, estimate_tokens, matches_empty_patterns, matches_error_patterns,
    MAX_ADJACENT_DISTANCE, SIMILARITY_THRESHOLD,
};

/// Report from intra-turn pruning.
#[derive(Debug, Clone)]
pub struct IntraTurnPruningReport {
    /// Number of messages removed from working history.
    pub messages_removed: usize,
    /// Estimated tokens saved.
    pub tokens_saved: u32,
    /// Whether pruning was skipped (below threshold or iteration gate).
    pub skipped: bool,
}

impl IntraTurnPruningReport {
    fn skipped() -> Self {
        Self {
            messages_removed: 0,
            tokens_saved: 0,
            skipped: true,
        }
    }
}

/// Prunes failed tool call branches from in-memory working history.
///
/// Constructed from `IntraTurnPruningConfig`. Called once per iteration of
/// the agent execution loop, before `build_chat_request`.
pub struct IntraTurnPruner {
    enabled: bool,
    min_iteration: u32,
    token_threshold: u32,
    extra_patterns: Vec<String>,
}

impl IntraTurnPruner {
    /// Create from configuration.
    pub fn from_config(config: &IntraTurnPruningConfig) -> Self {
        Self {
            enabled: config.enabled,
            min_iteration: config.min_iteration,
            token_threshold: config.token_threshold,
            extra_patterns: Vec::new(),
        }
    }

    /// Create from configuration with extra heuristic patterns.
    pub fn from_config_with_patterns(
        config: &IntraTurnPruningConfig,
        extra_patterns: Vec<String>,
    ) -> Self {
        Self {
            enabled: config.enabled,
            min_iteration: config.min_iteration,
            token_threshold: config.token_threshold,
            extra_patterns,
        }
    }

    /// Prune failed tool call branches from working history in-place.
    ///
    /// Returns a report with pruning stats. Does not modify `new_messages`
    /// -- only `working_history` is affected. The caller is responsible for
    /// ensuring `new_messages` retains all messages for display persistence.
    pub fn prune_working_history(
        &self,
        working_history: &mut Vec<Message>,
        iteration: usize,
    ) -> IntraTurnPruningReport {
        if !self.enabled {
            return IntraTurnPruningReport::skipped();
        }

        if (iteration as u32) < self.min_iteration {
            return IntraTurnPruningReport::skipped();
        }

        // Collect message IDs to remove and their token costs.
        let mut remove_ids: HashSet<String> = HashSet::new();
        let mut candidate_tokens: u32 = 0;

        // Find the index of the last assistant+tool boundary to protect it.
        let last_assistant_idx = Self::find_last_assistant_tool_boundary(working_history);

        // Signal 1: Error tool results.
        self.detect_error_tool_results(
            working_history,
            last_assistant_idx,
            &mut remove_ids,
            &mut candidate_tokens,
        );

        // Signal 2: Repeated similar assistant calls.
        Self::detect_repeated_calls(
            working_history,
            last_assistant_idx,
            &mut remove_ids,
            &mut candidate_tokens,
        );

        // Signal 3: Empty tool results.
        Self::detect_empty_tool_results(
            working_history,
            last_assistant_idx,
            &mut remove_ids,
            &mut candidate_tokens,
        );

        if remove_ids.is_empty() || candidate_tokens < self.token_threshold {
            return IntraTurnPruningReport::skipped();
        }

        // Ensure structural integrity: if removing an assistant with tool_calls,
        // also remove all matching tool results. If removing a tool result, also
        // remove the parent assistant.
        Self::ensure_structural_integrity(working_history, &mut remove_ids);

        // Recalculate tokens after structural integrity adjustments.
        let tokens_saved: u32 = working_history
            .iter()
            .filter(|m| remove_ids.contains(&m.message_id))
            .map(|m| estimate_tokens(&m.content))
            .sum();

        let messages_removed = remove_ids.len();
        working_history.retain(|m| !remove_ids.contains(&m.message_id));

        IntraTurnPruningReport {
            messages_removed,
            tokens_saved,
            skipped: false,
        }
    }

    /// Find the index of the last assistant message that has a following tool
    /// result. Returns `None` if no such boundary exists. Messages at or after
    /// this index are protected from pruning.
    fn find_last_assistant_tool_boundary(messages: &[Message]) -> Option<usize> {
        for i in (0..messages.len()).rev() {
            if messages[i].role == Role::Assistant {
                // Check if there is a tool result following this assistant message.
                let has_tool_result = (i + 1 < messages.len()
                    && messages[i + 1].role == Role::Tool)
                    || messages[i].tool_calls.iter().any(|tc| {
                        messages[i + 1..]
                            .iter()
                            .any(|m| m.tool_call_id.as_deref() == Some(&tc.id))
                    });
                if has_tool_result {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Signal 1: Find tool messages with error patterns.
    ///
    /// Skips tool results whose parent assistant carries `tool_calls`. The
    /// error-pattern heuristic (`"error:"`, `"not found"`, etc.) fires on
    /// informative outcomes (e.g. `FileRead` on a missing path) that the LLM
    /// needs to see to avoid re-requesting the same call. Removing the pair
    /// would strip the `tool_call` from history and trigger an infinite loop
    /// where the model re-invokes the call, gets the same "error" result,
    /// and repeats. Pure-text retry noise is left to Signal 2.
    fn detect_error_tool_results(
        &self,
        messages: &[Message],
        last_boundary: Option<usize>,
        remove_ids: &mut HashSet<String>,
        candidate_tokens: &mut u32,
    ) {
        for (i, msg) in messages.iter().enumerate() {
            if msg.role != Role::Tool {
                continue;
            }
            if Self::is_protected(i, last_boundary) {
                continue;
            }
            // Never prune a tool result whose parent assistant carries
            // tool_calls: the call + result pair is load-bearing context.
            if i > 0
                && messages[i - 1].role == Role::Assistant
                && !messages[i - 1].tool_calls.is_empty()
            {
                continue;
            }
            if matches_error_patterns(&msg.content, &self.extra_patterns) {
                let tokens = estimate_tokens(&msg.content);
                remove_ids.insert(msg.message_id.clone());
                *candidate_tokens += tokens;

                // Also mark the preceding assistant if it immediately precedes.
                if i > 0
                    && messages[i - 1].role == Role::Assistant
                    && !Self::is_protected(i - 1, last_boundary)
                {
                    remove_ids.insert(messages[i - 1].message_id.clone());
                    *candidate_tokens += estimate_tokens(&messages[i - 1].content);
                }
            }
        }
    }

    /// Signal 2: Find repeated similar assistant messages.
    ///
    /// Never prunes an assistant message that carries `tool_calls`. Similarity
    /// pruning targets pure-text retry noise; an assistant with `tool_calls`
    /// represents a real tool invocation whose result is referenced by later
    /// messages. Dropping it (and its paired `Role::Tool` result) would leave
    /// the LLM with no record of the call and trigger re-invocation in the
    /// next iteration -- the symptom being an infinite tool-call loop.
    ///
    /// Legitimate progressive sequences (e.g. consecutive `FileRead` with
    /// increasing `line_offset`) and even identical retry calls are preserved;
    /// the error/empty-result signals (1 and 3) already handle genuine failures.
    fn detect_repeated_calls(
        messages: &[Message],
        last_boundary: Option<usize>,
        remove_ids: &mut HashSet<String>,
        candidate_tokens: &mut u32,
    ) {
        let assistant_msgs: Vec<(usize, &Message)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::Assistant)
            .collect();

        if assistant_msgs.len() < 2 {
            return;
        }

        let mut j = 0;
        while j < assistant_msgs.len() - 1 {
            let (idx_a, msg_a) = assistant_msgs[j];
            let (idx_b, msg_b) = assistant_msgs[j + 1];

            if idx_b - idx_a > MAX_ADJACENT_DISTANCE {
                j += 1;
                continue;
            }

            if Self::is_protected(idx_a, last_boundary) {
                j += 1;
                continue;
            }

            // Never prune an assistant that actually invoked tools. The
            // tool_calls + tool_result pair is load-bearing context; removing
            // it makes the LLM re-request the same call on the next turn.
            if !msg_a.tool_calls.is_empty() {
                j += 1;
                continue;
            }

            // Skip the similarity check when both messages have empty text
            // content. Two empty strings have `content_similarity` of 1.0,
            // which would incorrectly classify legitimate progressive calls
            // as duplicates.
            if msg_a.content.trim().is_empty() && msg_b.content.trim().is_empty() {
                j += 1;
                continue;
            }

            let similarity = content_similarity(&msg_a.content, &msg_b.content);
            if similarity > SIMILARITY_THRESHOLD {
                // Mark the earlier one for removal.
                let tokens = estimate_tokens(&msg_a.content);
                remove_ids.insert(msg_a.message_id.clone());
                *candidate_tokens += tokens;

                // Include the tool result following msg_a if present.
                if idx_a + 1 < messages.len()
                    && messages[idx_a + 1].role == Role::Tool
                    && !Self::is_protected(idx_a + 1, last_boundary)
                {
                    remove_ids.insert(messages[idx_a + 1].message_id.clone());
                    *candidate_tokens += estimate_tokens(&messages[idx_a + 1].content);
                }
            }

            j += 1;
        }
    }

    /// Signal 3: Find tool messages with empty result patterns.
    ///
    /// Same safety rule as Signal 1: never prune a tool result whose parent
    /// assistant carries `tool_calls`. An empty result (e.g. `Glob` with no
    /// matches) is meaningful context; dropping it causes the LLM to re-issue
    /// the same search.
    fn detect_empty_tool_results(
        messages: &[Message],
        last_boundary: Option<usize>,
        remove_ids: &mut HashSet<String>,
        candidate_tokens: &mut u32,
    ) {
        for (i, msg) in messages.iter().enumerate() {
            if msg.role != Role::Tool {
                continue;
            }
            if Self::is_protected(i, last_boundary) {
                continue;
            }
            if i > 0
                && messages[i - 1].role == Role::Assistant
                && !messages[i - 1].tool_calls.is_empty()
            {
                continue;
            }
            if matches_empty_patterns(&msg.content) {
                let tokens = estimate_tokens(&msg.content);
                remove_ids.insert(msg.message_id.clone());
                *candidate_tokens += tokens;

                if i > 0
                    && messages[i - 1].role == Role::Assistant
                    && !Self::is_protected(i - 1, last_boundary)
                {
                    remove_ids.insert(messages[i - 1].message_id.clone());
                    *candidate_tokens += estimate_tokens(&messages[i - 1].content);
                }
            }
        }
    }

    /// Check if a message index is at or after the protected boundary.
    fn is_protected(idx: usize, last_boundary: Option<usize>) -> bool {
        match last_boundary {
            Some(boundary) => idx >= boundary,
            None => false,
        }
    }

    /// Ensure structural integrity of native tool calls.
    ///
    /// When an assistant message with `tool_calls` is marked for removal,
    /// all corresponding tool results (matched by `tool_call_id`) must also
    /// be removed. Conversely, when a tool result is removed, the parent
    /// assistant (if it has no remaining tool results) should be removed too.
    fn ensure_structural_integrity(messages: &[Message], remove_ids: &mut HashSet<String>) {
        // Forward pass: assistant with tool_calls being removed -> remove tool results.
        let assistant_removals: Vec<(usize, Vec<String>)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.role == Role::Assistant
                    && !m.tool_calls.is_empty()
                    && remove_ids.contains(&m.message_id)
            })
            .map(|(i, m)| {
                let tc_ids: Vec<String> = m.tool_calls.iter().map(|tc| tc.id.clone()).collect();
                (i, tc_ids)
            })
            .collect();

        for (_idx, tc_ids) in &assistant_removals {
            for msg in messages {
                if msg.role == Role::Tool {
                    if let Some(ref tcid) = msg.tool_call_id {
                        if tc_ids.contains(tcid) {
                            remove_ids.insert(msg.message_id.clone());
                        }
                    }
                }
            }
        }

        // Reverse pass: tool result being removed -> check if parent assistant
        // has all its tool results removed, and if so, remove the parent too.
        for msg in messages {
            if msg.role == Role::Assistant && !msg.tool_calls.is_empty() {
                let tc_ids: Vec<&str> = msg.tool_calls.iter().map(|tc| tc.id.as_str()).collect();
                let all_results_removed = messages
                    .iter()
                    .filter(|m| {
                        m.role == Role::Tool
                            && m.tool_call_id
                                .as_deref()
                                .is_some_and(|id| tc_ids.contains(&id))
                    })
                    .all(|m| remove_ids.contains(&m.message_id));

                if all_results_removed
                    && messages.iter().any(|m| {
                        m.role == Role::Tool
                            && m.tool_call_id
                                .as_deref()
                                .is_some_and(|id| tc_ids.contains(&id))
                            && remove_ids.contains(&m.message_id)
                    })
                {
                    remove_ids.insert(msg.message_id.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::ToolCallRequest;

    fn default_config() -> IntraTurnPruningConfig {
        IntraTurnPruningConfig {
            enabled: true,
            min_iteration: 3,
            token_threshold: 0, // Disable threshold for most tests.
        }
    }

    fn make_msg(id: &str, role: Role, content: &str) -> Message {
        Message {
            message_id: id.to_string(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_tool_msg(id: &str, content: &str, tool_call_id: &str) -> Message {
        Message {
            message_id: id.to_string(),
            role: Role::Tool,
            content: content.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_assistant_with_tool_calls(id: &str, content: &str, tc_ids: &[&str]) -> Message {
        Message {
            message_id: id.to_string(),
            role: Role::Assistant,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: tc_ids
                .iter()
                .map(|tc_id| ToolCallRequest {
                    id: tc_id.to_string(),
                    name: "test_tool".to_string(),
                    arguments: serde_json::Value::Null,
                })
                .collect(),
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_no_pruning_below_min_iteration() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("s1", Role::System, "system prompt"),
            make_msg("u1", Role::User, "search for X"),
            make_msg("a1", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"error\": \"bad\"}"),
        ];
        let report = pruner.prune_working_history(&mut history, 2);
        assert!(report.skipped);
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn test_no_pruning_when_disabled() {
        let mut config = default_config();
        config.enabled = false;
        let pruner = IntraTurnPruner::from_config(&config);
        let mut history = vec![
            make_msg("u1", Role::User, "search"),
            make_msg("a1", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"error\": \"bad\"}"),
            make_msg("a2", Role::Assistant, "trying again"),
            make_msg("t2", Role::Tool, "{\"results\": [\"ok\"]}"),
        ];
        let report = pruner.prune_working_history(&mut history, 5);
        assert!(report.skipped);
        assert_eq!(history.len(), 5);
    }

    #[test]
    fn test_prunes_error_tool_results() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("s1", Role::System, "system prompt"),
            make_msg("u1", Role::User, "search for X"),
            make_msg("a1", Role::Assistant, "calling ToolSearch"),
            make_msg(
                "t1",
                Role::Tool,
                "{\"error\": \"parameter validation failed\"}",
            ),
            make_msg("a2", Role::Assistant, "trying different approach"),
            make_msg("t2", Role::Tool, "{\"results\": [\"found\"]}"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(!report.skipped);
        assert_eq!(report.messages_removed, 2); // a1 + t1
        assert_eq!(history.len(), 4); // s1, u1, a2, t2 remain
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"t2"));
    }

    #[test]
    fn test_preserves_most_recent_pair() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        // Even if the last tool result is an error, it should be preserved.
        let mut history = vec![
            make_msg("u1", Role::User, "search"),
            make_msg("a1", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"error\": \"first failure\"}"),
            make_msg("a2", Role::Assistant, "calling tool again"),
            make_msg("t2", Role::Tool, "{\"error\": \"second failure\"}"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        // a1+t1 should be pruned, but a2+t2 (last pair) must be preserved.
        assert!(!report.skipped);
        assert_eq!(report.messages_removed, 2); // a1 + t1
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"t2"));
    }

    #[test]
    fn test_prunes_repeated_similar_calls() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("u1", Role::User, "search for X"),
            make_msg("a1", Role::Assistant, "calling ToolSearch(query='X')"),
            make_msg("t1", Role::Tool, "some result"),
            make_msg("a2", Role::Assistant, "calling ToolSearch(query='X')"),
            make_msg("t2", Role::Tool, "better result"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(!report.skipped);
        assert_eq!(report.messages_removed, 2); // a1 + t1 (earlier repeated pair)
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"t2"));
    }

    #[test]
    fn test_prunes_empty_results() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("u1", Role::User, "find files"),
            make_msg("a1", Role::Assistant, "calling FileSearch"),
            make_msg("t1", Role::Tool, "{\"results\": [], \"count\": 0}"),
            make_msg("a2", Role::Assistant, "trying broader search"),
            make_msg("t2", Role::Tool, "{\"results\": [\"file.rs\"]}"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(!report.skipped);
        assert_eq!(report.messages_removed, 2); // a1 + t1
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn test_threshold_gate() {
        let mut config = default_config();
        config.token_threshold = 999_999; // Very high threshold.
        let pruner = IntraTurnPruner::from_config(&config);
        let mut history = vec![
            make_msg("u1", Role::User, "search"),
            make_msg("a1", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"error\": \"bad\"}"),
            make_msg("a2", Role::Assistant, "retry"),
            make_msg("t2", Role::Tool, "ok"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(report.skipped);
        assert_eq!(history.len(), 5); // Nothing removed.
    }

    #[test]
    fn test_preserves_system_and_user_messages() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("s1", Role::System, "system prompt with error: something"),
            make_msg("u1", Role::User, "FAILED to understand, no results found"),
            make_msg("a1", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"error\": \"bad\"}"),
            make_msg("a2", Role::Assistant, "retry"),
            make_msg("t2", Role::Tool, "ok"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        // System and User messages must always be preserved.
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"u1"));
        assert!(!report.skipped);
    }

    #[test]
    fn test_structural_integrity_native_tool_calls() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("u1", Role::User, "do something"),
            make_assistant_with_tool_calls("a1", "calling two tools", &["tc1", "tc2"]),
            make_tool_msg("t1", "{\"error\": \"bad\"}", "tc1"),
            make_tool_msg("t2", "good result", "tc2"),
            make_msg("a2", Role::Assistant, "final answer"),
            make_msg("t3", Role::Tool, "ok"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        // a1 carries tool_calls -> t1 (error) must NOT be pruned even though
        // its content matches error patterns. Removing the call+result pair
        // would strip causal history and cause the LLM to re-issue the call.
        assert!(report.skipped || report.messages_removed == 0);
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a1"));
        assert!(ids.contains(&"t1"));
        assert!(ids.contains(&"t2"));
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"t3"));
    }

    #[test]
    fn test_no_pruning_on_clean_history() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("s1", Role::System, "system"),
            make_msg("u1", Role::User, "hello"),
            make_msg("a1", Role::Assistant, "Hi! How can I help?"),
            make_msg("u2", Role::User, "search for files"),
            make_msg("a2", Role::Assistant, "calling tool"),
            make_msg("t1", Role::Tool, "{\"results\": [\"file.rs\", \"lib.rs\"]}"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(report.skipped);
        assert_eq!(history.len(), 6);
    }

    #[test]
    fn test_multiple_failures_pruned() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("u1", Role::User, "search"),
            make_msg("a1", Role::Assistant, "try 1"),
            make_msg("t1", Role::Tool, "{\"error\": \"fail 1\"}"),
            make_msg("a2", Role::Assistant, "try 2"),
            make_msg("t2", Role::Tool, "{\"error\": \"fail 2\"}"),
            make_msg("a3", Role::Assistant, "try 3"),
            make_msg("t3", Role::Tool, "{\"error\": \"fail 3\"}"),
            make_msg("a4", Role::Assistant, "try 4"),
            make_msg("t4", Role::Tool, "{\"results\": [\"success\"]}"),
        ];

        let report = pruner.prune_working_history(&mut history, 5);
        assert!(!report.skipped);
        // a1+t1, a2+t2, a3+t3 should be pruned. a4+t4 preserved (last pair).
        assert_eq!(report.messages_removed, 6);
        assert_eq!(history.len(), 3); // u1, a4, t4
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"a4"));
        assert!(ids.contains(&"t4"));
    }

    /// Regression: native tool-calling produces empty assistant message content
    /// (all info is in `tool_calls`). Two consecutive empty-content assistant
    /// messages must NOT be classified as "repeated calls" regardless of
    /// iteration count, because `content_similarity("", "") == 1.0` would
    /// otherwise delete the earlier `FileRead` result and break the loop.
    #[test]
    fn test_no_pruning_for_empty_content_native_tool_calls() {
        let pruner = IntraTurnPruner::from_config(&default_config());

        // Simulate knowledge-summarizer: two consecutive FileRead tool calls
        // where the assistant messages have empty text content (native mode).
        let mut history = vec![
            make_msg("s1", Role::System, "system prompt"),
            make_msg("u1", Role::User, "summarize /path/to/file.md (1000 lines)"),
            // Iteration 1: empty assistant + FileRead result (chunk 1)
            make_assistant_with_tool_calls("a1", "", &["tc1"]),
            make_tool_msg(
                "t1",
                "{\"content\": \"chunk 1 content\", \"lines_read\": 500}",
                "tc1",
            ),
            // Iteration 2: empty assistant + FileRead result (chunk 2)
            make_assistant_with_tool_calls("a2", "", &["tc2"]),
            make_tool_msg(
                "t2",
                "{\"content\": \"chunk 2 content\", \"lines_read\": 500}",
                "tc2",
            ),
        ];

        // Iteration 3 fires pruning (min_iteration = 3). The two empty assistant
        // messages must NOT be identified as repeats -- all history must survive.
        let report = pruner.prune_working_history(&mut history, 3);
        assert!(
            report.skipped || report.messages_removed == 0,
            "progressive FileRead chain was incorrectly pruned: {} messages removed",
            report.messages_removed
        );
        assert_eq!(history.len(), 6, "all messages must be preserved");

        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a1"), "first FileRead call must be preserved");
        assert!(
            ids.contains(&"t1"),
            "first FileRead result must be preserved"
        );
        assert!(
            ids.contains(&"a2"),
            "second FileRead call must be preserved"
        );
        assert!(
            ids.contains(&"t2"),
            "second FileRead result must be preserved"
        );
    }

    /// Regression: when the model emits short explanatory text alongside each
    /// tool call (e.g. "Let me read the next chunk"), similarity between
    /// consecutive assistant messages can exceed `SIMILARITY_THRESHOLD`. The
    /// previous implementation deleted the earlier assistant + its tool result,
    /// stripping a real tool invocation from history and causing the LLM to
    /// re-issue the same call on the next iteration -- an infinite loop.
    /// An assistant carrying `tool_calls` must never be pruned by the
    /// repeated-calls signal.
    #[test]
    fn test_no_pruning_for_similar_text_when_assistant_has_tool_calls() {
        let pruner = IntraTurnPruner::from_config(&default_config());

        let mut history = vec![
            make_msg("s1", Role::System, "system"),
            make_msg("u1", Role::User, "list files in src/ then in tests/"),
            make_assistant_with_tool_calls("a1", "Let me list the directory.", &["tc1"]),
            make_tool_msg("t1", "{\"files\": [\"main.rs\"]}", "tc1"),
            make_assistant_with_tool_calls("a2", "Let me list the directory.", &["tc2"]),
            make_tool_msg("t2", "{\"files\": [\"test.rs\"]}", "tc2"),
        ];

        let report = pruner.prune_working_history(&mut history, 3);
        assert!(
            report.skipped || report.messages_removed == 0,
            "tool-calling assistant pair was incorrectly pruned: {} messages removed",
            report.messages_removed
        );
        assert_eq!(history.len(), 6, "all tool_call messages must be preserved");
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a1"));
        assert!(ids.contains(&"t1"));
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"t2"));
    }

    /// Regression: `FileRead` on a missing path returns `"error": "not found"`,
    /// which matches `ERROR_PATTERNS`. The previous implementation pruned the
    /// tool_result AND the parent assistant (the `tool_call`), so the LLM
    /// reissued the same `FileRead` on the next iteration and entered an
    /// infinite loop. A tool_result whose parent assistant has `tool_calls`
    /// must be preserved regardless of content heuristics.
    #[test]
    fn test_no_pruning_for_error_tool_results_with_tool_calls() {
        let pruner = IntraTurnPruner::from_config(&default_config());
        let mut history = vec![
            make_msg("s1", Role::System, "system"),
            make_msg("u1", Role::User, "read README.md and access.go"),
            make_assistant_with_tool_calls("a1", "", &["tc_readme"]),
            make_tool_msg(
                "t1",
                "{\"error\": \"file not found: README.md\"}",
                "tc_readme",
            ),
            make_assistant_with_tool_calls("a2", "", &["tc_go"]),
            make_tool_msg("t2", "{\"content\": \"package biz\"}", "tc_go"),
        ];

        let report = pruner.prune_working_history(&mut history, 3);
        assert!(
            report.skipped || report.messages_removed == 0,
            "tool_call pair with error result was wrongly pruned: {} removed",
            report.messages_removed
        );
        assert_eq!(history.len(), 6, "all tool_call pairs must survive");
        let ids: Vec<&str> = history.iter().map(|m| m.message_id.as_str()).collect();
        assert!(ids.contains(&"a1"), "earlier FileRead call must survive");
        assert!(ids.contains(&"t1"), "earlier error result must survive");
    }
}
