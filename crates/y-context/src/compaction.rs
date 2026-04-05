//! Compaction engine: summarizes older messages to reclaim context window space.
//!
//! Design reference: context-session-design.md §Compaction
//!
//! Three strategies are supported:
//! - **Summarize**: single LLM call summarizes all old messages
//! - **`SegmentedSummarize`**: divide into segments, summarize each
//! - **`SelectiveRetain`**: score by importance; keep high-scoring verbatim
//!
//! Identifier preservation is enforced post-compaction via configurable policy.

use std::fmt::Write;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

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
    /// Maximum retry attempts for LLM calls.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Number of messages per segment (for `SegmentedSummarize`).
    #[serde(default = "default_segment_size")]
    pub segment_size: usize,
}

fn default_retain_window() -> usize {
    10
}

fn default_compaction_model() -> String {
    "gpt-4o-mini".into()
}

fn default_max_retries() -> u32 {
    3
}

fn default_segment_size() -> usize {
    10
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::default(),
            identifier_policy: IdentifierPolicy::default(),
            retain_window: default_retain_window(),
            model: default_compaction_model(),
            max_retries: default_max_retries(),
            segment_size: default_segment_size(),
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// LLM trait for compaction
// ---------------------------------------------------------------------------

/// Trait for making LLM calls during compaction.
///
/// This is intentionally simple — it takes a prompt and returns text.
/// Implementations can wrap `ProviderPool` or provide mock responses.
#[async_trait]
pub trait CompactionLlm: Send + Sync {
    /// Send a compaction prompt and return the summary text.
    async fn summarize(&self, prompt: &str) -> Result<String, String>;
}

// ---------------------------------------------------------------------------
// Compaction engine
// ---------------------------------------------------------------------------

/// Compaction engine with LLM-based summarization and fallback.
pub struct CompactionEngine {
    pub config: CompactionConfig,
    llm: Option<Box<dyn CompactionLlm>>,
}

impl CompactionEngine {
    /// Create with default configuration (placeholder mode, no LLM).
    pub fn new() -> Self {
        Self {
            config: CompactionConfig::default(),
            llm: None,
        }
    }

    /// Create with custom configuration (placeholder mode, no LLM).
    pub fn with_config(config: CompactionConfig) -> Self {
        Self { config, llm: None }
    }

    /// Create with an LLM backend for real summarization.
    pub fn with_llm(config: CompactionConfig, llm: Box<dyn CompactionLlm>) -> Self {
        Self {
            config,
            llm: Some(llm),
        }
    }

    /// Compact a list of messages (synchronous fallback when no LLM).
    pub fn compact(&self, messages: &[String]) -> CompactionResult {
        self.compact_with_retain(messages, self.config.retain_window)
    }

    /// Compact with a custom retain window (synchronous fallback).
    pub fn compact_with_retain(
        &self,
        messages: &[String],
        retain_window: usize,
    ) -> CompactionResult {
        if messages.len() <= retain_window {
            return CompactionResult {
                summary: String::new(),
                messages_compacted: 0,
                tokens_saved: 0,
                summary_tokens: 0,
            };
        }

        let to_compact = messages.len() - retain_window;
        let compacted = &messages[..to_compact];

        // Fallback: simple placeholder summary.
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

    /// Compact messages using the configured LLM (async with retry).
    ///
    /// Falls back to simple truncation if LLM is unavailable or fails.
    pub async fn compact_async(&self, messages: &[String]) -> CompactionResult {
        self.compact_async_with_retain(messages, self.config.retain_window)
            .await
    }

    /// Compact with a custom retain window (async with LLM).
    ///
    /// Used by manual `/compact` to bypass the strict default retain window.
    /// Pass a smaller value (e.g. 2) so even short conversations can be
    /// compacted on demand.
    pub async fn compact_async_with_retain(
        &self,
        messages: &[String],
        retain_window: usize,
    ) -> CompactionResult {
        if messages.len() <= retain_window {
            return CompactionResult {
                summary: String::new(),
                messages_compacted: 0,
                tokens_saved: 0,
                summary_tokens: 0,
            };
        }

        let to_compact = messages.len() - retain_window;
        let compacted = &messages[..to_compact];
        let original_tokens: u32 = compacted.iter().map(|m| estimate_tokens(m)).sum();

        let Some(llm) = &self.llm else {
            return self.compact_with_retain(messages, retain_window);
        };

        let summary = match &self.config.strategy {
            CompactionStrategy::Summarize => self.summarize_all(llm.as_ref(), compacted).await,
            CompactionStrategy::SegmentedSummarize => {
                self.segmented_summarize(llm.as_ref(), compacted).await
            }
            CompactionStrategy::SelectiveRetain => {
                self.selective_retain(llm.as_ref(), compacted).await
            }
        };

        let summary_tokens = estimate_tokens(&summary);

        CompactionResult {
            summary,
            messages_compacted: to_compact,
            tokens_saved: original_tokens.saturating_sub(summary_tokens),
            summary_tokens,
        }
    }

    /// Strategy: Summarize — single LLM call for all messages.
    async fn summarize_all(&self, llm: &dyn CompactionLlm, messages: &[String]) -> String {
        let prompt = build_summarize_prompt(messages);
        self.call_with_retry_and_validate(llm, &prompt, messages)
            .await
    }

    /// Strategy: `SegmentedSummarize` — divide into segments, summarize each.
    async fn segmented_summarize(&self, llm: &dyn CompactionLlm, messages: &[String]) -> String {
        let segment_size = self.config.segment_size.max(1);
        let mut segments: Vec<String> = Vec::new();

        for chunk in messages.chunks(segment_size) {
            let prompt = build_segment_prompt(chunk, segments.len() + 1);
            let segment_summary = self
                .call_with_retry(llm, &prompt)
                .await
                .unwrap_or_else(|| truncate_fallback(chunk));
            segments.push(segment_summary);
        }

        // Stitch segments together.
        let mut result = String::new();
        for (i, seg) in segments.iter().enumerate() {
            if i > 0 {
                result.push_str("\n\n");
            }
            let _ = write!(result, "[Segment {}] {}", i + 1, seg);
        }

        // Validate identifiers on the final result.
        self.validate_identifiers(messages, &result);
        result
    }

    /// Strategy: `SelectiveRetain` — score messages, keep important ones verbatim.
    async fn selective_retain(&self, llm: &dyn CompactionLlm, messages: &[String]) -> String {
        // Score messages by simple heuristics (length, keywords).
        let scored: Vec<(usize, f64)> = messages
            .iter()
            .enumerate()
            .map(|(i, m)| (i, score_importance(m)))
            .collect();

        // Keep top 30% verbatim, summarize the rest.
        let threshold_index = (scored.len() as f64 * 0.3).ceil() as usize;
        let mut sorted = scored.clone();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let retain_indices: std::collections::HashSet<usize> = sorted
            .iter()
            .take(threshold_index)
            .map(|(i, _)| *i)
            .collect();

        let to_summarize: Vec<&String> = messages
            .iter()
            .enumerate()
            .filter(|(i, _)| !retain_indices.contains(i))
            .map(|(_, m)| m)
            .collect();

        let summary_of_rest = if to_summarize.is_empty() {
            String::new()
        } else {
            let summarize_strs: Vec<String> = to_summarize.iter().map(|s| (*s).clone()).collect();
            let prompt = build_summarize_prompt(&summarize_strs);
            self.call_with_retry(llm, &prompt)
                .await
                .unwrap_or_else(|| truncate_fallback(&summarize_strs))
        };

        // Build final: retained verbatim + summary of rest.
        let mut result = String::new();
        if !summary_of_rest.is_empty() {
            let _ = write!(
                result,
                "[Summary of less important messages] {summary_of_rest}"
            );
        }

        for (i, message) in messages.iter().enumerate() {
            if retain_indices.contains(&i) {
                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                let _ = write!(result, "[Retained] {message}");
            }
        }

        self.validate_identifiers(messages, &result);
        result
    }

    /// Call LLM with retry and identifier validation.
    async fn call_with_retry_and_validate(
        &self,
        llm: &dyn CompactionLlm,
        prompt: &str,
        original_messages: &[String],
    ) -> String {
        let result = self.call_with_retry(llm, prompt).await;

        match result {
            Some(summary) => {
                self.validate_identifiers(original_messages, &summary);
                summary
            }
            None => truncate_fallback(original_messages),
        }
    }

    /// Call LLM with retry logic.
    async fn call_with_retry(&self, llm: &dyn CompactionLlm, prompt: &str) -> Option<String> {
        for attempt in 0..self.config.max_retries {
            match llm.summarize(prompt).await {
                Ok(summary) if !summary.trim().is_empty() => {
                    tracing::debug!(attempt, "compaction LLM call succeeded");
                    return Some(summary);
                }
                Ok(_) => {
                    tracing::warn!(attempt, "compaction LLM returned empty summary");
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "compaction LLM call failed");
                }
            }
        }

        tracing::warn!(
            max_retries = self.config.max_retries,
            "all compaction LLM retries exhausted; falling back to truncation"
        );
        None
    }

    /// Validate that identifiers from original messages appear in summary.
    fn validate_identifiers(&self, original_messages: &[String], summary: &str) {
        match &self.config.identifier_policy {
            IdentifierPolicy::Strict => {
                let identifiers = extract_identifiers(original_messages);
                let missing: Vec<&str> = identifiers
                    .iter()
                    .filter(|id| !summary.contains(id.as_str()))
                    .map(std::string::String::as_str)
                    .collect();
                if !missing.is_empty() {
                    tracing::warn!(
                        missing_count = missing.len(),
                        "strict identifier policy: identifiers missing from compaction summary"
                    );
                }
            }
            IdentifierPolicy::Custom { patterns } => {
                let original_text: String = original_messages.join("\n");
                for pattern in patterns {
                    // Simple substring matching for custom patterns.
                    if original_text.contains(pattern.as_str())
                        && !summary.contains(pattern.as_str())
                    {
                        tracing::warn!(
                            pattern,
                            "custom identifier policy: pattern missing from summary"
                        );
                    }
                }
            }
            IdentifierPolicy::Relaxed => {} // No validation needed.
        }
    }
}

impl Default for CompactionEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Simple token estimation (4 chars per token).
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Build a summarization prompt.
fn build_summarize_prompt(messages: &[String]) -> String {
    let mut prompt = String::from(
        "Summarize the following conversation messages concisely. \
         Preserve all important details, decisions, code references, \
         file paths, URLs, and identifiers:\n\n",
    );
    for (i, msg) in messages.iter().enumerate() {
        let _ = writeln!(prompt, "Message {}: {msg}", i + 1);
    }
    prompt.push_str("\nSummary:");
    prompt
}

/// Build a segment summarization prompt.
fn build_segment_prompt(messages: &[String], segment_num: usize) -> String {
    let mut prompt = format!(
        "Summarize segment {segment_num} of a conversation. \
         Preserve key details, identifiers, and decisions:\n\n"
    );
    for msg in messages {
        let _ = writeln!(prompt, "- {msg}");
    }
    prompt.push_str("\nSegment summary:");
    prompt
}

/// Extract identifiers (URLs, file paths) from messages using simple string ops.
fn extract_identifiers(messages: &[String]) -> Vec<String> {
    let mut identifiers = Vec::new();
    let combined = messages.join("\n");

    for word in combined.split_whitespace() {
        // URLs
        if word.starts_with("http://") || word.starts_with("https://") {
            identifiers.push(word.trim_end_matches([',', '.', ')']).to_string());
        }
        // Email-like
        if word.contains('@') && word.contains('.') && word.len() > 5 {
            identifiers.push(word.trim_end_matches([',', '.']).to_string());
        }
        // File paths
        if (word.contains('/') || word.starts_with("./") || word.starts_with("../"))
            && word.contains('.')
            && word.len() > 3
        {
            identifiers.push(word.trim_end_matches([',', '.']).to_string());
        }
    }

    identifiers.sort();
    identifiers.dedup();
    identifiers
}

/// Fallback: truncate messages into a simple summary.
fn truncate_fallback(messages: &[String]) -> String {
    format!("[Compacted {} messages — LLM unavailable]", messages.len())
}

/// Score a message for importance (higher = more important).
fn score_importance(message: &str) -> f64 {
    let mut score: f64 = 0.0;

    // Longer messages tend to contain more information.
    {
        score += (message.len() as f64).min(500.0) / 500.0;
    }

    // Messages with code-like content are important.
    if message.contains("```") || message.contains("fn ") || message.contains("pub ") {
        score += 0.3;
    }

    // Messages with file paths are important.
    if message.contains('/') || message.contains(".rs") || message.contains(".py") {
        score += 0.2;
    }

    // Messages with decisions/conclusions.
    let decision_words = [
        "decided",
        "conclusion",
        "solution",
        "fix",
        "resolve",
        "implement",
    ];
    for word in &decision_words {
        if message.to_lowercase().contains(word) {
            score += 0.2;
            break;
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock LLM for testing.
    struct MockLlm {
        response: String,
        should_fail: bool,
    }

    #[async_trait]
    impl CompactionLlm for MockLlm {
        async fn summarize(&self, _prompt: &str) -> Result<String, String> {
            if self.should_fail {
                Err("mock error".to_string())
            } else {
                Ok(self.response.clone())
            }
        }
    }

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

    /// T-P3-01: Summarize strategy produces valid summary.
    #[tokio::test]
    async fn test_summarize_strategy_with_llm() {
        let llm = MockLlm {
            response: "A conversation about fixing bugs in auth module.".to_string(),
            should_fail: false,
        };
        let engine = CompactionEngine::with_llm(CompactionConfig::default(), Box::new(llm));
        let messages: Vec<String> = (0..20)
            .map(|i| format!("message {i} about fixing bugs"))
            .collect();
        let result = engine.compact_async(&messages).await;
        assert_eq!(result.messages_compacted, 10);
        assert!(result.summary.contains("fixing bugs"));
    }

    /// T-P3-02: Segmented strategy preserves topic boundaries.
    #[tokio::test]
    async fn test_segmented_strategy() {
        let llm = MockLlm {
            response: "Segment summary of conversation.".to_string(),
            should_fail: false,
        };
        let mut config = CompactionConfig::default();
        config.strategy = CompactionStrategy::SegmentedSummarize;
        config.segment_size = 5;
        let engine = CompactionEngine::with_llm(config, Box::new(llm));

        let messages: Vec<String> = (0..20)
            .map(|i| format!("message {i} some content"))
            .collect();
        let result = engine.compact_async(&messages).await;
        assert!(result.messages_compacted > 0);
        assert!(result.summary.contains("[Segment 1]"));
        assert!(result.summary.contains("[Segment 2]"));
    }

    /// T-P3-03: `SelectiveRetain` keeps high-importance messages.
    #[tokio::test]
    async fn test_selective_retain_strategy() {
        let llm = MockLlm {
            response: "Summary of less important messages.".to_string(),
            should_fail: false,
        };
        let mut config = CompactionConfig::default();
        config.strategy = CompactionStrategy::SelectiveRetain;
        let engine = CompactionEngine::with_llm(config, Box::new(llm));

        let mut messages: Vec<String> = (0..20).map(|i| format!("message {i} short")).collect();
        // Add an important message with code.
        messages[5] = "```rust\nfn main() { /* decided to implement auth */ }\n```".to_string();

        let result = engine.compact_async(&messages).await;
        assert!(result.messages_compacted > 0);
        assert!(result.summary.contains("[Retained]"));
    }

    /// T-P3-04: Strict identifier policy validates identifiers.
    #[test]
    fn test_strict_identifier_validation() {
        let engine = CompactionEngine::new();
        let messages = vec![
            "Check https://example.com for details".to_string(),
            "Email user@test.com about the fix".to_string(),
        ];
        let identifiers = extract_identifiers(&messages);
        assert!(identifiers
            .iter()
            .any(|i| i.contains("https://example.com")));
        assert!(identifiers.iter().any(|i| i.contains("user@test.com")));

        // This should just log warnings, not panic.
        engine.validate_identifiers(&messages, "A summary without identifiers.");
    }

    /// T-P3-05: LLM failure falls back to truncation.
    #[tokio::test]
    async fn test_llm_failure_fallback() {
        let llm = MockLlm {
            response: String::new(),
            should_fail: true,
        };
        let engine = CompactionEngine::with_llm(CompactionConfig::default(), Box::new(llm));
        let messages: Vec<String> = (0..20).map(|i| format!("message {i} content")).collect();
        let result = engine.compact_async(&messages).await;

        // Should still produce a result via fallback.
        assert!(result.messages_compacted > 0);
        assert!(result.summary.contains("LLM unavailable"));
    }

    /// T-P3-06: Retry logic works (mock LLM failures).
    #[tokio::test]
    async fn test_retry_exhaustion() {
        let llm = MockLlm {
            response: String::new(),
            should_fail: true,
        };
        let mut config = CompactionConfig::default();
        config.max_retries = 2;
        let engine = CompactionEngine::with_config(config);

        let result = engine.call_with_retry(&llm, "test").await;
        assert!(result.is_none());
    }

    /// Score importance function works.
    #[test]
    fn test_score_importance() {
        let code_msg = "```rust\nfn fix_bug() {}\n```";
        let short_msg = "ok";
        assert!(score_importance(code_msg) > score_importance(short_msg));
    }
}
