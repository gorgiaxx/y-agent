//! `ProgressivePruning` strategy: replaces completed multi-step sequences
//! with LLM-generated rolling summaries via subagent delegation.
//!
//! Architecture reference: `docs/guides/ARCHITECTURE.md`
//!
//! Delegates summarization to the `pruning-summarizer` built-in agent
//! via `AgentDelegator`, following the same pattern as `title-generator`.

use std::sync::Arc;

use async_trait::async_trait;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionError};
use y_core::types::SessionId;

use super::report::{PruningReport, PruningStrategyType};
use super::strategy::{PruningCandidate, PruningReason, PruningStrategy};
use crate::token_utils::estimate_tokens;

/// Progressive pruning: replaces completed multi-step sequences with
/// LLM-generated summaries via the `pruning-summarizer` subagent.
pub struct ProgressivePruning {
    delegator: Option<Arc<dyn AgentDelegator>>,
    max_retries: u32,
}

impl ProgressivePruning {
    /// Create without an agent delegator (will skip summarization).
    pub fn new() -> Self {
        Self {
            delegator: None,
            max_retries: 2,
        }
    }

    /// Create with an agent delegator for subagent-based summarization.
    pub fn with_delegator(delegator: Arc<dyn AgentDelegator>, max_retries: u32) -> Self {
        Self {
            delegator: Some(delegator),
            max_retries,
        }
    }

    /// Detect completed multi-step sequences eligible for summarization.
    ///
    /// A "completed sequence" is a series of assistant+tool message pairs
    /// followed by a final assistant message (the conclusion). Only the
    /// intermediate steps are candidates for summary; the conclusion is kept.
    fn detect_sequences(messages: &[ChatMessageRecord]) -> Vec<PruningCandidate> {
        let mut candidates = Vec::new();
        let mut sequence_start: Option<usize> = None;
        let mut sequence_ids: Vec<String> = Vec::new();
        let mut sequence_tokens: u32 = 0;

        for (i, msg) in messages.iter().enumerate() {
            match msg.role.as_str() {
                "assistant" => {
                    // Check if the next message is a tool result.
                    let has_tool_result = i + 1 < messages.len() && messages[i + 1].role == "tool";

                    if has_tool_result {
                        // This is an intermediate step (assistant + tool).
                        if sequence_start.is_none() {
                            sequence_start = Some(i);
                        }
                        sequence_ids.push(msg.id.clone());
                        sequence_tokens += estimate_tokens(&msg.content);
                    } else if sequence_start.is_some() && sequence_ids.len() >= 4 {
                        // This assistant message is the conclusion.
                        // The preceding sequence is a completed multi-step workflow.
                        candidates.push(PruningCandidate {
                            message_ids: std::mem::take(&mut sequence_ids),
                            estimated_tokens: sequence_tokens,
                            reason: PruningReason::CompletedSequence,
                        });
                        sequence_start = None;
                        sequence_tokens = 0;
                    } else {
                        // Reset: too short to be worth summarizing.
                        sequence_ids.clear();
                        sequence_start = None;
                        sequence_tokens = 0;
                    }
                }
                "tool" => {
                    if sequence_start.is_some() {
                        sequence_ids.push(msg.id.clone());
                        sequence_tokens += estimate_tokens(&msg.content);
                    }
                }
                "user" => {
                    // User message breaks any ongoing sequence.
                    if sequence_start.is_some() && sequence_ids.len() >= 4 {
                        candidates.push(PruningCandidate {
                            message_ids: std::mem::take(&mut sequence_ids),
                            estimated_tokens: sequence_tokens,
                            reason: PruningReason::CompletedSequence,
                        });
                    }
                    sequence_ids.clear();
                    sequence_start = None;
                    sequence_tokens = 0;
                }
                _ => {}
            }
        }

        candidates
    }

