//! Input enrichment: context provider that enhances user input.
//!
//! Design reference: input-enrichment-design.md
//!
//! The input enrichment sub-agent analyzes user queries to determine
//! if additional context or clarification would improve the response.
//! Three strategies are available:
//! - **Clarification**: Ask the user for missing information
//! - **`OptionList`**: Present structured choices
//! - **`AutoExpand`**: Automatically enrich the input with context

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Strategy for enriching user input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentStrategy {
    /// Ask the user for missing information.
    Clarification,
    /// Present structured option choices.
    OptionList,
    /// Automatically expand the input with context.
    AutoExpand,
}

/// Result of an enrichment attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentResult {
    /// The enriched input text.
    pub enriched_input: String,
    /// Which strategy was used.
    pub strategy_used: EnrichmentStrategy,
    /// Confidence in the enrichment (0.0–1.0).
    pub confidence: f64,
    /// Whether the original input is preserved (or replaced).
    pub original_preserved: bool,
}

/// Configuration for input enrichment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentConfig {
    /// Whether enrichment is enabled.
    pub enabled: bool,
    /// Maximum enrichment rounds.
    pub max_rounds: u32,
    /// Minimum confidence to apply enrichment.
    pub confidence_threshold: f64,
    /// Allowed strategies.
    pub strategies: Vec<EnrichmentStrategy>,
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_rounds: 1,
            confidence_threshold: 0.7,
            strategies: vec![EnrichmentStrategy::AutoExpand],
        }
    }
}

// ---------------------------------------------------------------------------
// Enrichment provider
// ---------------------------------------------------------------------------

/// Input enrichment context provider.
///
/// Implements `ContextProvider` to inject enriched input into the
/// context assembly pipeline. When enrichment confidence exceeds the
/// threshold, the enriched input *replaces* the original to save tokens.
pub struct InputEnrichmentProvider {
    config: EnrichmentConfig,
    /// The raw user input to analyze.
    raw_input: String,
}

impl InputEnrichmentProvider {
    /// Create a new enrichment provider.
    pub fn new(config: EnrichmentConfig, raw_input: String) -> Self {
        Self { config, raw_input }
    }

    /// Analyze the input and determine enrichment.
    ///
    /// In production, this would invoke the enrichment sub-agent (LLM call).
    /// This stub applies simple heuristic enrichment.
    pub fn analyze(&self) -> Option<EnrichmentResult> {
        if !self.config.enabled {
            return None;
        }

        // Heuristic: short inputs (< 20 chars) benefit from auto-expand.
        if self.raw_input.len() < 20
            && self
                .config
                .strategies
                .contains(&EnrichmentStrategy::AutoExpand)
        {
            let enriched = format!(
                "{} (please provide a detailed and comprehensive response)",
                self.raw_input
            );
            return Some(EnrichmentResult {
                enriched_input: enriched,
                strategy_used: EnrichmentStrategy::AutoExpand,
                confidence: 0.8,
                original_preserved: false,
            });
        }

        None
    }
}

#[async_trait]
impl ContextProvider for InputEnrichmentProvider {
    fn name(&self) -> &'static str {
        "input_enrichment"
    }

    fn priority(&self) -> u32 {
        // Run before most other providers (low priority number = runs first).
        50
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        if let Some(result) = self.analyze() {
            if result.confidence >= self.config.confidence_threshold {
                ctx.add(ContextItem {
                    category: ContextCategory::SystemPrompt,
                    content: result.enriched_input,
                    token_estimate: 20,
                    priority: 100,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P3-35-01: Enrichment provider implements `ContextProvider`.
    #[tokio::test]
    async fn test_enrichment_provides_context() {
        let provider =
            InputEnrichmentProvider::new(EnrichmentConfig::default(), "fix bug".to_string());

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // Short input should be auto-expanded.
        assert_eq!(ctx.items.len(), 1);
        assert!(ctx.items[0].content.contains("fix bug"));
        assert!(ctx.items[0].content.contains("comprehensive"));
    }

    /// T-P3-35-02: Long input skips enrichment.
    #[tokio::test]
    async fn test_enrichment_skips_long_input() {
        let provider = InputEnrichmentProvider::new(
            EnrichmentConfig::default(),
            "Please fix the null pointer exception in the user authentication module".to_string(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // Long input does not trigger enrichment.
        assert!(ctx.items.is_empty());
    }

    /// T-P3-35-03: Disabled enrichment produces nothing.
    #[tokio::test]
    async fn test_enrichment_disabled() {
        let config = EnrichmentConfig {
            enabled: false,
            ..Default::default()
        };
        let provider = InputEnrichmentProvider::new(config, "fix bug".to_string());

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    /// T-P3-35-04: Confidence below threshold skips enrichment.
    #[tokio::test]
    async fn test_enrichment_below_threshold() {
        let config = EnrichmentConfig {
            confidence_threshold: 0.99, // Very high threshold
            ..Default::default()
        };
        let provider = InputEnrichmentProvider::new(config, "fix bug".to_string());

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        // 0.8 < 0.99 → skipped
        assert!(ctx.items.is_empty());
    }
}
