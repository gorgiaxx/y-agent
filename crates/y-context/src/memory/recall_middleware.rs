//! `RecallMiddleware`: injects recalled memories into context pipeline.
//!
//! Operates at priority 300 in the context middleware chain.
//! Queries LTM for relevant memories based on the current prompt
//! and injects them as context items within the token budget.

use crate::pipeline::{ContextCategory, ContextItem};

/// Recalled memory formatted for context injection.
#[derive(Debug, Clone)]
pub struct RecalledItem {
    pub content: String,
    pub relevance: f32,
    pub memory_type: String,
}

/// Configuration for memory recall in context.
#[derive(Debug, Clone)]
pub struct RecallMiddlewareConfig {
    /// Maximum tokens to allocate for recalled memories.
    pub max_recall_tokens: u32,
    /// Minimum relevance score to include a memory.
    pub min_relevance: f32,
    /// Maximum number of memories to recall.
    pub max_memories: usize,
}

impl Default for RecallMiddlewareConfig {
    fn default() -> Self {
        Self {
            max_recall_tokens: 1000,
            min_relevance: 0.3,
            max_memories: 5,
        }
    }
}

/// Middleware that injects recalled memories into the context pipeline.
#[derive(Debug)]
pub struct RecallMiddleware {
    config: RecallMiddlewareConfig,
}

impl RecallMiddleware {
    pub fn new(config: RecallMiddlewareConfig) -> Self {
        Self { config }
    }

    /// Format recalled items as context items, respecting token budget.
    pub fn format_items(&self, items: &[RecalledItem]) -> Vec<ContextItem> {
        let mut budget = self.config.max_recall_tokens;
        let mut context_items = Vec::new();

        for item in items
            .iter()
            .filter(|i| i.relevance >= self.config.min_relevance)
            .take(self.config.max_memories)
        {
            let token_estimate = u32::try_from(item.content.len())
                .unwrap_or(u32::MAX)
                .div_ceil(4);
            if token_estimate > budget {
                break;
            }

            context_items.push(ContextItem {
                category: ContextCategory::Memory,
                content: format!("[{}] {}", item.memory_type, item.content),
                token_estimate,
                priority: 300,
            });

            budget = budget.saturating_sub(token_estimate);
        }

        context_items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_items() -> Vec<RecalledItem> {
        vec![
            RecalledItem {
                content: "User prefers thiserror for library errors".to_string(),
                relevance: 0.9,
                memory_type: "personal".to_string(),
            },
            RecalledItem {
                content: "cargo test --lib runs only library tests".to_string(),
                relevance: 0.7,
                memory_type: "tool".to_string(),
            },
            RecalledItem {
                content: "Low relevance memory".to_string(),
                relevance: 0.1,
                memory_type: "task".to_string(),
            },
        ]
    }

    /// T-MEM-RECALL-01: High-relevance memories are injected.
    #[test]
    fn test_recall_injects_relevant() {
        let middleware = RecallMiddleware::new(RecallMiddlewareConfig::default());
        let items = middleware.format_items(&test_items());

        // 2 items above min_relevance (0.3)
        assert_eq!(items.len(), 2);
        assert!(items[0].content.contains("thiserror"));
    }

    /// T-MEM-RECALL-02: Low-relevance memories are filtered.
    #[test]
    fn test_recall_filters_low_relevance() {
        let middleware = RecallMiddleware::new(RecallMiddlewareConfig::default());
        let items = middleware.format_items(&test_items());

        // "Low relevance memory" (0.1) should be excluded
        assert!(items.iter().all(|i| !i.content.contains("Low relevance")));
    }

    /// T-MEM-RECALL-03: Token budget is respected.
    #[test]
    fn test_recall_respects_budget() {
        let config = RecallMiddlewareConfig {
            max_recall_tokens: 20, // Very small
            min_relevance: 0.0,
            max_memories: 10,
        };
        let middleware = RecallMiddleware::new(config);

        let items = middleware.format_items(&test_items());
        let total: u32 = items.iter().map(|i| i.token_estimate).sum();
        assert!(total <= 20, "total {total} exceeds budget 20");
    }
}