    /// Build structured input for the pruning-summarizer subagent.
    ///
    /// Includes the candidate messages **plus surrounding context**: the
    /// user message that initiated the workflow (so the summarizer knows
    /// *why* the tools were called) and the conclusion assistant message
    /// that followed (so it knows *what was concluded*). Without this
    /// context the summarizer produces a flat "explored X, Y, Z" list that
    /// loses the decision rationale — the root cause of task-amnesia.
    fn build_delegation_input(
        messages: &[ChatMessageRecord],
        candidate: &PruningCandidate,
    ) -> serde_json::Value {
        let candidate_set: std::collections::HashSet<&str> =
            candidate.message_ids.iter().map(String::as_str).collect();

        // Find the index range of the candidate sequence in the full message list.
        let first_idx = messages
            .iter()
            .position(|m| candidate_set.contains(m.id.as_str()));
        let last_idx = messages
            .iter()
            .rposition(|m| candidate_set.contains(m.id.as_str()));

        let mut workflow_messages: Vec<serde_json::Value> = Vec::new();

        // Include the preceding user message as context (the "why").
        if let Some(first) = first_idx {
            if first > 0 {
                for lookback in (1..=first.min(3)).rev() {
                    let prev = &messages[first - lookback];
                    if prev.role == "user" {
                        workflow_messages.push(serde_json::json!({
                            "role": "user",
                            "content": prev.content,
                            "_context": "preceding_user_instruction"
                        }));
                        break;
                    }
                }
            }
        }

        // The candidate messages themselves.
        for m in messages
            .iter()
            .filter(|m| candidate_set.contains(m.id.as_str()))
        {
            workflow_messages.push(serde_json::json!({ "role": m.role, "content": m.content }));
        }

        // Include the following assistant conclusion as context (the "what was concluded").
        if let Some(last) = last_idx {
            if last + 1 < messages.len() {
                let next = &messages[last + 1];
                if next.role == "assistant" {
                    workflow_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": next.content,
                        "_context": "conclusion"
                    }));
                }
            }
        }

        serde_json::json!({ "messages": workflow_messages })
    }

    /// Call the pruning-summarizer subagent with retry logic.
    async fn call_with_retry(
        &self,
        input: serde_json::Value,
        session_id: Option<uuid::Uuid>,
    ) -> Option<String> {
        let Some(delegator) = &self.delegator else {
            return None;
        };

        for attempt in 0..self.max_retries {
            match delegator
                .delegate(
                    "pruning-summarizer",
                    input.clone(),
                    ContextStrategyHint::None,
                    session_id,
                )
                .await
            {
                Ok(output) if !output.text.trim().is_empty() => {
                    tracing::debug!(attempt, "pruning-summarizer delegation succeeded");
                    return Some(output.text);
                }
                Ok(_) => {
                    tracing::warn!(attempt, "pruning-summarizer returned empty summary");
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "pruning-summarizer delegation failed");
                }
            }
        }

        tracing::warn!(
            max_retries = self.max_retries,
            "all pruning-summarizer retries exhausted; skipping progressive pruning"
        );
        None
    }
}

impl Default for ProgressivePruning {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PruningStrategy for ProgressivePruning {
    fn name(&self) -> &'static str {
        "progressive_pruning"
    }

    fn detect_candidates(&self, messages: &[ChatMessageRecord]) -> Vec<PruningCandidate> {
        Self::detect_sequences(messages)
    }

