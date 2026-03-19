//! `ContextManager` — facade orchestrating the full context preparation flow.
//!
//! Design reference: context-session-design.md §Full Context Preparation Flow
//!
//! The `ContextManager` owns a `ContextPipeline`, `ContextWindowGuard`, and
//! `CompactionEngine`, coordinating the `prepare_context` flow:
//!
//! 1. Build `ContextRequest` from session + user message
//! 2. Execute pipeline (all stages in priority order)
//! 3. Evaluate via guard
//! 4. If overflow → trigger compaction recovery
//! 5. Return `PreparedContext`

use crate::compaction::{CompactionEngine, CompactionResult};
use crate::guard::{ContextWindowGuard, GuardVerdict};
use crate::pipeline::{
    AssembledContext, ContextCategory, ContextPipeline, ContextPipelineError, ContextRequest,
};

/// Result of the full context preparation flow.
#[derive(Debug, Clone)]
pub struct PreparedContext {
    /// Fully assembled context items (system prompt + history + tools etc.).
    pub assembled: AssembledContext,
    /// Total tokens used.
    pub tokens_used: u32,
    /// Whether compaction was triggered during preparation.
    pub compacted: bool,
    /// Compaction result if compaction was triggered.
    pub compaction_result: Option<CompactionResult>,
    /// Guard verdict after final evaluation.
    pub verdict: GuardVerdict,
}

/// Errors from the context manager.
#[derive(Debug, thiserror::Error)]
pub enum ContextManagerError {
    #[error("pipeline error: {0}")]
    Pipeline(#[from] ContextPipelineError),

    #[error("context overflow: {tokens_over} tokens over budget after all recovery attempts")]
    UnrecoverableOverflow { tokens_over: u32 },
}

/// Facade that orchestrates the full context preparation flow.
///
/// Owns the pipeline, guard, and compaction engine. The caller registers
/// providers on the pipeline before calling `prepare_context`.
pub struct ContextManager {
    /// The context assembly pipeline.
    pub pipeline: ContextPipeline,
    /// The context window guard for budget evaluation.
    pub guard: ContextWindowGuard,
    /// The compaction engine for overflow recovery.
    pub compaction: CompactionEngine,
}

impl ContextManager {
    /// Create a new `ContextManager` with default components.
    pub fn new() -> Self {
        Self {
            pipeline: ContextPipeline::new(),
            guard: ContextWindowGuard::new(),
            compaction: CompactionEngine::new(),
        }
    }

    /// Create with custom components.
    pub fn with_components(
        pipeline: ContextPipeline,
        guard: ContextWindowGuard,
        compaction: CompactionEngine,
    ) -> Self {
        Self {
            pipeline,
            guard,
            compaction,
        }
    }

    /// Prepare context for an LLM call.
    ///
    /// Executes the full flow per the design:
    /// 1. Execute pipeline with `ContextRequest`
    /// 2. Evaluate guard verdict
    /// 3. If overflow, apply recovery actions
    /// 4. Return `PreparedContext`
    pub async fn prepare_context(
        &self,
        request: ContextRequest,
    ) -> Result<PreparedContext, ContextManagerError> {
        // Step 1: Execute the pipeline.
        let mut assembled = self.pipeline.assemble_with_request(Some(request)).await?;

        // Step 2: Evaluate the guard.
        let mut verdict = self.guard.evaluate(&assembled);
        let mut compacted = false;
        let mut compaction_result = None;

        // Step 3: Handle overflow with priority-ordered recovery.
        match &verdict {
            GuardVerdict::Overflow { .. } | GuardVerdict::Critical { .. } => {
                tracing::warn!(verdict = ?verdict, "context overflow detected, attempting recovery");

                // Recovery action 1: Compact history.
                let recovery = self.recover_by_compacting_history(&mut assembled);
                if recovery.is_some() {
                    compacted = true;
                    compaction_result = recovery;
                    verdict = self.guard.evaluate(&assembled);
                }

                // Recovery action 2: Trim bootstrap (if still overflowing).
                if matches!(
                    verdict,
                    GuardVerdict::Overflow { .. } | GuardVerdict::Critical { .. }
                ) {
                    self.recover_by_trimming_bootstrap(&mut assembled);
                    verdict = self.guard.evaluate(&assembled);
                }

                // Recovery action 3: Evict tools (if still overflowing).
                if matches!(
                    verdict,
                    GuardVerdict::Overflow { .. } | GuardVerdict::Critical { .. }
                ) {
                    self.recover_by_evicting_tools(&mut assembled);
                    verdict = self.guard.evaluate(&assembled);
                }

                // If still critical after all recovery, return error.
                if let GuardVerdict::Critical { tokens_over } = verdict {
                    return Err(ContextManagerError::UnrecoverableOverflow { tokens_over });
                }
            }
            _ => {}
        }

        let tokens_used = assembled.total_tokens();

        Ok(PreparedContext {
            assembled,
            tokens_used,
            compacted,
            compaction_result,
            verdict,
        })
    }

