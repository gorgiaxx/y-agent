//! LLM-driven tag generation for knowledge entries.
//!
//! Provides the [`TagGenerator`] trait for async tag generation via the
//! `knowledge-tagger` built-in sub-agent, [`ContentPreparator`] for
//! preparing document content within LLM context limits, and [`TagMerger`]
//! for normalizing and deduplicating tag sets.
//!
//! # Token-Aware Content Preparation
//!
//! Documents are classified by estimated token count into three tiers:
//!
//! | Tier   | Threshold       | Strategy                              |
//! |--------|-----------------|---------------------------------------|
//! | Small  | < 4K tokens     | Pass full content to tagger agent     |
//! | Medium | 4K - 30K tokens | L0 summary + L1 titles + excerpts     |
//! | Large  | > 30K tokens    | Map-reduce: per-window tags + merge   |

use crate::error::KnowledgeError;
use async_trait::async_trait;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Token thresholds
// ---------------------------------------------------------------------------

/// Maximum tokens for "small" documents — passed in full.
const SMALL_DOC_THRESHOLD: u32 = 4_000;

/// Maximum tokens for "medium" documents — use summary + excerpts.
const MEDIUM_DOC_THRESHOLD: u32 = 30_000;

/// Target window size for map-reduce chunking of large documents.
const MAP_REDUCE_WINDOW_TOKENS: u32 = 4_000;

/// Maximum number of tags to keep after merging.
const MAX_TAGS: usize = 15;

// ---------------------------------------------------------------------------
// TagGenerator trait
// ---------------------------------------------------------------------------

/// Trait for async tag generation via LLM sub-agent.
///
/// Implementations call the `knowledge-tagger` built-in agent via
/// `AgentDelegator` to generate tags for knowledge content.
#[async_trait]
pub trait TagGenerator: Send + Sync {
    /// Generate tags for the given content.
    ///
    /// The implementation handles chunking for large documents internally.
    /// Returns a normalized, deduplicated list of tags.
    async fn generate_tags(
        &self,
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
    ) -> Result<Vec<String>, KnowledgeError>;
}

// ---------------------------------------------------------------------------
// ContentPreparator
// ---------------------------------------------------------------------------

/// Prepares document content for the tagger agent, respecting token limits.
///
/// Selects the appropriate strategy based on content size:
/// - **Small** (< 4K tokens): full content
/// - **Medium** (4K-30K tokens): L0 summary + L1 section titles + excerpts
/// - **Large** (> 30K tokens): chunked windows for map-reduce tagging
pub struct ContentPreparator {
    /// Target window size in tokens for map-reduce chunking.
    window_tokens: u32,
}

/// The prepared content strategy returned by [`ContentPreparator::prepare`].
pub enum PreparedContent {
    /// Full content fits within context window.
    Full(String),
    /// Summary + section titles + first/last excerpts.
    Summarized(String),
    /// Multiple windows for map-reduce tagging.
    MapReduce(Vec<String>),
}

impl Default for ContentPreparator {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentPreparator {
    /// Create a new `ContentPreparator` with default window size.
    pub fn new() -> Self {
        Self {
            window_tokens: MAP_REDUCE_WINDOW_TOKENS,
        }
    }

    /// Create with a custom window size.
    pub fn with_window_tokens(window_tokens: u32) -> Self {
        Self { window_tokens }
    }

    /// Prepare content for the tagger agent.
    ///
    /// Returns the appropriate [`PreparedContent`] variant based on
    /// estimated token count.
    pub fn prepare(
        &self,
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
    ) -> PreparedContent {
        let tokens = estimate_tokens(content);

        if tokens <= SMALL_DOC_THRESHOLD {
            PreparedContent::Full(content.to_string())
        } else if tokens <= MEDIUM_DOC_THRESHOLD {
            PreparedContent::Summarized(Self::build_summary_input(
                content,
                l0_summary,
                l1_section_titles,
            ))
        } else {
            PreparedContent::MapReduce(self.split_into_windows(content))
        }
    }

    /// Build a summarized input combining L0 summary, L1 section titles,
    /// and first/last content excerpts.
    fn build_summary_input(
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
    ) -> String {
        let mut parts = Vec::new();

        if let Some(summary) = l0_summary {
            parts.push(format!("Document Summary:\n{summary}"));
        }

        if !l1_section_titles.is_empty() {
            let titles = l1_section_titles.join(", ");
            parts.push(format!("Section Titles: {titles}"));
        }

        // Add first ~1000 chars and last ~1000 chars as excerpts.
        let chars: Vec<char> = content.chars().collect();
        let excerpt_len = 1000;
        if chars.len() > excerpt_len * 2 {
            let first: String = chars[..excerpt_len].iter().collect();
            let last: String = chars[chars.len() - excerpt_len..].iter().collect();
            parts.push(format!("Content Excerpt (start):\n{first}"));
            parts.push(format!("Content Excerpt (end):\n{last}"));
        } else {
            parts.push(format!("Content:\n{content}"));
        }

        parts.join("\n\n")
    }

