//! `InjectContextStatus` pipeline stage.
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! This stage runs last (priority 700) and injects a status report about
//! the current context budget — how many tokens are used, remaining
//! capacity, and which categories consumed the most space.

use async_trait::async_trait;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// `InjectContextStatus` — adds a summary of context space usage.
///
/// This runs at priority 700 (last stage) to report on the final state
/// of the assembled context after all previous stages have contributed.
pub struct InjectContextStatus {
    /// Total token budget for the context window.
    context_budget: u32,
}

impl InjectContextStatus {
    /// Create a new `InjectContextStatus` stage.
    pub fn new(context_budget: u32) -> Self {
        Self { context_budget }
    }
}

#[async_trait]
impl ContextProvider for InjectContextStatus {
    fn name(&self) -> &'static str {
        "inject_context_status"
    }

    fn priority(&self) -> u32 {
        700 // Last stage — after all content has been assembled.
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        let total_used = ctx.total_tokens();
        let remaining = self.context_budget.saturating_sub(total_used);
        let usage_pct = if self.context_budget > 0 {
            (f64::from(total_used) / f64::from(self.context_budget)) * 100.0
        } else {
            0.0
        };

        // Build a concise status report.
        let mut status = format!(
            "[Context Status] {total_used}/{} tokens used ({usage_pct:.0}%)",
            self.context_budget
        );

        // Category breakdown.
        let categories = [
            ("System", ContextCategory::SystemPrompt),
            ("Bootstrap", ContextCategory::Bootstrap),
            ("Memory", ContextCategory::Memory),
            ("Skills", ContextCategory::Skills),
            ("Tools", ContextCategory::Tools),
            ("History", ContextCategory::History),
            ("Status", ContextCategory::Status),
        ];

        let mut breakdown_parts = Vec::new();
        for (label, cat) in &categories {
            let tokens = ctx.tokens_for(*cat);
            if tokens > 0 {
                breakdown_parts.push(format!("{label}={tokens}"));
            }
        }
        if !breakdown_parts.is_empty() {
            status.push_str(&format!(" | {}", breakdown_parts.join(", ")));
        }

        if usage_pct > 90.0 {
            status.push_str(" ⚠ context nearly full, consider compaction");
        }

        // Estimate the status message itself at ~20 tokens.
        let status_tokens = 20;

        ctx.add(ContextItem {
            category: ContextCategory::Status,
            content: status,
            token_estimate: status_tokens,
            priority: 700,
        });

        tracing::debug!(
            total_used = total_used,
            remaining = remaining,
            budget = self.context_budget,
            "context status injected"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{AssembledContext, ContextCategory, ContextItem};

    #[tokio::test]
    async fn test_context_status_basic() {
        let mut ctx = AssembledContext::default();
        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "system".into(),
            token_estimate: 100,
            priority: 100,
        });
        ctx.add(ContextItem {
            category: ContextCategory::History,
            content: "history".into(),
            token_estimate: 500,
            priority: 600,
        });

        let stage = InjectContextStatus::new(4096);
        stage.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 3); // 2 existing + 1 status.
        let status_item = ctx.items.last().unwrap();
        assert_eq!(status_item.category, ContextCategory::Status);
        assert!(status_item.content.contains("600/4096"));
        assert!(status_item.content.contains("System=100"));
        assert!(status_item.content.contains("History=500"));
    }

    #[tokio::test]
    async fn test_context_status_nearly_full_warning() {
        let mut ctx = AssembledContext::default();
        ctx.add(ContextItem {
            category: ContextCategory::History,
            content: "lots of history".into(),
            token_estimate: 950,
            priority: 600,
        });

        let stage = InjectContextStatus::new(1000);
        stage.provide(&mut ctx).await.unwrap();

        let status_item = ctx.items.last().unwrap();
        assert!(status_item.content.contains("⚠ context nearly full"));
    }

    #[tokio::test]
    async fn test_context_status_empty_context() {
        let mut ctx = AssembledContext::default();

        let stage = InjectContextStatus::new(4096);
        stage.provide(&mut ctx).await.unwrap();

        let status_item = ctx.items.last().unwrap();
        assert!(status_item.content.contains("0/4096"));
    }

    #[test]
    fn test_stage_priority_is_700() {
        let stage = InjectContextStatus::new(4096);
        assert_eq!(stage.priority(), 700);
        assert_eq!(stage.name(), "inject_context_status");
    }
}
