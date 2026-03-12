//! `InjectMemory` pipeline stage (priority 300).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Uses `RecallMiddleware` to format recalled memories and injects them
//! as `ContextCategory::Memory` items in the context pipeline.

use async_trait::async_trait;

use crate::memory::recall_middleware::{RecallMiddleware, RecalledItem};
use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Default maximum tokens for memory context.
const DEFAULT_MEMORY_BUDGET: u32 = 4_000;

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// `InjectMemory` — injects recalled memories into the context pipeline.
///
/// Runs at priority 300 (`INJECT_MEMORY`).
pub struct InjectMemory {
    /// Pre-recalled memory items (provided by the caller after search).
    recalled_items: Vec<RecalledItem>,
    /// Recall middleware for formatting and budget enforcement.
    middleware: RecallMiddleware,
    /// Token budget for memory context.
    budget: u32,
}

impl InjectMemory {
    /// Create a new `InjectMemory` provider with recalled items.
    pub fn new(recalled_items: Vec<RecalledItem>, middleware: RecallMiddleware) -> Self {
        Self {
            recalled_items,
            middleware,
            budget: DEFAULT_MEMORY_BUDGET,
        }
    }

    /// Create with a custom token budget.
    pub fn with_budget(
        recalled_items: Vec<RecalledItem>,
        middleware: RecallMiddleware,
        budget: u32,
    ) -> Self {
        Self {
            recalled_items,
            middleware,
            budget,
        }
    }
}

#[async_trait]
impl ContextProvider for InjectMemory {
    fn name(&self) -> &'static str {
        "inject_memory"
    }

    fn priority(&self) -> u32 {
        300
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        if self.recalled_items.is_empty() {
            tracing::debug!("no recalled memories to inject");
            return Ok(());
        }

        let formatted = self.middleware.format_items(&self.recalled_items);

        let mut remaining = self.budget;
        let mut count = 0;

        for item in formatted {
            if remaining == 0 {
                break;
            }
            if item.token_estimate > remaining {
                break;
            }
            remaining = remaining.saturating_sub(item.token_estimate);
            count += 1;
            ctx.add(item);
        }

        // If there were any recalled memories, add a header item.
        if count > 0 {
            let header = format!("[Recalled {count} relevant memories]");
            let header_tokens = estimate_tokens(&header);
            ctx.add(ContextItem {
                category: ContextCategory::Memory,
                content: header,
                token_estimate: header_tokens,
                priority: 300,
            });
        }

        tracing::debug!(
            recalled = self.recalled_items.len(),
            injected = count,
            budget = self.budget,
            remaining,
            "memory context injected"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::recall_middleware::RecallMiddlewareConfig;

    fn sample_items() -> Vec<RecalledItem> {
        vec![
            RecalledItem {
                content: "User prefers thiserror for error handling".to_string(),
                relevance: 0.9,
                memory_type: "personal".to_string(),
            },
            RecalledItem {
                content: "cargo test --lib runs library tests only".to_string(),
                relevance: 0.7,
                memory_type: "tool".to_string(),
            },
        ]
    }

    /// T-P1-03: Provider name and priority; injects recalled memories.
    #[tokio::test]
    async fn test_provider_name_priority_and_inject() {
        let middleware = RecallMiddleware::new(RecallMiddlewareConfig::default());
        let provider = InjectMemory::new(sample_items(), middleware);

        assert_eq!(provider.name(), "inject_memory");
        assert_eq!(provider.priority(), 300);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // Should have formatted items + a header.
        assert!(ctx.items.len() >= 2);
        assert!(ctx
            .items
            .iter()
            .any(|i| i.category == ContextCategory::Memory));
    }

    /// T-P1-04: Empty recall produces no items.
    #[tokio::test]
    async fn test_empty_recall_produces_no_items() {
        let middleware = RecallMiddleware::new(RecallMiddlewareConfig::default());
        let provider = InjectMemory::new(vec![], middleware);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert!(ctx.items.is_empty());
    }

    /// Memory provider respects budget.
    #[tokio::test]
    async fn test_respects_budget() {
        let middleware = RecallMiddleware::new(RecallMiddlewareConfig {
            max_recall_tokens: 10_000,
            min_relevance: 0.0,
            max_memories: 100,
        });
        let provider = InjectMemory::with_budget(sample_items(), middleware, 5);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // With a 5-token budget, very few or no items should fit.
        let total: u32 = ctx.items.iter().map(|i| i.token_estimate).sum();
        // Budget is very small so total should be limited.
        assert!(total <= 20); // Allow for header overhead
    }
}