    /// Split content into windows of approximately `window_tokens` tokens each.
    fn split_into_windows(&self, content: &str) -> Vec<String> {
        let chars_per_window = tokens_to_chars(self.window_tokens);
        let chars: Vec<char> = content.chars().collect();
        let mut windows = Vec::new();

        let mut start = 0;
        while start < chars.len() {
            let end = (start + chars_per_window).min(chars.len());
            let window: String = chars[start..end].iter().collect();
            if !window.trim().is_empty() {
                windows.push(window);
            }
            start = end;
        }

        windows
    }
}

// ---------------------------------------------------------------------------
// TagMerger
// ---------------------------------------------------------------------------

/// Merges, deduplicates, and normalizes tags from multiple sources.
///
/// Handles:
/// - Case-insensitive deduplication
/// - Format normalization (lowercase, spaces to hyphens)
/// - Frequency-based ranking (tags appearing more often rank higher)
/// - Capping at a configurable maximum (default: 15)
pub struct TagMerger;

impl TagMerger {
    /// Merge multiple tag sets into a single normalized list.
    ///
    /// Tags are ranked by frequency (how many input sets contain them),
    /// deduplicated case-insensitively, and capped at `MAX_TAGS`.
    pub fn merge(tag_sets: &[Vec<String>]) -> Vec<String> {
        let mut frequency: HashMap<String, usize> = HashMap::new();

        for tags in tag_sets {
            for tag in tags {
                let normalized = Self::normalize_tag(tag);
                if !normalized.is_empty() {
                    *frequency.entry(normalized).or_insert(0) += 1;
                }
            }
        }

        // Sort by frequency descending, then alphabetically for ties.
        let mut sorted: Vec<(String, usize)> = frequency.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        sorted
            .into_iter()
            .take(MAX_TAGS)
            .map(|(tag, _)| tag)
            .collect()
    }

    /// Normalize a single tag: lowercase, trim, replace spaces with hyphens,
    /// remove non-alphanumeric characters except hyphens.
    pub fn normalize_tag(tag: &str) -> String {
        tag.trim()
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    }

