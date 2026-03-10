//! Compaction engine: summarizes older messages to reclaim context window space.

use serde::{Deserialize, Serialize};

/// Compaction strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    /// Single LLM call summarizes all old messages.
    #[default]
    Summarize,
    /// Divide into segments, summarize each independently.
    SegmentedSummarize,
    /// Score by importance; retain high-scoring, summarize rest.
    SelectiveRetain,
}

/// Identifier preservation policy during compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierPolicy {
    /// All identifiers must appear verbatim in summary.
    #[default]
    Strict,
    /// Identifiers may be paraphrased.
    Relaxed,
    /// Custom regex patterns specify which to preserve.
    Custom { patterns: Vec<String> },
}

/// Compaction configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Strategy to use.
    #[serde(default)]
    pub strategy: CompactionStrategy,
    /// Identifier preservation policy.
    #[serde(default)]
    pub identifier_policy: IdentifierPolicy,
    /// Number of recent messages to retain (not compacted).
    #[serde(default = "default_retain_window")]
    pub retain_window: usize,
    /// Model to use for compaction LLM calls.
    #[serde(default = "default_compaction_model")]
    pub model: String,
}

fn default_retain_window() -> usize {
    10
}

fn default_compaction_model() -> String {
    "gpt-4o-mini".into()
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::default(),
            identifier_policy: IdentifierPolicy::default(),
            retain_window: default_retain_window(),
            model: default_compaction_model(),
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Summary replacing older messages.
    pub summary: String,
    /// Number of messages compacted.
    pub messages_compacted: usize,
    /// Estimated tokens saved.
    pub tokens_saved: u32,
    /// Estimated tokens in the summary.
    pub summary_tokens: u32,
}

/// Compaction engine (placeholder — actual LLM calls deferred to Phase 5).
pub struct CompactionEngine {
    pub config: CompactionConfig,
}

impl CompactionEngine {
    /// Create with default configuration.
    pub fn new() -> Self {
        Self {
            config: CompactionConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Compact a list of messages.
    ///
    /// In the placeholder implementation, creates a simple summary.
    /// Full LLM-based compaction is deferred to Phase 5.
    pub fn compact(&self, messages: &[String]) -> CompactionResult {
        if messages.len() <= self.config.retain_window {
            return CompactionResult {
                summary: String::new(),
                messages_compacted: 0,
                tokens_saved: 0,
                summary_tokens: 0,
            };
        }

        let to_compact = messages.len() - self.config.retain_window;
        let compacted = &messages[..to_compact];

        // Placeholder: simple concatenation summary.
        let summary = format!(
            "[Compacted {to_compact} messages using {:?} strategy]",
            self.config.strategy
        );

        let original_tokens: u32 = compacted.iter().map(|m| estimate_tokens(m)).sum();
        let summary_tokens = estimate_tokens(&summary);

        CompactionResult {
            summary,
            messages_compacted: to_compact,
            tokens_saved: original_tokens.saturating_sub(summary_tokens),
            summary_tokens,
        }
    }
}

impl Default for CompactionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_below_retain_window() {
        let engine = CompactionEngine::new();
        let messages: Vec<String> = (0..5).map(|i| format!("msg {i}")).collect();
        let result = engine.compact(&messages);
        assert_eq!(result.messages_compacted, 0);
    }

    #[test]
    fn test_compact_above_retain_window() {
        let engine = CompactionEngine::new();
        let messages: Vec<String> = (0..20)
            .map(|i| format!("message {i} with some content"))
            .collect();
        let result = engine.compact(&messages);
        assert_eq!(result.messages_compacted, 10); // 20 - retain_window(10)
        assert!(result.tokens_saved > 0);
        assert!(result.summary.contains("Compacted"));
    }

    #[test]
    fn test_compact_custom_retain_window() {
        let mut config = CompactionConfig::default();
        config.retain_window = 5;
        let engine = CompactionEngine::with_config(config);
        let messages: Vec<String> = (0..10).map(|i| format!("msg {i}")).collect();
        let result = engine.compact(&messages);
        assert_eq!(result.messages_compacted, 5);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("1234"), 1);
        assert_eq!(estimate_tokens("12345"), 2);
    }
}