    /// Recovery action 1: Compact history items.
    fn recover_by_compacting_history(
        &self,
        assembled: &mut AssembledContext,
    ) -> Option<CompactionResult> {
        let history_messages: Vec<String> = assembled
            .items
            .iter()
            .filter(|i| i.category == ContextCategory::History)
            .map(|i| i.content.clone())
            .collect();

        if history_messages.is_empty() {
            return None;
        }

        let result = self.compaction.compact(&history_messages);

        if result.messages_compacted > 0 {
            // Remove the compacted history items and insert summary.
            assembled
                .items
                .retain(|i| i.category != ContextCategory::History);

            // Add the summary as a single history item.
            if !result.summary.is_empty() {
                assembled.add(crate::pipeline::ContextItem {
                    category: ContextCategory::History,
                    content: result.summary.clone(),
                    token_estimate: result.summary_tokens,
                    priority: 600,
                });
            }

            // Add the retained (non-compacted) messages back.
            let retain_start = history_messages.len().saturating_sub(
                history_messages
                    .len()
                    .saturating_sub(result.messages_compacted),
            );
            for msg in &history_messages[retain_start..] {
                let tokens = u32::try_from(msg.len().div_ceil(4)).unwrap_or(u32::MAX);
                assembled.add(crate::pipeline::ContextItem {
                    category: ContextCategory::History,
                    content: msg.clone(),
                    token_estimate: tokens,
                    priority: 600,
                });
            }

            tracing::info!(
                compacted = result.messages_compacted,
                tokens_saved = result.tokens_saved,
                "history compacted for overflow recovery"
            );

            Some(result)
        } else {
            None
        }
    }

    /// Recovery action 2: Trim bootstrap items.
    fn recover_by_trimming_bootstrap(&self, assembled: &mut AssembledContext) {
        let before = assembled.items.len();
        assembled
            .items
            .retain(|i| i.category != ContextCategory::Bootstrap);
        let removed = before - assembled.items.len();
        if removed > 0 {
            tracing::info!(removed, "bootstrap context trimmed for overflow recovery");
        }
    }

    /// Recovery action 3: Evict tool items.
    fn recover_by_evicting_tools(&self, assembled: &mut AssembledContext) {
        let before = assembled.items.len();
        assembled
            .items
            .retain(|i| i.category != ContextCategory::Tools);
        let removed = before - assembled.items.len();
        if removed > 0 {
            tracing::info!(removed, "tool context evicted for overflow recovery");
        }
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ContextItem, ContextProvider};
    use async_trait::async_trait;

    /// A test provider that fills a specific category with a given token count.
    struct FillProvider {
        name: &'static str,
        priority: u32,
        category: ContextCategory,
        tokens: u32,
    }

