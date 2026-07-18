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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkingHistoryOptimization {
    NotNeeded,
    Applied,
    #[cfg(feature = "compaction_prefire")]
    Suppressed,
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

#[cfg(feature = "compaction_prefire")]
fn threshold_tokens(context_window: usize, threshold_pct: u32) -> u32 {
    let context_window = u64::try_from(context_window).unwrap_or(u64::MAX);
    let tokens = context_window.saturating_mul(u64::from(threshold_pct.min(100))) / 100;
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

fn find_previous_compaction_summary(transcript: &[y_core::types::Message]) -> Option<String> {
    transcript.iter().find_map(|message| {
        (message.role == y_core::types::Role::System
            && message
                .metadata
                .get("type")
                .and_then(|value| value.as_str())
                == Some("compaction_summary"))
        .then(|| message.content.clone())
    })
}

async fn replace_context_transcript(
    container: &ServiceContainer,
    session_id: &SessionId,
    messages: &[y_core::types::Message],
) -> Result<(), OptimizationError> {
    let transcript_store = container.session_manager.transcript_store();
    transcript_store
        .truncate(session_id, 0)
        .await
        .map_err(|error| OptimizationError::CompactionFailed(error.to_string()))?;
    for message in messages {
        transcript_store
            .append(session_id, message)
            .await
            .map_err(|error| OptimizationError::CompactionFailed(error.to_string()))?;
    }
    Ok(())
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
    pub(crate) async fn optimize_working_history_before_sampling(
        container: &ServiceContainer,
        session_id: &SessionId,
        history: &mut Vec<y_core::types::Message>,
        request_prefix_len: usize,
        request: &y_core::provider::ChatRequest,
        context_window: usize,
        emergency: bool,
    ) -> Result<WorkingHistoryOptimization, OptimizationError> {
        #[cfg(not(feature = "compaction_prefire"))]
        tracing::trace!(%session_id, "compaction prefire feature is disabled");

        if !emergency {
            let estimate = y_context::sampling::estimate_sampling_tokens(request);
            if !matches!(
                y_context::sampling::sampling_preflight(
                    estimate,
                    context_window,
                    container.compaction_threshold_pct,
                ),
                y_context::sampling::SamplingPreflightVerdict::Compact { .. }
            ) {
                return Ok(WorkingHistoryOptimization::NotNeeded);
            }
        }

        let request_prefix_len = request_prefix_len.min(history.len());
        let transcript_history = &history[request_prefix_len..];
        let retain_window = if emergency {
            2
        } else {
            container.compaction_engine.config.retain_window
        };
        let Some(default_range) =
            y_context::sampling::safe_compaction_range(transcript_history, retain_window)
        else {
            if emergency {
                return Err(OptimizationError::CompactionFailed(
                    "no protocol-safe history range is available for emergency compaction"
                        .to_string(),
                ));
            }
            return Ok(WorkingHistoryOptimization::NotNeeded);
        };

        #[cfg(feature = "compaction_prefire")]
        let prefired = if emergency {
            None
        } else if let Some(key) = container
            .compaction_prefire_registry
            .pending_key(session_id)
            .await
            .filter(|key| key.range.end <= transcript_history.len())
        {
            let fingerprint = y_context::sampling::compaction_prefix_fingerprint(
                transcript_history,
                key.range.clone(),
                &container.compaction_engine.config,
            );
            match container
                .compaction_prefire_registry
                .consume(session_id, &fingerprint)
                .await
            {
                crate::compaction_prefire::PrefireConsume::Ready { key, result } => {
                    Some((key.range, result))
                }
                crate::compaction_prefire::PrefireConsume::Failed { failure, .. } => {
                    return Err(OptimizationError::CompactionFailed(format!(
                        "prefired compaction failed: {failure}"
                    )));
                }
                crate::compaction_prefire::PrefireConsume::Suppressed => {
                    return Ok(WorkingHistoryOptimization::Suppressed);
                }
                crate::compaction_prefire::PrefireConsume::Miss
                | crate::compaction_prefire::PrefireConsume::Stale => None,
            }
        } else {
            None
        };
        #[cfg(not(feature = "compaction_prefire"))]
        let prefired: Option<(std::ops::Range<usize>, y_context::CompactionResult)> = None;

        let (range, result) = if let Some(prefired) = prefired {
            prefired
        } else {
            #[cfg(feature = "compaction_prefire")]
            let fingerprint = y_context::sampling::compaction_prefix_fingerprint(
                transcript_history,
                default_range.clone(),
                &container.compaction_engine.config,
            );
            #[cfg(feature = "compaction_prefire")]
            if container
                .compaction_prefire_registry
                .is_suppressed(session_id, &fingerprint)
                .await
            {
                return Ok(WorkingHistoryOptimization::Suppressed);
            }

            let effective_retain = transcript_history.len().saturating_sub(default_range.end);
            let message_strings = transcript_history[default_range.start..]
                .iter()
                .map(y_context::compaction::serialize_for_compaction)
                .collect::<Vec<_>>();
            let previous_summary = find_previous_compaction_summary(transcript_history);
            let result = container
                .compaction_engine
                .compact_async_with_retain_and_previous(
                    &message_strings,
                    effective_retain,
                    previous_summary.as_deref(),
                )
                .await;

            if let y_context::CompactionOutcome::Fallback { failure } = &result.outcome {
                #[cfg(feature = "compaction_prefire")]
                if let Some(failure) = failure {
                    container
                        .compaction_prefire_registry
                        .record_failure(
                            session_id.clone(),
                            crate::compaction_prefire::PrefireKey {
                                fingerprint,
                                range: default_range.clone(),
                            },
                            failure.class,
                        )
                        .await;
                }
                let message = failure.as_ref().map_or_else(
                    || "compaction produced a fallback result".to_string(),
                    ToString::to_string,
                );
                return Err(OptimizationError::CompactionFailed(message));
            }
            (default_range, result)
        };

        if result.messages_compacted != range.len() || result.summary.trim().is_empty() {
            return Err(OptimizationError::CompactionFailed(
                "compaction result did not match the selected history range".to_string(),
            ));
        }

        let absolute_start = request_prefix_len.saturating_add(range.start);
        let absolute_end = request_prefix_len.saturating_add(range.end);
        let mut optimized =
            Vec::with_capacity(history.len().saturating_sub(range.len()).saturating_add(1));
        optimized.extend_from_slice(&history[..absolute_start]);
        optimized.push(y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::System,
            content: result.summary,
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({ "type": "compaction_summary" }),
        });
        optimized.extend_from_slice(&history[absolute_end..]);
        *history = optimized;

        Ok(WorkingHistoryOptimization::Applied)
    }

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

        // Step 2: Immutable compaction prefire below the hard threshold.
        Self::schedule_prefire_if_needed(container, session_id, context_window).await?;

        // Step 3: Compaction -- gated by percentage of context window.
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

    #[cfg(feature = "compaction_prefire")]
    async fn schedule_prefire_if_needed(
        container: &ServiceContainer,
        session_id: &SessionId,
        context_window: usize,
    ) -> Result<(), OptimizationError> {
        if context_window == 0 {
            return Ok(());
        }
        let transcript = container
            .session_manager
            .read_transcript(session_id)
            .await
            .map_err(|error| OptimizationError::CompactionFailed(error.to_string()))?;
        if let Some(key) = container
            .compaction_prefire_registry
            .pending_key(session_id)
            .await
            .filter(|key| key.range.end <= transcript.len())
        {
            let current_fingerprint = y_context::sampling::compaction_prefix_fingerprint(
                &transcript,
                key.range,
                &container.compaction_engine.config,
            );
            if current_fingerprint == key.fingerprint {
                return Ok(());
            }
        }
        let total_tokens: u32 = transcript
            .iter()
            .map(|message| estimate_tokens(&message.content))
            .sum();
        let hard_threshold = threshold_tokens(context_window, container.compaction_threshold_pct);
        let prefire_pct = container
            .compaction_prefire_threshold_pct
            .min(container.compaction_threshold_pct.saturating_sub(1));
        let prefire_threshold = threshold_tokens(context_window, prefire_pct);
        if total_tokens < prefire_threshold || total_tokens >= hard_threshold {
            return Ok(());
        }

        let Some(range) = y_context::sampling::safe_compaction_range(
            &transcript,
            container.compaction_engine.config.retain_window,
        ) else {
            return Ok(());
        };
        let fingerprint = y_context::sampling::compaction_prefix_fingerprint(
            &transcript,
            range.clone(),
            &container.compaction_engine.config,
        );
        if container
            .compaction_prefire_registry
            .is_suppressed(session_id, &fingerprint)
            .await
        {
            return Ok(());
        }

        let effective_retain = transcript.len().saturating_sub(range.end);
        let message_strings = transcript[range.start..]
            .iter()
            .map(y_context::compaction::serialize_for_compaction)
            .collect::<Vec<_>>();
        let previous_summary = find_previous_compaction_summary(&transcript);
        let engine = container.compaction_engine.clone();
        let key = crate::compaction_prefire::PrefireKey { fingerprint, range };
        let scheduled = container
            .compaction_prefire_registry
            .schedule(session_id.clone(), key, async move {
                engine
                    .compact_async_with_retain_and_previous(
                        &message_strings,
                        effective_retain,
                        previous_summary.as_deref(),
                    )
                    .await
            })
            .await;
        if scheduled {
            tracing::info!(
                session_id = %session_id,
                total_tokens,
                prefire_threshold,
                hard_threshold,
                "compaction prefire scheduled"
            );
        }
        Ok(())
    }

    #[cfg(not(feature = "compaction_prefire"))]
    fn schedule_prefire_if_needed(
        _container: &ServiceContainer,
        _session_id: &SessionId,
        _context_window: usize,
    ) -> std::future::Ready<Result<(), OptimizationError>> {
        std::future::ready(Ok(()))
    }

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
        let retain_window = container.compaction_engine.config.retain_window;
        let Some(compaction_range) =
            y_context::sampling::safe_compaction_range(&transcript, retain_window)
        else {
            return Ok(false);
        };

        let handoff_msg = y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: y_core::types::Role::System,
            content: result.document.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({ "type": "handoff" }),
        };
        let mut rewritten = Vec::with_capacity(
            transcript
                .len()
                .saturating_sub(compaction_range.len())
                .saturating_add(1),
        );
        rewritten.extend_from_slice(&transcript[..compaction_range.start]);
        rewritten.push(handoff_msg);
        rewritten.extend_from_slice(&transcript[compaction_range.end..]);
        replace_context_transcript(container, session_id, &rewritten).await?;

        #[cfg(feature = "compaction_prefire")]
        container
            .compaction_prefire_registry
            .clear(session_id)
            .await;

        report.compaction_triggered = true;
        report.messages_compacted = compaction_range.len();
        report.compaction_summary.clone_from(&result.document);
        report.compaction_tokens_saved = 0; // handoff doesn't save tokens, it restructures them

        tracing::info!(
            session_id = %session_id,
            messages_compacted = compaction_range.len(),
            retained_count = transcript.len().saturating_sub(compaction_range.end),
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

        let retain_window =
            retain_window_override.unwrap_or(container.compaction_engine.config.retain_window);
        let Some(default_compaction_range) =
            y_context::sampling::safe_compaction_range(&transcript, retain_window)
        else {
            return Ok(());
        };
        let default_fingerprint = y_context::sampling::compaction_prefix_fingerprint(
            &transcript,
            default_compaction_range.clone(),
            &container.compaction_engine.config,
        );

        #[cfg(feature = "compaction_prefire")]
        if retain_window_override.is_none()
            && container
                .compaction_prefire_registry
                .is_suppressed(session_id, &default_fingerprint)
                .await
        {
            return Err(OptimizationError::CompactionFailed(
                "compaction is suppressed for the unchanged source fingerprint".to_string(),
            ));
        }

        #[cfg(feature = "compaction_prefire")]
        let prefired = if retain_window_override.is_none() {
            if let Some(key) = container
                .compaction_prefire_registry
                .pending_key(session_id)
                .await
                .filter(|key| key.range.end <= transcript.len())
            {
                let fingerprint = y_context::sampling::compaction_prefix_fingerprint(
                    &transcript,
                    key.range.clone(),
                    &container.compaction_engine.config,
                );
                match container
                    .compaction_prefire_registry
                    .consume(session_id, &fingerprint)
                    .await
                {
                    crate::compaction_prefire::PrefireConsume::Ready { key, result } => {
                        Some((key.range, result, key.fingerprint))
                    }
                    crate::compaction_prefire::PrefireConsume::Failed { failure, .. } => {
                        return Err(OptimizationError::CompactionFailed(format!(
                            "prefired compaction failed: {failure}"
                        )));
                    }
                    crate::compaction_prefire::PrefireConsume::Suppressed => {
                        return Err(OptimizationError::CompactionFailed(
                            "compaction is suppressed for the unchanged source fingerprint"
                                .to_string(),
                        ));
                    }
                    crate::compaction_prefire::PrefireConsume::Miss
                    | crate::compaction_prefire::PrefireConsume::Stale => None,
                }
            } else {
                None
            }
        } else {
            None
        };
        #[cfg(not(feature = "compaction_prefire"))]
        let prefired: Option<(
            std::ops::Range<usize>,
            y_context::CompactionResult,
            String,
        )> = None;

        let (compaction_range, result, compaction_fingerprint) = if let Some(prefired) = prefired {
            prefired
        } else {
            let effective_retain_window = transcript
                .len()
                .saturating_sub(default_compaction_range.end);
            let compactable_messages = &transcript[default_compaction_range.start..];

            // Structured serialization: preserves tool_calls and tool_call_id
            // pairing instead of flat "[Role] content" strings.
            let message_strings: Vec<String> = compactable_messages
                .iter()
                .map(y_context::compaction::serialize_for_compaction)
                .collect();

            if message_strings.is_empty() {
                return Ok(());
            }

            let previous_summary = find_previous_compaction_summary(&transcript);

            let result = container
                .compaction_engine
                .compact_async_with_retain_and_previous(
                    &message_strings,
                    effective_retain_window,
                    previous_summary.as_deref(),
                )
                .await;
            (default_compaction_range, result, default_fingerprint)
        };

        if let y_context::CompactionOutcome::Fallback { failure } = &result.outcome {
            tracing::warn!(
                fingerprint = %compaction_fingerprint,
                "compaction returned a fallback outcome"
            );
            #[cfg(feature = "compaction_prefire")]
            if retain_window_override.is_none() {
                if let Some(failure) = failure {
                    container
                        .compaction_prefire_registry
                        .record_failure(
                            session_id.clone(),
                            crate::compaction_prefire::PrefireKey {
                                fingerprint: compaction_fingerprint,
                                range: compaction_range.clone(),
                            },
                            failure.class,
                        )
                        .await;
                }
            }
            let message = failure.as_ref().map_or_else(
                || "compaction produced a fallback result".to_string(),
                ToString::to_string,
            );
            return Err(OptimizationError::CompactionFailed(message));
        }

        if result.messages_compacted != compaction_range.len() {
            return Err(OptimizationError::CompactionFailed(format!(
                "compaction range mismatch: expected {}, got {}",
                compaction_range.len(),
                result.messages_compacted
            )));
        }

        if result.messages_compacted > 0 {
            report.compaction_triggered = true;
            report.messages_compacted = result.messages_compacted;
            report.compaction_tokens_saved = result.tokens_saved;
            report.compaction_summary.clone_from(&result.summary);

            // Keep the messages that were NOT compacted (the recent ones).
            let preserved_prefix = &transcript[..compaction_range.start];
            let retained = &transcript[compaction_range.end..];
            let retained_count = retained.len();
            let mut rewritten = Vec::with_capacity(
                transcript
                    .len()
                    .saturating_sub(compaction_range.len())
                    .saturating_add(1),
            );
            rewritten.extend_from_slice(preserved_prefix);

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
                rewritten.push(summary_msg);
            }

            rewritten.extend_from_slice(retained);
            replace_context_transcript(container, session_id, &rewritten).await?;

            #[cfg(feature = "compaction_prefire")]
            container
                .compaction_prefire_registry
                .clear(session_id)
                .await;

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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::config::ServiceConfig;
    use async_trait::async_trait;
    use y_context::{CompactionConfig, CompactionEngine, CompactionLlm, CompactionLlmError};
    use y_core::provider::{ChatRequest, RequestMode, ToolCallingMode, ToolDialect};
    use y_core::session::{CreateSessionOptions, SessionType};
    use y_core::types::{Message, Role, ToolCallRequest};

    struct SuccessfulCompactionLlm;

    #[async_trait]
    impl CompactionLlm for SuccessfulCompactionLlm {
        async fn summarize(&self, _prompt: &str) -> Result<String, CompactionLlmError> {
            Ok("safe summary".to_string())
        }
    }

    struct CountingCompactionLlm {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CompactionLlm for CountingCompactionLlm {
        async fn summarize(&self, _prompt: &str) -> Result<String, CompactionLlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("prefired summary".to_string())
        }
    }

    struct DeterministicFailureCompactionLlm {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CompactionLlm for DeterministicFailureCompactionLlm {
        async fn summarize(&self, _prompt: &str) -> Result<String, CompactionLlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(CompactionLlmError::deterministic(
                "compaction model is unavailable",
            ))
        }
    }

    async fn make_test_container() -> (ServiceContainer, tempfile::TempDir) {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config = ServiceConfig {
            storage: y_storage::StorageConfig {
                db_path: ":memory:".to_string(),
                pool_size: 1,
                wal_enabled: false,
                transcript_dir: temp.path().join("transcripts"),
                ..y_storage::StorageConfig::default()
            },
            pruning: y_context::pruning::PruningConfig {
                enabled: false,
                ..y_context::pruning::PruningConfig::default()
            },
            ..ServiceConfig::default()
        };
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, temp)
    }

    fn message(role: Role, content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

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

    #[tokio::test]
    async fn failed_compaction_does_not_rewrite_transcript_with_placeholder() {
        let (container, _temp) = make_test_container().await;
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .expect("session");
        for (role, content) in [
            (Role::User, "request one"),
            (Role::Assistant, "response one"),
            (Role::User, "request two"),
            (Role::Assistant, "response two"),
        ] {
            container
                .session_manager
                .append_message(&session.id, &message(role, content))
                .await
                .expect("append message");
        }
        let before = container
            .session_manager
            .read_transcript(&session.id)
            .await
            .expect("read before");

        let result = ContextOptimizationService::compact_now(&container, &session.id).await;
        let after = container
            .session_manager
            .read_transcript(&session.id)
            .await
            .expect("read after");

        assert!(matches!(
            result,
            Err(OptimizationError::CompactionFailed(_))
        ));
        assert_eq!(after, before);
    }

    #[cfg(feature = "compaction_prefire")]
    #[tokio::test]
    async fn automatic_compaction_suppresses_repeated_deterministic_failure() {
        let (mut container, _temp) = make_test_container().await;
        let calls = Arc::new(AtomicUsize::new(0));
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig {
                max_retries: 1,
                retain_window: 2,
                ..CompactionConfig::default()
            },
            Box::new(DeterministicFailureCompactionLlm {
                calls: Arc::clone(&calls),
            }),
        );
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .expect("session");
        for (role, content) in [
            (Role::User, "old request one"),
            (Role::Assistant, "old response one"),
            (Role::User, "old request two"),
            (Role::Assistant, "old response two"),
        ] {
            container
                .session_manager
                .append_message(&session.id, &message(role, content))
                .await
                .expect("append message");
        }

        let first =
            ContextOptimizationService::optimize_post_turn(&container, &session.id, 1).await;
        container
            .session_manager
            .append_message(&session.id, &message(Role::User, "new recent request"))
            .await
            .expect("append recent message");
        let second =
            ContextOptimizationService::optimize_post_turn(&container, &session.id, 1).await;

        assert!(matches!(first, Err(OptimizationError::CompactionFailed(_))));
        assert!(matches!(
            second,
            Err(OptimizationError::CompactionFailed(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transcript_compaction_keeps_tool_call_and_result_together() {
        let (mut container, _temp) = make_test_container().await;
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(SuccessfulCompactionLlm),
        );
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .expect("session");
        let mut assistant = message(Role::Assistant, "reading");
        assistant.tool_calls.push(ToolCallRequest {
            id: "call-1".to_string(),
            name: "FileRead".to_string(),
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        });
        let mut tool_result = message(Role::Tool, "file contents");
        tool_result.tool_call_id = Some("call-1".to_string());
        for item in [
            message(Role::User, "old request"),
            assistant,
            tool_result,
            message(Role::User, "recent request"),
        ] {
            container
                .session_manager
                .append_message(&session.id, &item)
                .await
                .expect("append message");
        }

        ContextOptimizationService::compact_now(&container, &session.id)
            .await
            .expect("compaction");
        let transcript = container
            .session_manager
            .read_transcript(&session.id)
            .await
            .expect("read transcript");

        assert_eq!(transcript[1].role, Role::Assistant);
        assert_eq!(transcript[2].role, Role::Tool);
        assert_eq!(transcript[2].tool_call_id.as_deref(), Some("call-1"));
    }

    #[cfg(feature = "compaction_prefire")]
    #[tokio::test]
    async fn prefire_threshold_schedules_without_rewriting_transcript() {
        let (mut container, _temp) = make_test_container().await;
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(SuccessfulCompactionLlm),
        );
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .expect("session");
        for index in 0..12 {
            container
                .session_manager
                .append_message(
                    &session.id,
                    &message(
                        Role::User,
                        &format!("message-{index:02}-{}", "x".repeat(17)),
                    ),
                )
                .await
                .expect("append message");
        }
        let before = container
            .session_manager
            .read_transcript(&session.id)
            .await
            .expect("read before");

        let report = ContextOptimizationService::optimize_post_turn(&container, &session.id, 100)
            .await
            .expect("post-turn optimization");
        let after = container
            .session_manager
            .read_transcript(&session.id)
            .await
            .expect("read after");

        assert!(!report.compaction_triggered);
        assert_eq!(after, before);
        assert!(container
            .compaction_prefire_registry
            .pending_key(&session.id)
            .await
            .is_some());
    }

    #[cfg(feature = "compaction_prefire")]
    #[tokio::test]
    async fn hard_threshold_reuses_matching_prefire_result() {
        let (mut container, _temp) = make_test_container().await;
        let calls = Arc::new(AtomicUsize::new(0));
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(CountingCompactionLlm {
                calls: Arc::clone(&calls),
            }),
        );
        container.handoff_generator = None;
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .expect("session");
        for index in 0..12 {
            container
                .session_manager
                .append_message(
                    &session.id,
                    &message(
                        Role::User,
                        &format!("message-{index:02}-{}", "x".repeat(17)),
                    ),
                )
                .await
                .expect("append message");
        }
        ContextOptimizationService::optimize_post_turn(&container, &session.id, 100)
            .await
            .expect("prefire");
        container
            .session_manager
            .append_message(
                &session.id,
                &message(Role::User, &format!("new-message-{}", "x".repeat(20))),
            )
            .await
            .expect("append recent message");

        let report = ContextOptimizationService::optimize_post_turn(&container, &session.id, 100)
            .await
            .expect("hard compaction");

        assert!(report.compaction_triggered);
        assert_eq!(report.compaction_summary, "prefired summary");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sampling_preflight_compacts_in_memory_before_provider_call() {
        let (mut container, _temp) = make_test_container().await;
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(SuccessfulCompactionLlm),
        );
        let session_id = SessionId("sampling-session".to_string());
        let mut history = (0..12)
            .map(|index| message(Role::User, &format!("message-{index}-{}", "x".repeat(20))))
            .collect::<Vec<_>>();
        let request = ChatRequest {
            messages: history.clone(),
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: Some(0),
            temperature: None,
            top_p: None,
            tools: Vec::new(),
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::default(),
            stop: Vec::new(),
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let outcome = ContextOptimizationService::optimize_working_history_before_sampling(
            &container,
            &session_id,
            &mut history,
            0,
            &request,
            100,
            false,
        )
        .await
        .expect("preflight compaction");

        assert!(matches!(outcome, WorkingHistoryOptimization::Applied));
        assert!(history.len() < 12);
        assert_eq!(history[0].metadata["type"], "compaction_summary");
    }
}
