//! Tool output pruning: blank superseded and useless tool results.
//!
//! Unlike `RetryPruning` (which tombstones failed branches) and
//! `ProgressivePruning` (which LLM-summarizes completed sequences), this
//! module does **in-place content replacement**: the tool result message
//! stays in the transcript but its content is replaced with a short
//! placeholder. This preserves `tool_call`↔`tool_result` pairing for provider
//! history replay while reclaiming context tokens.
//!
//! Two signals:
//!
//! - **Superseded**: a tool result that has been rendered obsolete by a
//!   newer result for the same key (e.g. `read("src/main.rs")` called
//!   twice — the older read is superseded).
//! - **Useless**: a tool result the tool itself flagged as uninformative
//!   (zero-match search, empty glob, timed-out poll).
//!
//! Design reference: omp `packages/agent/src/compaction/pruning.ts:175-301`

use y_core::types::{Message, Role};

/// Placeholder written over a superseded tool result.
pub const SUPERSEDED_NOTICE: &str = "[Superseded by a newer read of this file]";

/// Placeholder written over a useless tool result.
pub const USELESS_NOTICE: &str = "[Uneventful result elided]";

/// Minimum original token count before a result is worth blanking.
/// The placeholder itself costs ~8 tokens, so blanking a sub-floor result
/// would grow the context and churn the prompt cache for nothing.
const MIN_PRUNE_TOKENS: u32 = 50;

/// Maximum suffix tokens (messages after the candidate) for a non-idle
/// cache-aware prune. When the suffix is small, re-caching is cheap.
const DEFAULT_SUFFIX_TOKEN_LIMIT: u32 = 8_000;

/// Result of a tool output pruning pass.
#[derive(Debug, Clone, Default)]
pub struct ToolOutputPruneResult {
    /// Number of tool results blanked.
    pub pruned_count: usize,
    /// Estimated tokens saved.
    pub tokens_saved: u32,
    /// Message IDs of tool results that were blanked, for in-place
    /// transcript updates via `TranscriptStore::update_message`.
    pub modified_message_ids: Vec<String>,
}

/// Supersede key: maps a tool call to a key that identifies "same resource".
///
/// Two tool results with the same key are candidates for supersession —
/// the older one is blanked.
///
/// Currently supports:
/// - `FileRead`: key = file path (the `path` argument)
/// - Other tools: no supersede key (returns `None`)
pub fn supersede_key(tool_name: &str, tool_args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "FileRead" | "read" => tool_args
            .get("path")
            .and_then(|v| v.as_str())
            .map(normalize_path),
        _ => None,
    }
}

/// Normalize a file path for supersede key comparison.
/// Strips line-range selectors so `file.rs:50-100` and `file.rs:200-300`
/// share the same key (both are reads of `file.rs`).
fn normalize_path(path: &str) -> String {
    // Strip line selectors: `path:offset:limit`, `path:offset-limit`, etc.
    if let Some(colon_pos) = path.rfind(':') {
        let after = &path[colon_pos + 1..];
        if after
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == ',')
        {
            return path[..colon_pos].to_string();
        }
    }
    path.to_string()
}

/// Detect whether a tool result is "useless" — uninformative enough that
/// blanking it saves context without losing actionable information.
///
/// Patterns:
/// - Empty array/object: `[]`, `{}`
/// - Zero-count results: `"count": 0`, `"no results"`, `"no matches"`
/// - Short content (< 200 chars) with empty-result patterns
fn is_useless_result(content: &str) -> bool {
    if content.len() > 200 {
        return false;
    }
    let patterns = [
        "[]",
        "{}",
        "\"results\": []",
        "\"count\": 0",
        "no results",
        "No results",
        "no matches",
        "No matches",
        "nothing found",
    ];
    patterns.iter().any(|p| content.contains(p))
}

/// Estimate tokens from character count (~4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.chars().count().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Compute suffix token counts: for each message index, the total tokens
/// of all messages strictly after it.
fn compute_suffix_tokens(messages: &[Message]) -> Vec<u32> {
    let mut suffix: Vec<u32> = vec![0; messages.len()];
    let mut running: u32 = 0;
    for i in (0..messages.len()).rev() {
        suffix[i] = running;
        running = running.saturating_add(estimate_tokens(&messages[i].content));
    }
    suffix
}

/// Configuration for tool output pruning.
#[derive(Debug, Clone)]
pub struct ToolOutputPruneConfig {
    /// Whether to prune superseded tool results.
    pub prune_superseded: bool,
    /// Whether to prune useless tool results.
    pub prune_useless: bool,
    /// Maximum suffix tokens for a non-idle prune (cache-aware).
    pub suffix_token_limit: u32,
    /// Whether the session is idle (cache is cold, so re-caching is free).
    pub is_idle: bool,
}

impl Default for ToolOutputPruneConfig {
    fn default() -> Self {
        Self {
            prune_superseded: true,
            prune_useless: true,
            suffix_token_limit: DEFAULT_SUFFIX_TOKEN_LIMIT,
            is_idle: false,
        }
    }
}