    /// Parse a JSON array of tags from LLM output.
    ///
    /// Handles common LLM output quirks:
    /// - Leading/trailing whitespace
    /// - Markdown code fences around JSON
    /// - Single tags without array brackets
    pub fn parse_tags(llm_output: &str) -> Vec<String> {
        let trimmed = llm_output.trim();

        // Strip markdown code fences if present.
        let cleaned = if trimmed.starts_with("```") {
            let inner = trimmed
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            inner
        } else {
            trimmed
        };

        // Try parsing as JSON array.
        if let Ok(tags) = serde_json::from_str::<Vec<String>>(cleaned) {
            return tags
                .into_iter()
                .map(|t| Self::normalize_tag(&t))
                .filter(|t| !t.is_empty())
                .collect();
        }

        // Fallback: split by commas or newlines.
        cleaned
            .split([',', '\n'])
            .map(|s| {
                let s = s.trim().trim_matches('"').trim_matches('\'');
                Self::normalize_tag(s)
            })
            .filter(|t| !t.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Token estimation helpers
// ---------------------------------------------------------------------------

/// Estimate token count for a text string (4 chars per token heuristic).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Convert a token count to approximate character count.
fn tokens_to_chars(tokens: u32) -> usize {
    (tokens as usize) * 4
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- TagMerger tests ---

    #[test]
    fn test_normalize_tag_basic() {
        assert_eq!(TagMerger::normalize_tag("Error Handling"), "error-handling");
        assert_eq!(TagMerger::normalize_tag("  RUST  "), "rust");
        assert_eq!(TagMerger::normalize_tag("ISO-26262"), "iso-26262");
    }

    #[test]
    fn test_normalize_tag_special_chars() {
        assert_eq!(TagMerger::normalize_tag("c++"), "c");
        assert_eq!(TagMerger::normalize_tag("node.js"), "nodejs");
        assert_eq!(
            TagMerger::normalize_tag("--leading-trailing--"),
            "leading-trailing"
        );
    }

    #[test]
    fn test_normalize_tag_empty() {
        assert_eq!(TagMerger::normalize_tag(""), "");
        assert_eq!(TagMerger::normalize_tag("   "), "");
        assert_eq!(TagMerger::normalize_tag("---"), "");
    }

    #[test]
    fn test_merge_single_set() {
        let sets = vec![vec![
            "rust".to_string(),
            "Error Handling".to_string(),
            "RUST".to_string(),
        ]];
        let merged = TagMerger::merge(&sets);
        // "rust" and "RUST" should deduplicate.
        assert!(merged.contains(&"rust".to_string()));
        assert!(merged.contains(&"error-handling".to_string()));
        assert!(merged.len() <= 2);
    }

    #[test]
    fn test_merge_frequency_ranking() {
        let sets = vec![
            vec!["rust".to_string(), "python".to_string()],
            vec!["rust".to_string(), "java".to_string()],
            vec!["rust".to_string(), "python".to_string()],
        ];
        let merged = TagMerger::merge(&sets);
        // "rust" appears 3 times, should be first.
        assert_eq!(merged[0], "rust");
        // "python" appears 2 times, should be before "java" (1 time).
        let python_pos = merged.iter().position(|t| t == "python").unwrap();
        let java_pos = merged.iter().position(|t| t == "java").unwrap();
        assert!(python_pos < java_pos);
    }

    #[test]
    fn test_merge_caps_at_max() {
        let tags: Vec<String> = (0..20).map(|i| format!("tag-{i}")).collect();
        let sets = vec![tags];
        let merged = TagMerger::merge(&sets);
        assert!(merged.len() <= MAX_TAGS);
    }

    #[test]
    fn test_parse_tags_json_array() {
        let output = r#"["rust", "error-handling", "result-type"]"#;
        let tags = TagMerger::parse_tags(output);
        assert_eq!(tags, vec!["rust", "error-handling", "result-type"]);
    }

    #[test]
    fn test_parse_tags_with_code_fences() {
        let output = "```json\n[\"rust\", \"tokio\"]\n```";
        let tags = TagMerger::parse_tags(output);
        assert_eq!(tags, vec!["rust", "tokio"]);
    }

    #[test]
    fn test_parse_tags_fallback_csv() {
        let output = "rust, error-handling, tokio";
        let tags = TagMerger::parse_tags(output);
        assert_eq!(tags, vec!["rust", "error-handling", "tokio"]);
    }

    #[test]
    fn test_parse_tags_with_whitespace() {
        let output = "  [\"rust\", \"async\"]  ";
        let tags = TagMerger::parse_tags(output);
        assert_eq!(tags, vec!["rust", "async"]);
    }

    // --- ContentPreparator tests ---

    #[test]
    fn test_prepare_small_content() {
        let preparator = ContentPreparator::new();
        let content = "Short document about Rust.";
        let prepared = preparator.prepare(content, None, &[]);
        assert!(matches!(prepared, PreparedContent::Full(_)));
        if let PreparedContent::Full(text) = prepared {
            assert_eq!(text, content);
        }
    }

    #[test]
    fn test_prepare_medium_content() {
        let preparator = ContentPreparator::new();
        // ~5K tokens = ~20K chars.
        let content = "x".repeat(20_000);
        let summary = "This is a summary.";
        let titles = vec!["Section A".to_string(), "Section B".to_string()];
        let prepared = preparator.prepare(&content, Some(summary), &titles);
        assert!(matches!(prepared, PreparedContent::Summarized(_)));
        if let PreparedContent::Summarized(text) = prepared {
            assert!(text.contains("Document Summary:"));
            assert!(text.contains("Section A"));
            assert!(text.contains("Content Excerpt"));
        }
    }

    #[test]
    fn test_prepare_large_content() {
        let preparator = ContentPreparator::new();
        // ~40K tokens = ~160K chars.
        let content = "x".repeat(160_000);
        let prepared = preparator.prepare(&content, None, &[]);
        assert!(matches!(prepared, PreparedContent::MapReduce(_)));
        if let PreparedContent::MapReduce(windows) = prepared {
            assert!(windows.len() > 1);
            // Each window should be approximately window_tokens * 4 chars.
            for window in &windows {
                assert!(!window.is_empty());
            }
        }
    }

    #[test]
    fn test_prepare_medium_without_summary() {
        let preparator = ContentPreparator::new();
        let content = "y".repeat(20_000);
        let prepared = preparator.prepare(&content, None, &[]);
        assert!(matches!(prepared, PreparedContent::Summarized(_)));
        if let PreparedContent::Summarized(text) = prepared {
            // Without summary, should still have content excerpts.
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn test_split_into_windows() {
        let preparator = ContentPreparator::with_window_tokens(100);
        let content = "a".repeat(2000);
        let windows = preparator.split_into_windows(&content);
        // 2000 chars / (100 * 4 = 400 chars per window) = 5 windows.
        assert_eq!(windows.len(), 5);
    }
}
