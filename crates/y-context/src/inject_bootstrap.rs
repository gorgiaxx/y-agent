//! `InjectBootstrap` pipeline stage (priority 200).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Reads workspace context files (README.md, AGENTS.md, project structure)
//! and injects them as `ContextCategory::Bootstrap` items. Respects the
//! bootstrap token budget.

use async_trait::async_trait;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Default maximum tokens for bootstrap context.
const DEFAULT_BOOTSTRAP_BUDGET: u32 = 8_000;

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// A bootstrap file entry to inject.
#[derive(Debug, Clone)]
pub struct BootstrapEntry {
    /// Filename or label (e.g., "README.md").
    pub label: String,
    /// File content.
    pub content: String,
}

/// `InjectBootstrap` — injects workspace context (README, AGENTS, etc.).
///
/// Runs at priority 200 (`INJECT_BOOTSTRAP`).
pub struct InjectBootstrap {
    /// Pre-loaded bootstrap files.
    entries: Vec<BootstrapEntry>,
    /// Maximum tokens to allocate for bootstrap context.
    budget: u32,
}

impl InjectBootstrap {
    /// Create a new `InjectBootstrap` provider.
    pub fn new(entries: Vec<BootstrapEntry>) -> Self {
        Self {
            entries,
            budget: DEFAULT_BOOTSTRAP_BUDGET,
        }
    }

    /// Create with a custom token budget.
    pub fn with_budget(entries: Vec<BootstrapEntry>, budget: u32) -> Self {
        Self { entries, budget }
    }
}

#[async_trait]
impl ContextProvider for InjectBootstrap {
    fn name(&self) -> &'static str {
        "inject_bootstrap"
    }

    fn priority(&self) -> u32 {
        200
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        let mut remaining = self.budget;

        for entry in &self.entries {
            if remaining == 0 {
                break;
            }

            let formatted = format!("## {}\n\n{}", entry.label, entry.content);
            let tokens = estimate_tokens(&formatted);

            if tokens > remaining {
                // Truncate to fit within budget.
                let max_chars = (remaining as usize) * 4;
                let truncated = if formatted.len() > max_chars {
                    format!("{}... [truncated]", &formatted[..max_chars])
                } else {
                    formatted
                };
                let truncated_tokens = estimate_tokens(&truncated);

                ctx.add(ContextItem {
                    category: ContextCategory::Bootstrap,
                    content: truncated,
                    token_estimate: truncated_tokens,
                    priority: 200,
                });
                remaining = remaining.saturating_sub(truncated_tokens);
            } else {
                ctx.add(ContextItem {
                    category: ContextCategory::Bootstrap,
                    content: formatted,
                    token_estimate: tokens,
                    priority: 200,
                });
                remaining = remaining.saturating_sub(tokens);
            }
        }

        tracing::debug!(
            entries = self.entries.len(),
            budget = self.budget,
            remaining,
            "bootstrap context injected"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P1-01: Provider name and priority are correct.
    #[test]
    fn test_provider_name_and_priority() {
        let provider = InjectBootstrap::new(vec![]);
        assert_eq!(provider.name(), "inject_bootstrap");
        assert_eq!(provider.priority(), 200);
    }

    /// T-P1-01: Provides bootstrap items from entries.
    #[tokio::test]
    async fn test_provides_bootstrap_items() {
        let provider = InjectBootstrap::new(vec![
            BootstrapEntry {
                label: "README.md".into(),
                content: "# My Project\nA cool project.".into(),
            },
            BootstrapEntry {
                label: "AGENTS.md".into(),
                content: "Agent rules here.".into(),
            },
        ]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 2);
        assert_eq!(ctx.items[0].category, ContextCategory::Bootstrap);
        assert!(ctx.items[0].content.contains("README.md"));
        assert!(ctx.items[0].content.contains("My Project"));
        assert!(ctx.items[1].content.contains("AGENTS.md"));
    }

    /// T-P1-02: Respects token budget; truncates long README.
    #[tokio::test]
    async fn test_respects_token_budget() {
        let long_content = "x".repeat(100_000); // ~25,000 tokens
        let provider = InjectBootstrap::with_budget(
            vec![BootstrapEntry {
                label: "README.md".into(),
                content: long_content,
            }],
            100, // Only 100 tokens = ~400 chars
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        assert!(ctx.items[0].content.contains("[truncated]"));
        // Should be within budget.
        assert!(ctx.items[0].token_estimate <= 110); // Some overhead for truncation marker
    }

    /// T-P1-02: Empty entries produce no items.
    #[tokio::test]
    async fn test_empty_entries() {
        let provider = InjectBootstrap::new(vec![]);
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }
}
