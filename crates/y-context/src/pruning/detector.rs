//! Hybrid failure detection for context pruning.
//!
//! Design reference: context-pruning-design.md, Hybrid Failure Detection
//!
//! Three detection signals, applied conservatively:
//! 1. Error status -- tool result contains error field or error-indicating patterns
//! 2. Repeated calls -- same tool with similar arguments within N turns
//! 3. Empty results -- tool result contains patterns indicating no useful output

use y_core::session::ChatMessageRecord;

use super::patterns::{
    content_similarity, estimate_tokens, matches_empty_patterns, matches_error_patterns,
    MAX_ADJACENT_DISTANCE, SIMILARITY_THRESHOLD,
};
use super::strategy::{PruningCandidate, PruningReason};

/// Detects failed message branches for pruning.
pub struct PruningDetector {
    /// Additional heuristic patterns for failure detection.
    pub extra_patterns: Vec<String>,
}

impl PruningDetector {
    /// Create a detector with default settings.
    pub fn new() -> Self {
        Self {
            extra_patterns: Vec::new(),
        }
    }

    /// Create a detector with additional failure patterns.
    pub fn with_patterns(patterns: Vec<String>) -> Self {
        Self {
            extra_patterns: patterns,
        }
    }

    /// Detect failed branches in a sequence of messages.
    ///
    /// Returns pruning candidates grouped by failure reason.
    /// Detection is conservative: only marks as failed if at least one signal fires.
    pub fn detect_failures(&self, messages: &[ChatMessageRecord]) -> Vec<PruningCandidate> {
        let mut candidates = Vec::new();

        // Signal 1: Error status in tool results.
        self.detect_error_status(messages, &mut candidates);

        // Signal 2: Repeated identical tool calls.
        Self::detect_repeated_calls(messages, &mut candidates);

        // Signal 3: Empty/unhelpful results.
        Self::detect_empty_results(messages, &mut candidates);

        candidates
    }

    /// Signal 1: Detect tool results containing error indicators.
    ///
    /// Skips tool results whose parent assistant has `has_tool_calls = true`.
    /// An error result for a real tool call (e.g. `read` on a missing path)
    /// is load-bearing context — removing it makes the LLM re-issue the same
    /// call. This mirrors the protection the intra-turn pruner already
    /// enforces (`intra_turn.rs:196-203`) but the post-turn detector
    /// previously lacked.
    fn detect_error_status(
        &self,
        messages: &[ChatMessageRecord],
        candidates: &mut Vec<PruningCandidate>,
    ) {
        for (i, msg) in messages.iter().enumerate() {
            if msg.role != "tool" {
                continue;
            }

            // Never prune a tool result whose parent assistant carries
            // tool_calls: the call + result pair is load-bearing context.
            if i > 0 && messages[i - 1].role == "assistant" && messages[i - 1].has_tool_calls {
                continue;
            }

            if matches_error_patterns(&msg.content, &self.extra_patterns) {
                // Also include the preceding assistant message (the tool call request)
                // if it immediately precedes this tool result.
                let mut ids = vec![msg.id.clone()];
                let mut tokens = estimate_tokens(&msg.content);

                if i > 0 && messages[i - 1].role == "assistant" {
                    ids.insert(0, messages[i - 1].id.clone());
                    tokens += estimate_tokens(&messages[i - 1].content);
                }

                candidates.push(PruningCandidate {
                    message_ids: ids,
                    estimated_tokens: tokens,
                    reason: PruningReason::ErrorStatus,
                });
            }
        }
    }

    /// Signal 2: Detect repeated identical tool calls (same tool name, similar args).
    ///
    /// Skips assistant messages with `has_tool_calls = true`: a real tool
    /// invocation whose result is referenced by later messages is
    /// load-bearing. Dropping it (and its paired tool result) would leave
    /// the LLM with no record of the call and trigger re-invocation.
    /// Also skips when both messages have empty content — two empty
    /// strings have `content_similarity` of 1.0, which would incorrectly
    /// classify legitimate progressive calls as duplicates.
    fn detect_repeated_calls(
        messages: &[ChatMessageRecord],
        candidates: &mut Vec<PruningCandidate>,
    ) {
        // Track assistant messages that look like tool calls.
        // Simple heuristic: look for consecutive assistant messages with similar content.
        let assistant_msgs: Vec<(usize, &ChatMessageRecord)> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "assistant")
            .collect();