    #[async_trait]
    impl ContextProvider for FillProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
            ctx.add(ContextItem {
                category: self.category,
                content: format!("[{} content]", self.name),
                token_estimate: self.tokens,
                priority: self.priority,
            });
            Ok(())
        }
    }

    /// T-P2-01: Simple prepare_context returns valid PreparedContext.
    #[tokio::test]
    async fn test_prepare_context_simple() {
        let mut manager = ContextManager::new();
        manager.pipeline.register(Box::new(FillProvider {
            name: "system_prompt",
            priority: 100,
            category: ContextCategory::SystemPrompt,
            tokens: 500,
        }));
        manager.pipeline.register(Box::new(FillProvider {
            name: "history",
            priority: 600,
            category: ContextCategory::History,
            tokens: 1_000,
        }));

        let request = ContextRequest::default();
        let result = manager.prepare_context(request).await.unwrap();

        assert!(result.tokens_used > 0);
        assert!(!result.compacted);
        assert!(result.compaction_result.is_none());
        assert_eq!(result.verdict, GuardVerdict::Ok);
        assert!(!result.assembled.items.is_empty());
    }

    /// T-P2-02: Overflow triggers compaction.
    #[tokio::test]
    async fn test_overflow_triggers_compaction() {
        let mut manager = ContextManager::new();

        // Add enough history to trigger overflow (>85% of 112K = ~95K).
        // We'll add multiple history items to allow compaction.
        for i in 0..20 {
            manager.pipeline.register(Box::new(FillProvider {
                name: Box::leak(format!("history_{i}").into_boxed_str()),
                priority: 600,
                category: ContextCategory::History,
                tokens: 5_000, // 20 * 5000 = 100K total
            }));
        }

        let request = ContextRequest::default();
        let result = manager.prepare_context(request).await.unwrap();

        // Compaction should have been triggered.
        assert!(result.compacted);
        assert!(result.compaction_result.is_some());
    }

    /// T-P2-04: Failed provider doesn't abort pipeline.
    #[tokio::test]
    async fn test_failed_provider_doesnt_abort() {
        struct FailingProvider;

        #[async_trait]
        impl ContextProvider for FailingProvider {
            fn name(&self) -> &'static str {
                "failing"
            }
            fn priority(&self) -> u32 {
                200
            }
            async fn provide(
                &self,
                _ctx: &mut AssembledContext,
            ) -> Result<(), ContextPipelineError> {
                Err(ContextPipelineError::ProviderFailed {
                    name: "failing".into(),
                    message: "test failure".into(),
                })
            }
        }

        let mut manager = ContextManager::new();
        manager.pipeline.register(Box::new(FailingProvider));
        manager.pipeline.register(Box::new(FillProvider {
            name: "history",
            priority: 600,
            category: ContextCategory::History,
            tokens: 1_000,
        }));

        let request = ContextRequest::default();
        let result = manager.prepare_context(request).await.unwrap();

        // Should still have the history item despite the failure.
        assert_eq!(result.assembled.items.len(), 1);
        assert_eq!(result.tokens_used, 1_000);
    }

    /// T-P2-05: Recovery actions applied in priority order.
    #[tokio::test]
    async fn test_recovery_actions_priority_order() {
        let mut manager = ContextManager::new();

        // Fill all categories to trigger overflow.
        manager.pipeline.register(Box::new(FillProvider {
            name: "system",
            priority: 100,
            category: ContextCategory::SystemPrompt,
            tokens: 5_000,
        }));
        manager.pipeline.register(Box::new(FillProvider {
            name: "bootstrap",
            priority: 200,
            category: ContextCategory::Bootstrap,
            tokens: 5_000,
        }));
        manager.pipeline.register(Box::new(FillProvider {
            name: "tools",
            priority: 500,
            category: ContextCategory::Tools,
            tokens: 5_000,
        }));
        // Heavy history to push into overflow.
        for i in 0..20 {
            manager.pipeline.register(Box::new(FillProvider {
                name: Box::leak(format!("hist_{i}").into_boxed_str()),
                priority: 600,
                category: ContextCategory::History,
                tokens: 5_000,
            }));
        }

        let request = ContextRequest::default();
        let result = manager.prepare_context(request).await.unwrap();

        // Should have attempted recovery. System prompt should survive.
        assert!(result
            .assembled
            .items
            .iter()
            .any(|i| i.category == ContextCategory::SystemPrompt));
    }

    /// `ContextManager` default creates valid instance.
    #[test]
    fn test_default() {
        let manager = ContextManager::default();
        assert_eq!(manager.pipeline.provider_count(), 0);
    }
}
