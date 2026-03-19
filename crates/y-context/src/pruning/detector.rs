//! Hybrid failure detection for context pruning.
//!
//! Design reference: context-pruning-design.md, Hybrid Failure Detection
//!
//! Three detection signals, applied conservatively:
//! 1. Error status -- tool result contains error field or error-indicating patterns
//! 2. Repeated calls -- same tool with similar arguments within N turns
//! 3. Empty results -- tool result contains patterns indicating no useful output

use y_core::session::ChatMessageRecord;

use super::strategy::{PruningCandidate, PruningReason};

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

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
    fn detect_error_status(
        &self,
        messages: &[ChatMessageRecord],
        candidates: &mut Vec<PruningCandidate>,
    ) {
        let error_patterns = [
            "\"error\":",
            "\"error_type\":",
            "error:",
            "Error:",
            "FAILED",
            "failed to",
            "parameter validation failed",
            "not found",
            "permission denied",
        ];

        for (i, msg) in messages.iter().enumerate() {
            if msg.role != "tool" {
                continue;
            }

            let is_error = error_patterns.iter().any(|p| msg.content.contains(p));

            // Check extra user-defined patterns.
            let matches_extra = self
                .extra_patterns
                .iter()
                .any(|p| msg.content.contains(p.as_str()));

            if is_error || matches_extra {
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
            if idx_b - idx_a > 3 {
                i += 1;
                continue;
            }

            // Simple similarity: check if content starts with the same prefix.
            let similarity = content_similarity(&msg_a.content, &msg_b.content);
            if similarity > 0.8 {
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
    fn detect_empty_results(
        messages: &[ChatMessageRecord],
        candidates: &mut Vec<PruningCandidate>,
    ) {
        let empty_patterns = [
            "\"results\": []",
            "\"results\":[]",
            "\"count\": 0",
            "\"count\":0",
            "no results found",
            "No results found",
            "no matches",
            "No matches",
            "[]",
        ];

        for (i, msg) in messages.iter().enumerate() {
            if msg.role != "tool" {
                continue;
            }

            // Only match if the content is short AND matches an empty pattern.
            // Long tool results with "no results" embedded are likely informative.
            let is_empty =
                msg.content.len() < 200 && empty_patterns.iter().any(|p| msg.content.contains(p));

            if is_empty {
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

/// Simple content similarity ratio (0.0 to 1.0).
///
/// Uses the shorter content as the reference length. Compares
/// character-by-character up to the shorter length.
fn content_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let min_len = a.len().min(b.len());
    let max_len = a.len().max(b.len());

    let matching: usize = a
        .chars()
        .zip(b.chars())
        .take(min_len)
        .filter(|(ca, cb)| ca == cb)
        .count();

    matching as f64 / max_len as f64
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
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_detect_error_status() {
        let messages = vec![
            make_msg("m1", "user", "search for X"),
            make_msg("m2", "assistant", "calling tool_search"),
            make_msg("m3", "tool", "{\"error\": \"parameter validation failed\"}"),
            make_msg("m4", "assistant", "calling tool_search again"),
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
            make_msg("m2", "assistant", "calling file_search"),
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
            make_msg("m2", "assistant", "calling tool_search(query='X')"),
            make_msg("m3", "tool", "no results found"),
            make_msg("m4", "assistant", "calling tool_search(query='X')"),
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
    fn test_content_similarity() {
        assert!((content_similarity("abc", "abc") - 1.0).abs() < f64::EPSILON);
        assert!(content_similarity("abc", "xyz") < 0.5);
        assert!(content_similarity("", "") > 0.99);
        assert!(content_similarity("abc", "") < 0.01);
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
}
