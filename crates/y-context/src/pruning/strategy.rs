//! Pruning strategy trait and shared types.

use async_trait::async_trait;
use y_core::session::{ChatMessageRecord, ChatMessageStore, SessionError};
use y_core::types::SessionId;

use super::report::PruningReport;

/// Reason a pruning candidate was identified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruningReason {
    /// Tool call returned an error status.
    ErrorStatus,
    /// Repeated identical tool calls detected.
    RepeatedCalls,
    /// Tool result contained empty/unhelpful output.
    EmptyResult,
    /// Completed multi-step sequence eligible for summarization.
    CompletedSequence,
}

/// A group of messages identified as a pruning candidate.
#[derive(Debug, Clone)]
pub struct PruningCandidate {
    /// Message IDs to prune.
    pub message_ids: Vec<String>,
    /// Estimated tokens in these messages.
    pub estimated_tokens: u32,
    /// Why these messages were identified for pruning.
    pub reason: PruningReason,
}

/// Trait for pruning strategies.
///
/// Each strategy implements detect-evaluate-prune lifecycle.
/// Summarization for progressive pruning is handled via `AgentDelegator`
/// delegation to the `pruning-summarizer` built-in agent.
#[async_trait]
pub trait PruningStrategy: Send + Sync {
    /// Human-readable name for logging.
    fn name(&self) -> &'static str;

    /// Detect candidate message groups for pruning.
    fn detect_candidates(&self, messages: &[ChatMessageRecord]) -> Vec<PruningCandidate>;

    /// Execute pruning on the detected candidates.
    async fn prune(
        &self,
        candidates: &[PruningCandidate],
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
    ) -> Result<PruningReport, SessionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pruning_candidate_construction() {
        let candidate = PruningCandidate {
            message_ids: vec!["m1".into(), "m2".into()],
            estimated_tokens: 500,
            reason: PruningReason::ErrorStatus,
        };
        assert_eq!(candidate.message_ids.len(), 2);
        assert_eq!(candidate.estimated_tokens, 500);
        assert_eq!(candidate.reason, PruningReason::ErrorStatus);
    }

    #[test]
    fn test_pruning_reason_variants() {
        assert_ne!(PruningReason::ErrorStatus, PruningReason::EmptyResult);
        assert_ne!(
            PruningReason::RepeatedCalls,
            PruningReason::CompletedSequence
        );
    }
}