        if assistant_msgs.len() < 2 {
            return;
        }

        // Find consecutive pairs with very similar content (likely retries).
        let mut i = 0;
        while i < assistant_msgs.len() - 1 {
            let (idx_a, msg_a) = assistant_msgs[i];
            let (idx_b, msg_b) = assistant_msgs[i + 1];

            // Skip if they are not close together in the original sequence.
            if idx_b - idx_a > MAX_ADJACENT_DISTANCE {
                i += 1;
                continue;
            }

            // Never prune an assistant that actually invoked tools.
            if msg_a.has_tool_calls {
                i += 1;
                continue;
            }

            // Skip the similarity check when both messages have empty text
            // content. Two empty strings have `content_similarity` of 1.0,
            // which would incorrectly classify legitimate progressive calls
            // as duplicates.
            if msg_a.content.trim().is_empty() && msg_b.content.trim().is_empty() {
                i += 1;
                continue;
            }

            let similarity = content_similarity(&msg_a.content, &msg_b.content);
            if similarity > SIMILARITY_THRESHOLD {
                // Mark the earlier one (and its tool result) as a candidate.
                let mut ids = vec![msg_a.id.clone()];
                let mut tokens = estimate_tokens(&msg_a.content);

                // Include the tool result following msg_a if present.
                if idx_a + 1 < messages.len() && messages[idx_a + 1].role == "tool" {
                    ids.push(messages[idx_a + 1].id.clone());
                    tokens += estimate_tokens(&messages[idx_a + 1].content);
                }

                candidates.push(PruningCandidate {
                    message_ids: ids,
                    estimated_tokens: tokens,
                    reason: PruningReason::RepeatedCalls,
                });
            }

            i += 1;
        }
    }

    /// Signal 3: Detect tool results with empty or unhelpful content.
    ///
    /// Same safety rule as Signal 1: never prune a tool result whose parent
    /// assistant carries `tool_calls`. An empty result (e.g. `Glob` with no
    /// matches) is meaningful context; dropping it causes the LLM to re-issue
    /// the same search.
    fn detect_empty_results(
        messages: &[ChatMessageRecord],
        candidates: &mut Vec<PruningCandidate>,
    ) {
        for (i, msg) in messages.iter().enumerate() {
            if msg.role != "tool" {
                continue;
            }

            // Never prune a tool result whose parent assistant carries
            // tool_calls: the call + result pair is load-bearing context.
            if i > 0 && messages[i - 1].role == "assistant" && messages[i - 1].has_tool_calls {
                continue;
            }

            if matches_empty_patterns(&msg.content) {
                let mut ids = vec![msg.id.clone()];
                let mut tokens = estimate_tokens(&msg.content);

                if i > 0 && messages[i - 1].role == "assistant" {
                    ids.insert(0, messages[i - 1].id.clone());
                    tokens += estimate_tokens(&messages[i - 1].content);
                }

                candidates.push(PruningCandidate {
                    message_ids: ids,
                    estimated_tokens: tokens,
                    reason: PruningReason::EmptyResult,
                });
            }
        }
    }
}

