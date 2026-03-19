//! `RetryPruning` strategy: removes failed tool call branches.
//!
//! Design reference: context-pruning-design.md, Flow 1
//!
//! Zero LLM cost, target < 5ms. Uses `PruningDetector` for hybrid
//! failure detection, then tombstones failed messages via batch status update.

use async_trait::async_trait;

use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionError};
use y_core::types::SessionId;

use super::detector::PruningDetector;
use super::report::{PruningReport, PruningStrategyType};
use super::strategy::{PruningCandidate, PruningStrategy};

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Retry pruning: removes failed tool call branches without LLM cost.
pub struct RetryPruning {
    detector: PruningDetector,
}

impl RetryPruning {
    /// Create with default detector.
    pub fn new() -> Self {
        Self {
            detector: PruningDetector::new(),
        }
    }

    /// Create with custom heuristic patterns.
    pub fn with_patterns(patterns: Vec<String>) -> Self {
        Self {
            detector: PruningDetector::with_patterns(patterns),
        }
    }
}

impl Default for RetryPruning {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PruningStrategy for RetryPruning {
    fn name(&self) -> &'static str {
        "retry_pruning"
    }

    fn detect_candidates(&self, messages: &[ChatMessageRecord]) -> Vec<PruningCandidate> {
        self.detector.detect_failures(messages)
    }

    async fn prune(
        &self,
        candidates: &[PruningCandidate],
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
    ) -> Result<PruningReport, SessionError> {
        if candidates.is_empty() {
            return Ok(PruningReport::skipped(PruningStrategyType::Retry));
        }

        let tokens_before: u32 = candidates.iter().map(|c| c.estimated_tokens).sum();

        // Collect all message IDs for batch update.
        let all_ids: Vec<String> = candidates
            .iter()
            .flat_map(|c| c.message_ids.clone())
            .collect();

        let pruned_count = store
            .set_status_batch(session_id, &all_ids, ChatMessageStatus::Pruned)
            .await?;

        let total_pruned = pruned_count as usize;

        Ok(PruningReport {
            strategy_used: PruningStrategyType::Retry,
            messages_pruned: total_pruned,
            tokens_before,
            tokens_after: 0,
            tokens_saved: tokens_before,
            skipped: false,
            summary_inserted: false,
        })
    }
}

/// Calculate total tokens across a list of messages.
pub fn total_message_tokens(messages: &[ChatMessageRecord]) -> u32 {
    messages
        .iter()
        .map(|m| {
            // Use stored token counts if available, otherwise estimate.
            if let Some(input) = m.input_tokens {
                u32::try_from(input).unwrap_or(0)
            } else if let Some(output) = m.output_tokens {
                u32::try_from(output).unwrap_or(0)
            } else {
                estimate_tokens(&m.content)
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_pruning_name() {
        let strategy = RetryPruning::new();
        assert_eq!(strategy.name(), "retry_pruning");
    }

    #[test]
    fn test_detect_candidates_finds_errors() {
        let messages = vec![
            make_msg("m1", "user", "search"),
            make_msg("m2", "assistant", "calling tool"),
            make_msg("m3", "tool", "{\"error\": \"not found\"}"),
            make_msg("m4", "assistant", "trying again"),
            make_msg("m5", "tool", "{\"results\": [\"ok\"]}"),
        ];

        let strategy = RetryPruning::new();
        let candidates = strategy.detect_candidates(&messages);
        assert!(!candidates.is_empty());
    }

    #[test]
    fn test_detect_candidates_empty_on_clean() {
        let messages = vec![
            make_msg("m1", "user", "hello"),
            make_msg("m2", "assistant", "world"),
        ];

        let strategy = RetryPruning::new();
        let candidates = strategy.detect_candidates(&messages);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_total_message_tokens() {
        let messages = vec![
            make_msg("m1", "user", "hello world"),
            make_msg("m2", "assistant", "hi there"),
        ];
        let tokens = total_message_tokens(&messages);
        assert!(tokens > 0);
    }

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
}