/// Prune tool outputs in-place: blank superseded and useless tool results.
///
/// Modifies `messages` in-place by replacing tool result content with
/// placeholder notices. Returns a report with counts and tokens saved.
///
/// # Cache-aware timing
///
/// When `is_idle` is false, only candidates whose suffix (messages after
/// them) is <= `suffix_token_limit` are pruned — this limits prompt cache
/// churn to the cheap tail. When `is_idle` is true, all candidates are
/// pruned (cache is already cold).
pub fn prune_tool_outputs(
    messages: &mut [Message],
    config: &ToolOutputPruneConfig,
) -> ToolOutputPruneResult {
    if messages.is_empty() {
        return ToolOutputPruneResult::default();
    }

    let suffix_tokens = compute_suffix_tokens(messages);

    // Collect supersede keys for tool results, keyed by tool_call_id.
    // We need to find which tool calls each tool result corresponds to.
    // In y-agent's Message model, tool results have `tool_call_id` linking
    // back to the assistant's `tool_calls` entry.
    let mut tool_call_args: std::collections::HashMap<&str, (&str, &serde_json::Value)> =
        std::collections::HashMap::new();
    for msg in messages.iter() {
        if msg.role == Role::Assistant {
            for tc in &msg.tool_calls {
                tool_call_args.insert(tc.id.as_str(), (tc.name.as_str(), &tc.arguments));
            }
        }
    }

    // First pass: collect supersede keys seen, from newest to oldest.
    // A tool result is superseded if a LATER result has the same key.
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut superseded_indices: Vec<usize> = Vec::new();
    let mut useless_indices: Vec<usize> = Vec::new();

    for i in (0..messages.len()).rev() {
        if messages[i].role != Role::Tool {
            continue;
        }

        let Some(tool_call_id) = &messages[i].tool_call_id else {
            continue;
        };

        let (tool_name, tool_args) = match tool_call_args.get(tool_call_id.as_str()) {
            Some(entry) => *entry,
            None => continue,
        };

        // Check superseded.
        if config.prune_superseded {
            if let Some(key) = supersede_key(tool_name, tool_args) {
                if seen_keys.contains(&key) {
                    // This result is superseded by a newer one.
                    let tokens = estimate_tokens(&messages[i].content);
                    if tokens >= MIN_PRUNE_TOKENS {
                        superseded_indices.push(i);
                    }
                } else {
                    seen_keys.insert(key);
                }
            }
        }

        // Check useless.
        if config.prune_useless && is_useless_result(&messages[i].content) {
            let tokens = estimate_tokens(&messages[i].content);
            if tokens >= MIN_PRUNE_TOKENS {
                useless_indices.push(i);
            }
        }
    }

    // Apply cache-aware filtering: only prune candidates whose suffix is small
    // (or when idle).
    let mut pruned_count = 0;
    let mut tokens_saved = 0;
    let mut modified_message_ids = Vec::new();

    for &i in superseded_indices.iter().chain(useless_indices.iter()) {
        if !config.is_idle && suffix_tokens[i] > config.suffix_token_limit {
            continue;
        }

        let original_tokens = estimate_tokens(&messages[i].content);
        let notice = if superseded_indices.contains(&i) {
            SUPERSEDED_NOTICE
        } else {
            USELESS_NOTICE
        };
        let notice_tokens = estimate_tokens(notice);
        let saved = original_tokens.saturating_sub(notice_tokens);
        if saved == 0 {
            continue;
        }

        messages[i].content = notice.to_string();
        modified_message_ids.push(messages[i].message_id.clone());
        pruned_count += 1;
        tokens_saved += saved;
    }

    ToolOutputPruneResult {
        pruned_count,
        tokens_saved,
        modified_message_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::{Message, Role, ToolCallRequest};

    fn make_msg(id: &str, role: Role, content: &str) -> Message {
        Message {
            message_id: id.to_string(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    fn make_tool_msg(id: &str, content: &str, tool_call_id: &str) -> Message {
        Message {
            message_id: id.to_string(),
            role: Role::Tool,
            content: content.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: vec![],
            timestamp: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    fn make_assistant_with_tool_call(
        id: &str,
        content: &str,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Message {
        Message {
            message_id: id.to_string(),
            role: Role::Assistant,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![ToolCallRequest {
                id: tool_call_id.to_string(),
                name: tool_name.to_string(),
                arguments: args,
            }],
            timestamp: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn test_supersede_key_fileread() {
        let key = supersede_key("FileRead", &serde_json::json!({"path": "src/main.rs"}));
        assert_eq!(key.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn test_supersede_key_strips_line_selector() {
        let key = supersede_key(
            "FileRead",
            &serde_json::json!({"path": "src/main.rs:50-100"}),
        );
        assert_eq!(key.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn test_supersede_key_unknown_tool() {
        let key = supersede_key("Grep", &serde_json::json!({"pattern": "foo"}));
        assert!(key.is_none());
    }

    #[test]
    fn test_prune_superseded_file_read() {
        let long_content = "x".repeat(300);
        let mut messages = vec![
            make_msg("u1", Role::User, "read file twice"),
            make_assistant_with_tool_call(
                "a1",
                "reading file",
                "tc1",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t1", &long_content, "tc1"),
            make_assistant_with_tool_call(
                "a2",
                "reading file again",
                "tc2",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t2", &long_content, "tc2"),
        ];

        let result = prune_tool_outputs(&mut messages, &ToolOutputPruneConfig::default());
        assert_eq!(result.pruned_count, 1);
        assert!(result.tokens_saved > 0);
        // The first read (t1) should be superseded.
        assert_eq!(messages[2].content, SUPERSEDED_NOTICE);
        // The second read (t2) should be preserved.
        assert_eq!(messages[4].content, long_content);
    }

    #[test]
    fn test_prune_useless_result() {
        let mut messages = vec![
            make_msg("u1", Role::User, "search for X"),
            make_assistant_with_tool_call(
                "a1",
                "searching",
                "tc1",
                "Grep",
                serde_json::json!({"pattern": "nonexistent"}),
            ),
            make_tool_msg("t1", "no matches found", "tc1"),
            make_assistant_with_tool_call(
                "a2",
                "done",
                "tc2",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t2", &"x".repeat(300), "tc2"),
        ];

        let result = prune_tool_outputs(&mut messages, &ToolOutputPruneConfig::default());
        // "no matches found" is 18 chars = ~5 tokens, below MIN_PRUNE_TOKENS (50).
        // So it should NOT be pruned.
        assert_eq!(result.pruned_count, 0);
    }

    #[test]
    fn test_prune_useless_result_above_min_tokens() {
        // Must be <= 200 chars (is_useless_result limit) AND >= ~50 tokens
        // (MIN_PRUNE_TOKENS). 200 chars / 4 = 50 tokens exactly.
        let long_empty = format!("no matches found\n{}", "x".repeat(181));
        let mut messages = vec![
            make_msg("u1", Role::User, "search for X"),
            make_assistant_with_tool_call(
                "a1",
                "searching",
                "tc1",
                "Grep",
                serde_json::json!({"pattern": "nonexistent"}),
            ),
            make_tool_msg("t1", &long_empty, "tc1"),
        ];

        let result = prune_tool_outputs(&mut messages, &ToolOutputPruneConfig::default());
        assert_eq!(result.pruned_count, 1);
        assert_eq!(messages[2].content, USELESS_NOTICE);
    }

    #[test]
    fn test_cache_aware_pruning_skips_large_suffix() {
        let long_content = "x".repeat(300);
        let large_suffix = "y".repeat(10_000);
        let mut messages = vec![
            make_msg("u1", Role::User, "read file twice"),
            make_assistant_with_tool_call(
                "a1",
                "reading",
                "tc1",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t1", &long_content, "tc1"),
            make_assistant_with_tool_call(
                "a2",
                "reading again",
                "tc2",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t2", &long_content, "tc2"),
            make_msg("u2", Role::User, &large_suffix),
        ];

        let config = ToolOutputPruneConfig {
            is_idle: false,
            suffix_token_limit: 1000, // small limit
            ..Default::default()
        };
        let result = prune_tool_outputs(&mut messages, &config);
        // Suffix after t1 is too large (u2 alone is >1000 tokens), so skip.
        assert_eq!(result.pruned_count, 0);
    }

    #[test]
    fn test_idle_pruning_ignores_suffix() {
        let long_content = "x".repeat(300);
        let large_suffix = "y".repeat(10_000);
        let mut messages = vec![
            make_msg("u1", Role::User, "read file twice"),
            make_assistant_with_tool_call(
                "a1",
                "reading",
                "tc1",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t1", &long_content, "tc1"),
            make_assistant_with_tool_call(
                "a2",
                "reading again",
                "tc2",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t2", &long_content, "tc2"),
            make_msg("u2", Role::User, &large_suffix),
        ];

        let config = ToolOutputPruneConfig {
            is_idle: true,
            suffix_token_limit: 1000,
            ..Default::default()
        };
        let result = prune_tool_outputs(&mut messages, &config);
        assert_eq!(result.pruned_count, 1);
    }

    #[test]
    fn test_no_prune_below_min_tokens() {
        let short_content = "short result"; // ~3 tokens, below MIN_PRUNE_TOKENS
        let mut messages = vec![
            make_msg("u1", Role::User, "read file"),
            make_assistant_with_tool_call(
                "a1",
                "reading",
                "tc1",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t1", short_content, "tc1"),
            make_assistant_with_tool_call(
                "a2",
                "reading again",
                "tc2",
                "FileRead",
                serde_json::json!({"path": "src/main.rs"}),
            ),
            make_tool_msg("t2", short_content, "tc2"),
        ];

        let result = prune_tool_outputs(&mut messages, &ToolOutputPruneConfig::default());
        assert_eq!(result.pruned_count, 0);
    }
}