impl Default for PruningDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::ChatMessageStatus;
    use y_core::types::SessionId;

    fn make_msg(id: &str, role: &str, content: &str) -> ChatMessageRecord {
        ChatMessageRecord {
            id: id.to_string(),
            session_id: SessionId("test".to_string()),
            role: role.to_string(),
            content: content.to_string(),
            status: ChatMessageStatus::Active,
            checkpoint_id: None,
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            context_window: None,
            parent_message_id: None,
            pruning_group_id: None,
            has_tool_calls: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_detect_error_status() {
        let messages = vec![
            make_msg("m1", "user", "search for X"),
            make_msg("m2", "assistant", "calling ToolSearch"),
            make_msg("m3", "tool", "{\"error\": \"parameter validation failed\"}"),
            make_msg("m4", "assistant", "calling ToolSearch again"),
            make_msg("m5", "tool", "{\"results\": [\"found\"]}"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);

        // Should detect m2+m3 as error (m2 is the preceding assistant message).
        assert!(!candidates.is_empty());
        let error_candidate = candidates
            .iter()
            .find(|c| c.reason == PruningReason::ErrorStatus)
            .unwrap();
        assert!(error_candidate.message_ids.contains(&"m3".to_string()));
        assert!(error_candidate.message_ids.contains(&"m2".to_string()));
    }

    #[test]
    fn test_detect_empty_results() {
        let messages = vec![
            make_msg("m1", "user", "find files"),
            make_msg("m2", "assistant", "calling FileSearch"),
            make_msg("m3", "tool", "{\"results\": [], \"count\": 0}"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);

        let empty_candidate = candidates
            .iter()
            .find(|c| c.reason == PruningReason::EmptyResult);
        assert!(empty_candidate.is_some());
    }

    #[test]
    fn test_detect_repeated_calls() {
        let messages = vec![
            make_msg("m1", "user", "search for X"),
            make_msg("m2", "assistant", "calling ToolSearch(query='X')"),
            make_msg("m3", "tool", "no results found"),
            make_msg("m4", "assistant", "calling ToolSearch(query='X')"),
            make_msg("m5", "tool", "{\"results\": [\"found\"]}"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);

        let repeated = candidates
            .iter()
            .find(|c| c.reason == PruningReason::RepeatedCalls);
        assert!(repeated.is_some());
    }

    #[test]
    fn test_no_false_positives_on_clean_messages() {
        let messages = vec![
            make_msg("m1", "user", "hello"),
            make_msg("m2", "assistant", "Hi there! How can I help?"),
            make_msg("m3", "user", "what is 2+2?"),
            make_msg("m4", "assistant", "2+2 = 4"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_extra_patterns() {
        let messages = vec![
            make_msg("m1", "assistant", "call api"),
            make_msg("m2", "tool", "CUSTOM_FAILURE_CODE: 42"),
        ];

        let detector = PruningDetector::with_patterns(vec!["CUSTOM_FAILURE_CODE".to_string()]);
        let candidates = detector.detect_failures(&messages);
        assert!(!candidates.is_empty());
    }

    #[test]
    fn test_protects_tool_calls_bearing_assistant() {
        // Assistant with has_tool_calls=true must NOT be pruned even if the
        // tool result matches error patterns. This is the core fix for the
        // post-turn detector's amnesia bug.
        let mut m2 = make_msg("m2", "assistant", "calling FileRead");
        m2.has_tool_calls = true;
        let messages = vec![
            make_msg("m1", "user", "read config"),
            m2,
            make_msg("m3", "tool", "error: file not found"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);

        // No candidates should be generated — the tool result is "error:"
        // but its parent assistant has tool_calls, so the pair is protected.
        let error_candidate = candidates
            .iter()
            .find(|c| c.reason == PruningReason::ErrorStatus);
        assert!(
            error_candidate.is_none(),
            "tool result with tool_calls-bearing parent must be protected from pruning"
        );
    }

    #[test]
    fn test_protects_tool_calls_from_repeated_call_detection() {
        // Two similar assistant messages, but the first has tool_calls.
        // The similarity detector must skip it.
        let mut m2 = make_msg("m2", "assistant", "calling ToolSearch(query='X')");
        m2.has_tool_calls = true;
        let messages = vec![
            make_msg("m1", "user", "search for X"),
            m2,
            make_msg("m3", "tool", "no results found"),
            make_msg("m4", "assistant", "calling ToolSearch(query='X')"),
            make_msg("m5", "tool", "{\"results\": [\"found\"]}"),
        ];

        let detector = PruningDetector::new();
        let candidates = detector.detect_failures(&messages);

        let repeated = candidates
            .iter()
            .find(|c| c.reason == PruningReason::RepeatedCalls);
        assert!(
            repeated.is_none(),
            "assistant with tool_calls must be protected from repeated-call pruning"
        );
    }
}
