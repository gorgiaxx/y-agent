//! `LoadHistory` pipeline stage (priority 600).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Loads session messages, applies `repair_history()` to fix
//! inconsistencies, and injects them as `ContextCategory::History` items.

use async_trait::async_trait;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};
use crate::repair::{repair_history, HistoryMessage, RepairReport};

/// Default maximum tokens for history context.
const DEFAULT_HISTORY_BUDGET: u32 = 80_000;

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// `LoadHistory` — loads and repairs session history for the context pipeline.
///
/// Runs at priority 600 (`LOAD_HISTORY`).
///
/// The provider accepts pre-loaded messages (as `HistoryMessage`). In a
/// full integration, the caller loads messages from `SessionManager` or
/// `TranscriptStore` before constructing this provider.
pub struct LoadHistory {
    /// Pre-loaded session messages.
    messages: Vec<HistoryMessage>,
    /// Token budget for history.
    budget: u32,
}

impl LoadHistory {
    /// Create a new `LoadHistory` provider with pre-loaded messages.
    pub fn new(messages: Vec<HistoryMessage>) -> Self {
        Self {
            messages,
            budget: DEFAULT_HISTORY_BUDGET,
        }
    }

    /// Create with a custom token budget.
    pub fn with_budget(messages: Vec<HistoryMessage>, budget: u32) -> Self {
        Self { messages, budget }
    }

    /// Get the repair report without running the full pipeline.
    ///
    /// Useful for diagnostics.
    pub fn repair_report(&self) -> RepairReport {
        let (_, report) = repair_history(&self.messages);
        report
    }
}

#[async_trait]
impl ContextProvider for LoadHistory {
    fn name(&self) -> &'static str {
        "load_history"
    }

    fn priority(&self) -> u32 {
        600
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        if self.messages.is_empty() {
            return Ok(());
        }

        // Apply repair to fix inconsistencies.
        let (repaired, report) = repair_history(&self.messages);

        if report.total_fixes() > 0 {
            tracing::info!(
                empty_removed = report.empty_removed,
                orphans_removed = report.orphans_removed,
                duplicates_removed = report.duplicates_removed,
                merged = report.merged,
                "history repaired"
            );
        }

        // Format repaired messages as context items, respecting budget.
        let mut remaining = self.budget;

        for msg in &repaired {
            if remaining == 0 {
                break;
            }

            let formatted = format!("[{}] {}", msg.role, msg.content);
            let tokens = estimate_tokens(&formatted);

            if tokens > remaining {
                // Truncate last message to fit.
                let max_chars = (remaining as usize) * 4;
                let truncated = if formatted.len() > max_chars {
                    format!("{}... [truncated]", &formatted[..max_chars])
                } else {
                    formatted
                };
                let truncated_tokens = estimate_tokens(&truncated);

                ctx.add(ContextItem {
                    category: ContextCategory::History,
                    content: truncated,
                    token_estimate: truncated_tokens,
                    priority: 600,
                });
                remaining = 0;
            } else {
                ctx.add(ContextItem {
                    category: ContextCategory::History,
                    content: formatted,
                    token_estimate: tokens,
                    priority: 600,
                });
                remaining = remaining.saturating_sub(tokens);
            }
        }

        tracing::debug!(
            original = self.messages.len(),
            repaired = repaired.len(),
            budget = self.budget,
            remaining,
            "history context loaded"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: &str, content: &str) -> HistoryMessage {
        HistoryMessage {
            id: id.into(),
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
        }
    }

    /// T-P1-08: Provider name and priority; loads and repairs history.
    #[tokio::test]
    async fn test_provider_name_priority_and_load() {
        let provider = LoadHistory::new(vec![
            msg("1", "user", "Hello"),
            msg("2", "assistant", "Hi there!"),
        ]);

        assert_eq!(provider.name(), "load_history");
        assert_eq!(provider.priority(), 600);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 2);
        assert!(ctx
            .items
            .iter()
            .all(|i| i.category == ContextCategory::History));
        assert!(ctx.items[0].content.contains("[user]"));
        assert!(ctx.items[1].content.contains("[assistant]"));
    }

    /// T-P1-09: Applies repair to orphan tool results.
    #[tokio::test]
    async fn test_repairs_history() {
        let messages = vec![
            msg("1", "system", "You are an AI."),
            msg("2", "system", "Duplicate system."), // Will be removed
            msg("3", "user", "Hello"),
            msg("4", "assistant", ""), // Empty, will be removed
            msg("5", "assistant", "Hi"),
        ];

        let provider = LoadHistory::new(messages);

        // Check repair report.
        let report = provider.repair_report();
        assert!(report.total_fixes() > 0);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // After repair: 1 system + 1 user + 1 assistant = 3
        assert_eq!(ctx.items.len(), 3);
    }

    /// Empty history produces no items.
    #[tokio::test]
    async fn test_empty_history() {
        let provider = LoadHistory::new(vec![]);
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    /// History respects token budget.
    #[tokio::test]
    async fn test_respects_budget() {
        let messages: Vec<HistoryMessage> = (0..100)
            .map(|i| {
                msg(
                    &format!("{i}"),
                    "user",
                    &format!("Message number {i} with some content that takes up tokens"),
                )
            })
            .collect();

        let provider = LoadHistory::with_budget(messages, 50); // 50 token budget

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let total: u32 = ctx.items.iter().map(|i| i.token_estimate).sum();
        assert!(total <= 60); // Allow slight overhead from truncation
    }
}
