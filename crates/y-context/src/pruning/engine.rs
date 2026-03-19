//! PruningEngine: coordinates strategy selection, threshold evaluation,
//! and pruning execution.
//!
//! Design reference: context-pruning-design.md, Component Overview

use std::sync::Arc;

use y_core::agent::AgentDelegator;
use y_core::session::{ChatMessageRecord, ChatMessageStore, SessionError};
use y_core::types::SessionId;

use super::config::{PruningConfig, PruningStrategyMode};
use super::progressive::ProgressivePruning;
use super::report::{PruningReport, PruningStrategyType};
use super::retry::RetryPruning;
use super::strategy::PruningStrategy;

/// Coordinates pruning strategy selection and execution.
pub struct PruningEngine {
    config: PruningConfig,
    retry: RetryPruning,
    progressive: ProgressivePruning,
}

impl PruningEngine {
    /// Create with default configuration (no delegator -- progressive pruning
    /// will be skipped).
    pub fn new() -> Self {
        Self {
            config: PruningConfig::default(),
            retry: RetryPruning::new(),
            progressive: ProgressivePruning::new(),
        }
    }

    /// Create with custom configuration (no delegator).
    pub fn with_config(config: PruningConfig) -> Self {
        let retry = RetryPruning::with_patterns(config.retry.heuristic_patterns.clone());
        Self {
            config,
            retry,
            progressive: ProgressivePruning::new(),
        }
    }

    /// Create with custom configuration and an agent delegator for
    /// progressive pruning (subagent-based summarization).
    pub fn with_delegator(config: PruningConfig, delegator: Arc<dyn AgentDelegator>) -> Self {
        let retry = RetryPruning::with_patterns(config.retry.heuristic_patterns.clone());
        let progressive =
            ProgressivePruning::with_delegator(delegator, config.progressive.max_retries);
        Self {
            config,
            retry,
            progressive,
        }
    }

    /// Execute pruning on a set of messages.
    ///
    /// Returns a list of reports (one per strategy applied).
    /// Pruning is threshold-gated: only activates when candidate tokens
    /// exceed the configured threshold.
    pub async fn prune(
        &self,
        messages: &[ChatMessageRecord],
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
    ) -> Result<Vec<PruningReport>, SessionError> {
        if !self.config.enabled || messages.is_empty() {
            return Ok(vec![]);
        }

        let mut reports = Vec::new();

        match self.config.strategy {
            PruningStrategyMode::RetryOnly => {
                let report = self
                    .run_strategy(&self.retry, messages, store, session_id)
                    .await?;
                reports.push(report);
            }
            PruningStrategyMode::ProgressiveOnly => {
                let report = self
                    .run_strategy(&self.progressive, messages, store, session_id)
                    .await?;
                reports.push(report);
            }
            PruningStrategyMode::Auto => {
                // Auto mode: retry first, then progressive.
                let retry_report = self
                    .run_strategy(&self.retry, messages, store, session_id)
                    .await?;
                reports.push(retry_report);

                // Re-read active messages after retry pruning (some may have been pruned).
                let remaining = store.list_active(session_id).await?;
                let progressive_report = self
                    .run_strategy(&self.progressive, &remaining, store, session_id)
                    .await?;
                reports.push(progressive_report);
            }
        }

        Ok(reports)
    }

    /// Run a single strategy with threshold gating.
    async fn run_strategy(
        &self,
        strategy: &dyn PruningStrategy,
        messages: &[ChatMessageRecord],
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
    ) -> Result<PruningReport, SessionError> {
        let candidates = strategy.detect_candidates(messages);

        if candidates.is_empty() {
            tracing::debug!(strategy = strategy.name(), "no pruning candidates detected");
            return Ok(PruningReport::skipped(
                if strategy.name() == "retry_pruning" {
                    PruningStrategyType::Retry
                } else {
                    PruningStrategyType::Progressive
                },
            ));
        }

        // Threshold evaluation: sum candidate tokens.
        let total_candidate_tokens: u32 = candidates.iter().map(|c| c.estimated_tokens).sum();

        if total_candidate_tokens < self.config.token_threshold {
            tracing::debug!(
                strategy = strategy.name(),
                total_candidate_tokens,
                threshold = self.config.token_threshold,
                "pruning skipped: below threshold"
            );
            return Ok(PruningReport::skipped(
                if strategy.name() == "retry_pruning" {
                    PruningStrategyType::Retry
                } else {
                    PruningStrategyType::Progressive
                },
            ));
        }

        tracing::info!(
            strategy = strategy.name(),
            candidates = candidates.len(),
            total_candidate_tokens,
            "pruning threshold met, executing"
        );

        strategy.prune(&candidates, store, session_id).await
    }

    /// Access the current configuration.
    pub fn config(&self) -> &PruningConfig {
        &self.config
    }
}

impl Default for PruningEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::ChatMessageStatus;

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
    fn test_engine_default() {
        let engine = PruningEngine::new();
        assert!(engine.config().enabled);
        assert_eq!(engine.config().token_threshold, 2000);
    }

    #[test]
    fn test_engine_disabled() {
        let mut config = PruningConfig::default();
        config.enabled = false;
        let engine = PruningEngine::with_config(config);
        assert!(!engine.config().enabled);
    }

    #[test]
    fn test_engine_with_config() {
        let mut config = PruningConfig::default();
        config.token_threshold = 500;
        config.strategy = PruningStrategyMode::RetryOnly;
        let engine = PruningEngine::with_config(config);
        assert_eq!(engine.config().token_threshold, 500);
    }
}
