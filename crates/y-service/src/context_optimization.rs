//! Context Optimization Service -- orchestrates pruning and compaction.
//!
//! **Pruning** is delta-triggered: it only runs when the token growth since
//! the last pruning exceeds `PruningConfig.token_threshold`. Per-session
//! token watermarks are tracked in-memory on `ServiceContainer`.
//!
//! **Compaction** is percentage-triggered: it runs when the total transcript
//! token count exceeds `compaction_threshold_pct` of the serving provider's
//! context window.
//!
//! Manual compaction (`compact_now`) bypasses both thresholds.

use y_core::session::ChatMessageStore;
use y_core::types::SessionId;

use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Report types
// ---------------------------------------------------------------------------

/// Summary of what the post-turn optimization did.
#[derive(Debug, Clone)]
pub struct OptimizationReport {
    /// Whether pruning ran and found candidates.
    pub pruning_ran: bool,
    /// Total messages pruned across all strategies.
    pub messages_pruned: usize,
    /// Estimated tokens saved by pruning.
    pub pruning_tokens_saved: u32,
    /// Whether compaction was triggered.
    pub compaction_triggered: bool,
    /// Messages compacted (if compaction ran).
    pub messages_compacted: usize,
    /// Tokens saved by compaction (if compaction ran).
    pub compaction_tokens_saved: u32,
    /// The compaction summary text (empty if compaction did not run).
    pub compaction_summary: String,
}

impl OptimizationReport {
    fn empty() -> Self {
        Self {
            pruning_ran: false,
            messages_pruned: 0,
            pruning_tokens_saved: 0,
            compaction_triggered: false,
            messages_compacted: 0,
            compaction_tokens_saved: 0,
            compaction_summary: String::new(),
        }
    }
}

/// Errors from the optimization service.
#[derive(Debug, thiserror::Error)]
pub enum OptimizationError {
    #[error("pruning failed: {0}")]
    PruningFailed(String),

    #[error("compaction failed: {0}")]
    CompactionFailed(String),
}

// ---------------------------------------------------------------------------
// Token estimation helper
// ---------------------------------------------------------------------------

/// Estimate tokens from text length (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Orchestrates both pruning and compaction in the right order.
///
/// **Pruning trigger**: delta-based. Tracks the token count at which pruning
/// last ran for each session. Only triggers when the growth (current tokens
/// minus last-pruned tokens) exceeds `PruningConfig.token_threshold`.
///
/// **Compaction trigger**: percentage-based. Triggers when the total token
/// count exceeds `compaction_threshold_pct` of the provider's context window.
pub struct ContextOptimizationService;

impl ContextOptimizationService {
    /// Post-turn optimization: conditional pruning, then conditional compaction.
    ///
    /// Fire-and-forget from the caller's perspective -- errors are logged
    /// but never block the turn result.
    ///
    /// A single "turn" in LLM API terms = user sends a message + assistant
    /// replies completely. This method is called once per turn.
    pub async fn optimize_post_turn(
        container: &ServiceContainer,
        session_id: &SessionId,
        context_window: usize,
    ) -> Result<OptimizationReport, OptimizationError> {
        let mut report = OptimizationReport::empty();

        // Step 1: Pruning -- gated by token growth delta.
        Self::run_pruning_if_needed(container, session_id, &mut report).await?;

        // Step 2: Compaction -- gated by percentage of context window.
        Self::run_compaction_if_needed(container, session_id, context_window, &mut report).await?;

        if report.pruning_ran || report.compaction_triggered {
            tracing::info!(
                session_id = %session_id,
                messages_pruned = report.messages_pruned,
                pruning_tokens_saved = report.pruning_tokens_saved,
                compaction_triggered = report.compaction_triggered,
                messages_compacted = report.messages_compacted,
                compaction_tokens_saved = report.compaction_tokens_saved,
                "post-turn context optimization complete"
            );
        }

        Ok(report)
    }

