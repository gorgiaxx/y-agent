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
    /// Whether tool output pruning ran (superseded/useless elision).
    pub tool_output_pruned: bool,
    /// Tool results blanked by superseded/useless elision.
    pub tool_outputs_pruned: usize,
    /// Tokens saved by tool output pruning.
    pub tool_output_tokens_saved: u32,
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
            tool_output_pruned: false,
            tool_outputs_pruned: 0,
            tool_output_tokens_saved: 0,
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

/// Estimate tokens from character count (~4 chars per token).
///
/// Uses `chars().count()` rather than `len()` (byte count) so multi-byte
/// scripts (Chinese, Japanese, Korean) produce accurate estimates instead
/// of being inflated by UTF-8 encoding overhead.
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.chars().count().div_ceil(4)).unwrap_or(u32::MAX)
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
    /// Post-turn optimization: tool output pruning, then conditional pruning,
    /// then conditional compaction.
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

        // Step 0: Tool output pruning — blank superseded/useless tool results.
        // Runs first because it's zero-cost (no LLM call, no tombstone) and
        // reduces the token count that gates the subsequent steps.
        Self::run_tool_output_pruning(container, session_id, &mut report).await;

        // Step 1: Pruning -- gated by token growth delta.
        Self::run_pruning_if_needed(container, session_id, &mut report).await?;

        // Step 2: Compaction -- gated by percentage of context window.
        Self::run_compaction_if_needed(container, session_id, context_window, &mut report).await?;

        if report.pruning_ran || report.compaction_triggered || report.tool_output_pruned {
            tracing::info!(
                session_id = %session_id,
                tool_outputs_pruned = report.tool_outputs_pruned,
                tool_output_tokens_saved = report.tool_output_tokens_saved,
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
    // Tool output pruning helpers
    // -----------------------------------------------------------------------

    /// Blank superseded and useless tool results in the context transcript.
    ///
    /// This is a zero-cost (no LLM call, no tombstone) optimization that
    /// replaces old/empty tool result content with short placeholders. It
    /// runs before pruning and compaction to reduce the token count that
    /// gates those more expensive steps.
    ///
    /// The transcript is rewritten via truncate + append (same pattern as
    /// `sync_transcript_after_pruning`). The display transcript is never
    /// touched.
    async fn run_tool_output_pruning(
        container: &ServiceContainer,
        session_id: &SessionId,
        report: &mut OptimizationReport,
    ) {
        let transcript = match container.session_manager.read_transcript(session_id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read transcript for tool output pruning");
                return;
            }
        };

        if transcript.is_empty() {
            return;
        }

        let mut messages = transcript;
        let config = y_context::pruning::ToolOutputPruneConfig::default();
        let result = y_context::pruning::prune_tool_outputs(&mut messages, &config);

        if result.pruned_count == 0 {
            return;
        }

        report.tool_output_pruned = true;
        report.tool_outputs_pruned = result.pruned_count;
        report.tool_output_tokens_saved = result.tokens_saved;

        // In-place update: only rewrite the modified messages, preserving the
        // prompt cache prefix for all unchanged messages before them.
        let transcript_store = container.session_manager.transcript_store();
        for msg in &messages {
            if result.modified_message_ids.contains(&msg.message_id) {
                let _ = transcript_store
                    .update_message(session_id, &msg.message_id, msg)
                    .await;
            }
        }

        tracing::info!(
            session_id = %session_id,
            pruned_count = result.pruned_count,
            tokens_saved = result.tokens_saved,
            "tool output pruning applied"
        );
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

        // Update watermark to the post-prune token count. Pruning reduced
        // the active message set, so the next delta should measure growth
        // from the reduced baseline — otherwise the delta stays artificially
        // small and noise accumulates until the next trigger.
        let post_prune_tokens: u32 = container
            .chat_message_store
            .list_active(session_id)
            .await
            .map(|msgs| msgs.iter().map(|m| estimate_tokens(&m.content)).sum())
            .unwrap_or(current_tokens);
        container
            .pruning_watermarks
            .write()
            .await
            .insert(session_id.clone(), post_prune_tokens);

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
            "compaction threshold reached, trying handoff first"
        );

        // Try handoff first: generate a structured state document and
        // replace the transcript with it + recent messages. This preserves
        // decision rationale (Goal/Decisions/Progress) that flat compaction
        // loses — the core amnesia fix for long sessions.
        if let Some(handoff_gen) = &container.handoff_generator {
            match Self::run_handoff(container, handoff_gen, session_id, report).await {
                Ok(true) => return Ok(()), // handoff succeeded
                Ok(false) => {
                    // handoff returned no document, fall through
                    tracing::info!("handoff returned no document; falling back to compaction");
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "handoff failed; falling back to compaction"
                    );
                }
            }
        }

        // Fallback: traditional compaction.
        Self::run_compaction(container, session_id, report, None).await
    }

    /// Generate a handoff document and replace the transcript with it.
    ///
    /// Returns `Ok(true)` if the handoff succeeded and the transcript was
    /// rewritten, `Ok(false)` if the handoff generator returned no document
    /// (caller should fall back to compaction).
    async fn run_handoff(
        container: &ServiceContainer,
        handoff_gen: &y_context::HandoffGenerator,
        session_id: &SessionId,
        report: &mut OptimizationReport,
    ) -> Result<bool, OptimizationError> {
        // Detect any existing compaction summary to pass as context.
        let transcript = container
            .session_manager
            .read_transcript(session_id)
            .await
            .map_err(|e| OptimizationError::CompactionFailed(e.to_string()))?;

        let previous_summary: Option<String> = transcript.iter().find_map(|m| {
            if m.role == y_core::types::Role::System
                && m.metadata.get("type").and_then(|v| v.as_str()) == Some("compaction_summary")
            {
                Some(m.content.clone())
            } else {
                None
            }
        });

        let result = handoff_gen
            .generate(
                container.chat_message_store.as_ref(),
                session_id,
                previous_summary.as_deref(),
                None,
            )
            .await
            .ok_or_else(|| {
                OptimizationError::CompactionFailed("handoff generation failed".into())
            })?;

        if result.document.is_empty() {
            return Ok(false);
        }
        // Retain the most recent messages (same window as compaction).
        let retain_window = container.compaction_engine.config.retain_window;
        let retained_count = transcript.len().saturating_sub(retain_window);
        let retained = &transcript[retained_count.min(transcript.len())..];

        // Rewrite the transcript: handoff doc as system message + retained recent messages.
        let _ = container
            .session_manager
            .transcript_store()
            .truncate(session_id, 0)
            .await;

        let handoff_msg = y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::System,
            content: result.document.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({ "type": "handoff" }),
        };
        let _ = container
            .session_manager
            .transcript_store()
            .append(session_id, &handoff_msg)
            .await;

        for msg in retained {
            let _ = container
                .session_manager
                .transcript_store()
                .append(session_id, msg)
                .await;
        }

        report.compaction_triggered = true;
        report.compaction_summary.clone_from(&result.document);
        report.compaction_tokens_saved = 0; // handoff doesn't save tokens, it restructures them

        tracing::info!(
            session_id = %session_id,
            messages_compacted = retained_count,
            retained_count = retained.len(),
            "handoff document generated and transcript rewritten"
        );

        Ok(true)
    }

    /// Execute compaction on the context transcript.
    ///
    /// When `retain_window_override` is `Some(n)`, the compaction engine keeps
    /// only the last `n` messages instead of the configured default. This lets
    /// manual `/compact` work on short conversations.
    ///
    /// Uses `serialize_for_compaction` to preserve message structure
    /// (`tool_calls`, `tool_call_id` pairing) instead of flat string formatting.
    /// When a prior compaction summary exists in the transcript, passes it
    /// as `previous_summary` so the LLM merges rather than regenerates.
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

        // Structured serialization: preserves tool_calls and tool_call_id
        // pairing instead of flat "[Role] content" strings.
        let message_strings: Vec<String> = transcript
            .iter()
            .map(y_context::compaction::serialize_for_compaction)
            .collect();

        if message_strings.is_empty() {
            return Ok(());
        }

        // Detect a previous compaction summary in the transcript so the
        // LLM can merge rather than regenerate (prevents summary-of-summary
        // quality degradation across multiple compaction cycles).
        let previous_summary: Option<String> = transcript.iter().find_map(|m| {
            if m.role == y_core::types::Role::System
                && m.metadata.get("type").and_then(|v| v.as_str()) == Some("compaction_summary")
            {
                Some(m.content.clone())
            } else {
                None
            }
        });

        let result = if let Some(retain) = retain_window_override {
            container
                .compaction_engine
                .compact_async_with_retain_and_previous(
                    &message_strings,
                    retain,
                    previous_summary.as_deref(),
                )
                .await
        } else {
            container
                .compaction_engine
                .compact_async_with_retain_and_previous(
                    &message_strings,
                    container.compaction_engine.config.retain_window,
                    previous_summary.as_deref(),
                )
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

    #[test]
    fn test_estimate_tokens_cjk() {
        // 4 Chinese characters = 1 token (4 chars / 4).
        // Before the fix, 4 CJK chars = 12 bytes -> 3 tokens (inflated by UTF-8).
        assert_eq!(estimate_tokens("abcd"), 1);
        // Actual CJK characters (3 bytes each in UTF-8).
        assert_eq!(estimate_tokens("\u{4F60}\u{597D}\u{4E16}\u{754C}"), 1); // 4 CJK chars -> 1 token
        assert_eq!(estimate_tokens("\u{4F60}\u{597D}"), 1); // 2 chars -> ceil(2/4) = 1
                                                            // Mixed: 4 ASCII + 4 CJK = 8 chars -> 2 tokens.
        assert_eq!(estimate_tokens("abcd\u{4F60}\u{597D}\u{4E16}\u{754C}"), 2);
    }
}