    async fn prune(
        &self,
        candidates: &[PruningCandidate],
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
    ) -> Result<PruningReport, SessionError> {
        if candidates.is_empty() || self.delegator.is_none() {
            return Ok(PruningReport::skipped(PruningStrategyType::Progressive));
        }

        let mut total_pruned: usize = 0;
        let mut total_tokens_before: u32 = 0;
        let mut total_tokens_after: u32 = 0;
        let mut any_summary_inserted = false;

        // Load all messages for input construction.
        let all_messages = store.list_by_session(session_id).await?;

        for candidate in candidates {
            total_tokens_before += candidate.estimated_tokens;

            let input = Self::build_delegation_input(&all_messages, candidate);

            let session_uuid = uuid::Uuid::parse_str(&session_id.0).ok();
            if let Some(summary) = self.call_with_retry(input, session_uuid).await {
                let summary_tokens = estimate_tokens(&summary);
                total_tokens_after += summary_tokens;

                // Insert summary as a new active message.
                let summary_record = ChatMessageRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    session_id: session_id.clone(),
                    role: "assistant".to_string(),
                    content: format!("[Pruning Summary] {summary}"),
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
                };
                store.insert(&summary_record).await?;

                // Mark original messages as pruned.
                let pruned = store
                    .set_status_batch(
                        session_id,
                        &candidate.message_ids,
                        ChatMessageStatus::Pruned,
                    )
                    .await?;

                total_pruned += pruned as usize;
                any_summary_inserted = true;
            } else {
                // Subagent failed -- skip this candidate (safe default).
                total_tokens_after += candidate.estimated_tokens;
                tracing::warn!(
                    candidate_messages = candidate.message_ids.len(),
                    "progressive pruning skipped for candidate: delegator unavailable"
                );
            }
        }

        Ok(PruningReport {
            strategy_used: PruningStrategyType::Progressive,
            messages_pruned: total_pruned,
            tokens_before: total_tokens_before,
            tokens_after: total_tokens_after,
            tokens_saved: total_tokens_before.saturating_sub(total_tokens_after),
            skipped: total_pruned == 0,
            summary_inserted: any_summary_inserted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_detect_multi_step_sequence() {
        let messages = vec![
            make_msg("m1", "user", "browse website"),
            make_msg("m2", "assistant", "calling browser_open"),
            make_msg("m3", "tool", "page loaded"),
            make_msg("m4", "assistant", "calling browser_click"),
            make_msg("m5", "tool", "clicked button"),
            make_msg("m6", "assistant", "calling browser_extract"),
            make_msg("m7", "tool", "extracted data: XYZ"),
            make_msg("m8", "assistant", "Here is the result: XYZ"),
        ];

        let strategy = ProgressivePruning::new();
        let candidates = strategy.detect_candidates(&messages);
        assert!(!candidates.is_empty());

        let seq = &candidates[0];
        assert_eq!(seq.reason, PruningReason::CompletedSequence);
        // Should include the intermediate steps but NOT the final assistant message.
        assert!(seq.message_ids.len() >= 4);
        assert!(!seq.message_ids.contains(&"m8".to_string()));
    }

    #[test]
    fn test_no_sequence_for_short_interactions() {
        let messages = vec![
            make_msg("m1", "user", "hello"),
            make_msg("m2", "assistant", "calling tool"),
            make_msg("m3", "tool", "result"),
            make_msg("m4", "assistant", "done"),
        ];

        let strategy = ProgressivePruning::new();
        let candidates = strategy.detect_candidates(&messages);
        // Only 1 assistant+tool pair -- too short.
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_progressive_name() {
        let strategy = ProgressivePruning::new();
        assert_eq!(strategy.name(), "progressive_pruning");
    }

    #[test]
    fn test_build_delegation_input_includes_context() {
        let messages = vec![
            make_msg("u1", "user", "do task A"),
            make_msg("m1", "assistant", "calling tool A"),
            make_msg("m2", "tool", "result A"),
            make_msg("m3", "assistant", "Task A found issue: missing config"),
        ];
        let candidate = PruningCandidate {
            message_ids: vec!["m1".into(), "m2".into()],
            estimated_tokens: 100,
            reason: PruningReason::CompletedSequence,
        };
        let input = ProgressivePruning::build_delegation_input(&messages, &candidate);
        let msgs = input["messages"].as_array().unwrap();
        // 1 preceding user + 2 candidate + 1 conclusion = 4
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["_context"], "preceding_user_instruction");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[3]["role"], "assistant");
        assert_eq!(msgs[3]["_context"], "conclusion");
    }
}