    /// Manual compaction: bypasses threshold check, compacts immediately.
    ///
    /// Used by the `/compact` slash command. Uses a small retain window (2)
    /// so even short conversations can be compacted on demand.
    pub async fn compact_now(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Result<OptimizationReport, OptimizationError> {
        // Retain window for manual compaction -- keep only the last
        // user+assistant exchange so /compact works even on short
        // conversations. The automatic post-turn path uses the configured
        // default (typically 10).
        const MANUAL_RETAIN_WINDOW: usize = 2;

        let mut report = OptimizationReport::empty();

        // Force pruning (bypass delta check).
        Self::run_pruning(container, session_id, &mut report).await?;

        // Force compaction with the small retain window.
        Self::run_compaction(
            container,
            session_id,
            &mut report,
            Some(MANUAL_RETAIN_WINDOW),
        )
        .await?;

        // Reset the watermark after forced optimization.
        Self::update_pruning_watermark(container, session_id).await;

        tracing::info!(
            session_id = %session_id,
            messages_pruned = report.messages_pruned,
            messages_compacted = report.messages_compacted,
            compaction_tokens_saved = report.compaction_tokens_saved,
            "manual compaction complete"
        );

        Ok(report)
    }

    // -----------------------------------------------------------------------
    // Pruning helpers
    // -----------------------------------------------------------------------

    /// Check token growth delta against threshold and prune if exceeded.
    ///
    /// The workflow:
    /// 1. Estimate current session token count from `ChatMessageStore`
    /// 2. Read the last-pruned watermark for this session
    /// 3. If `current - watermark >= token_threshold`, run pruning
    /// 4. After pruning, update the watermark to the new token count
    async fn run_pruning_if_needed(
        container: &ServiceContainer,
        session_id: &SessionId,
        report: &mut OptimizationReport,
    ) -> Result<(), OptimizationError> {
        let engine = &container.pruning_engine;

        if !engine.config().enabled {
            return Ok(());
        }

        let messages = container
            .chat_message_store
            .list_active(session_id)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        if messages.is_empty() {
            return Ok(());
        }

        // Estimate current total tokens from active messages.
        let current_tokens: u32 = messages.iter().map(|m| estimate_tokens(&m.content)).sum();

        // Read last-pruned watermark for this session.
        let last_pruned = container
            .pruning_watermarks
            .read()
            .await
            .get(session_id)
            .copied()
            .unwrap_or(0);

        let delta = current_tokens.saturating_sub(last_pruned);
        let threshold = engine.config().token_threshold;

        if delta < threshold {
            tracing::debug!(
                session_id = %session_id,
                current_tokens,
                last_pruned,
                delta,
                threshold,
                "pruning skipped: token growth below threshold"
            );
            return Ok(());
        }

        tracing::info!(
            session_id = %session_id,
            current_tokens,
            last_pruned,
            delta,
            threshold,
            "pruning threshold reached, executing"
        );

        // Actually run pruning.
        Self::run_pruning_inner(container, &messages, session_id, report).await?;

        // Update watermark to current tokens (post-pruning token count may
        // differ, but using current_tokens is correct because the next delta
        // should measure from this checkpoint, not from the post-pruned count).
        container
            .pruning_watermarks
            .write()
            .await
            .insert(session_id.clone(), current_tokens);

        Ok(())
    }

    /// Execute pruning via `PruningEngine` unconditionally (bypasses delta check).
    async fn run_pruning(
        container: &ServiceContainer,
        session_id: &SessionId,
        report: &mut OptimizationReport,
    ) -> Result<(), OptimizationError> {
        let engine = &container.pruning_engine;

        if !engine.config().enabled {
            return Ok(());
        }

        let messages = container
            .chat_message_store
            .list_active(session_id)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        if messages.is_empty() {
            return Ok(());
        }

        Self::run_pruning_inner(container, &messages, session_id, report).await
    }

    /// Core pruning logic shared by conditional and forced paths.
    ///
    /// After the `PruningEngine` marks messages as `Pruned` in `ChatMessageStore`,
    /// this method syncs the context transcript (JSONL) by removing pruned messages.
    /// Without this sync, pruning would have no effect on the actual LLM context
    /// because the JSONL transcript is the source of truth for context assembly.
    ///
    /// The display transcript is intentionally left unmodified -- by design it
    /// is never compacted so users always see the full conversation history.
    async fn run_pruning_inner(
        container: &ServiceContainer,
        messages: &[y_core::session::ChatMessageRecord],
        session_id: &SessionId,
        report: &mut OptimizationReport,
    ) -> Result<(), OptimizationError> {
        let reports = container
            .pruning_engine
            .prune(messages, container.chat_message_store.as_ref(), session_id)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        let mut total_pruned = 0;
        for r in &reports {
            if !r.skipped {
                report.pruning_ran = true;
                report.messages_pruned += r.messages_pruned;
                report.pruning_tokens_saved += r.tokens_saved;
                total_pruned += r.messages_pruned;
            }
        }

        // Sync context transcript: remove pruned messages from the JSONL
        // transcript so the next LLM call sees a smaller context.
        if total_pruned > 0 {
            if let Err(e) = Self::sync_transcript_after_pruning(container, session_id).await {
                tracing::warn!(
                    error = %e,
                    session_id = %session_id,
                    "failed to sync context transcript after pruning"
                );
            }
        }

        Ok(())
    }

    /// Rebuild the context transcript to exclude pruned messages.
    ///
    /// Reads the active message set from `ChatMessageStore` (which has pruned
    /// messages filtered out) and rebuilds the context transcript to match.
    /// Uses message IDs for precise matching.
    async fn sync_transcript_after_pruning(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Result<(), OptimizationError> {
        // 1. Get the set of active message IDs from the SQLite store.
        let active_records = container
            .chat_message_store
            .list_active(session_id)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        let active_ids: std::collections::HashSet<&str> =
            active_records.iter().map(|r| r.id.as_str()).collect();

        // 2. Read the current context transcript (JSONL).
        let transcript = container
            .session_manager
            .read_transcript(session_id)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        // 3. Filter: keep only messages whose message_id is in the active set.
        //    Also keep system messages (they have no ChatMessageStore record).
        let retained: Vec<&y_core::types::Message> = transcript
            .iter()
            .filter(|m| {
                m.role == y_core::types::Role::System || active_ids.contains(m.message_id.as_str())
            })
            .collect();

        let pruned_count = transcript.len() - retained.len();
        if pruned_count == 0 {
            return Ok(());
        }

        tracing::info!(
            session_id = %session_id,
            transcript_before = transcript.len(),
            retained = retained.len(),
            pruned = pruned_count,
            "syncing context transcript after pruning"
        );

        // 4. Truncate and re-write.
        let transcript_store = container.session_manager.transcript_store();
        transcript_store
            .truncate(session_id, 0)
            .await
            .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;

        for msg in retained {
            transcript_store
                .append(session_id, msg)
                .await
                .map_err(|e| OptimizationError::PruningFailed(e.to_string()))?;
        }

        Ok(())
    }

    /// Update the pruning watermark to the current session token count.
    async fn update_pruning_watermark(container: &ServiceContainer, session_id: &SessionId) {
        let current_tokens = match container.chat_message_store.list_active(session_id).await {
            Ok(msgs) => msgs.iter().map(|m| estimate_tokens(&m.content)).sum(),
            Err(_) => 0,
        };
        container
            .pruning_watermarks
            .write()
            .await
            .insert(session_id.clone(), current_tokens);
    }

    // -----------------------------------------------------------------------
    // Compaction helpers
    // -----------------------------------------------------------------------

    /// Check context usage against the compaction threshold and compact if needed.
    async fn run_compaction_if_needed(
        container: &ServiceContainer,
        session_id: &SessionId,
        context_window: usize,
        report: &mut OptimizationReport,
    ) -> Result<(), OptimizationError> {
        if context_window == 0 {
            return Ok(());
        }

        // Read the current context transcript to estimate token usage.
        let transcript = container
            .session_manager
            .read_transcript(session_id)
            .await
            .map_err(|e| OptimizationError::CompactionFailed(e.to_string()))?;

        // Estimate total tokens in the transcript.
        let total_tokens: u32 = transcript.iter().map(|m| estimate_tokens(&m.content)).sum();

        let context_window_u32 = u32::try_from(context_window).unwrap_or(u32::MAX);

        // Use the guard's compaction threshold (percentage) to decide.
        let threshold_pct = container.compaction_threshold_pct;
        let threshold_tokens = u64::from(context_window_u32) * u64::from(threshold_pct) / 100;
        let threshold_tokens = u32::try_from(threshold_tokens).unwrap_or(u32::MAX);

        if total_tokens < threshold_tokens {
            tracing::debug!(
                session_id = %session_id,
                total_tokens,
                threshold_tokens,
                threshold_pct,
                "compaction not needed: below threshold"
            );
            return Ok(());
        }

        tracing::info!(
            session_id = %session_id,
            total_tokens,
            threshold_tokens,
            threshold_pct,
            "compaction threshold reached, triggering compaction"
        );

        Self::run_compaction(container, session_id, report, None).await
    }

    /// Execute compaction on the context transcript.
    ///
    /// When `retain_window_override` is `Some(n)`, the compaction engine keeps
    /// only the last `n` messages instead of the configured default. This lets
    /// manual `/compact` work on short conversations.
    async fn run_compaction(
        container: &ServiceContainer,
        session_id: &SessionId,
        report: &mut OptimizationReport,
        retain_window_override: Option<usize>,
    ) -> Result<(), OptimizationError> {
        let transcript = container
            .session_manager
            .read_transcript(session_id)
            .await
            .map_err(|e| OptimizationError::CompactionFailed(e.to_string()))?;

        let message_strings: Vec<String> = transcript
            .iter()
            .map(|m| format!("[{:?}] {}", m.role, m.content))
            .collect();

        if message_strings.is_empty() {
            return Ok(());
        }

        let result = if let Some(retain) = retain_window_override {
            container
                .compaction_engine
                .compact_async_with_retain(&message_strings, retain)
                .await
        } else {
            container
                .compaction_engine
                .compact_async(&message_strings)
                .await
        };

        if result.messages_compacted > 0 {
            report.compaction_triggered = true;
            report.messages_compacted = result.messages_compacted;
            report.compaction_tokens_saved = result.tokens_saved;
            report.compaction_summary.clone_from(&result.summary);

            // Keep the messages that were NOT compacted (the recent ones).
            let retained = &transcript[result.messages_compacted..];
            let retained_count = retained.len();

            // Truncate context transcript and re-write with summary + retained.
            let _ = container
                .session_manager
                .transcript_store()
                .truncate(session_id, 0)
                .await;

            // Insert summary as a system message.
            if !result.summary.is_empty() {
                let summary_msg = y_core::types::Message {
                    message_id: y_core::types::generate_message_id(),
                    role: y_core::types::Role::System,
                    content: result.summary,
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: y_core::types::now(),
                    metadata: serde_json::json!({ "type": "compaction_summary" }),
                };
                let _ = container
                    .session_manager
                    .transcript_store()
                    .append(session_id, &summary_msg)
                    .await;
            }

            // Re-append the retained messages.
            for msg in retained {
                let _ = container
                    .session_manager
                    .transcript_store()
                    .append(session_id, msg)
                    .await;
            }

            tracing::info!(
                messages_compacted = result.messages_compacted,
                retained_count,
                tokens_saved = result.tokens_saved,
                "context transcript compacted"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimization_report_empty() {
        let report = OptimizationReport::empty();
        assert!(!report.pruning_ran);
        assert_eq!(report.messages_pruned, 0);
        assert!(!report.compaction_triggered);
        assert_eq!(report.messages_compacted, 0);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("1234"), 1);
        assert_eq!(estimate_tokens("12345"), 2);
        assert_eq!(estimate_tokens("12345678"), 2);
    }
}
