//! LLM-driven metadata extraction and tag generation for knowledge entries.
//!
//! Provides:
//! - [`TagGenerator`] trait for backward-compatible async tag generation
//! - [`MetadataExtractor`] trait for multi-dimensional metadata extraction
//!   via the `knowledge-metadata` sub-agent
//! - [`MetadataParser`] for parsing structured JSON from LLM output
//! - [`SummaryGenerator`] trait for LLM-driven L0/L1 summarization via
//!   the `knowledge-summarizer` sub-agent
//! - [`ContentPreparator`] for preparing document content within LLM
//!   context limits
//! - [`TagMerger`] for normalizing and deduplicating tag sets
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
use crate::metadata::DocumentMetadata;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Return a byte index clamped to the nearest char boundary at or before `max_bytes`.
///
/// Prevents panics when slicing multi-byte UTF-8 strings (e.g. CJK text).
fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    let idx = max_bytes.min(s.len());
    // Walk backward until we land on a char boundary.
    let mut pos = idx;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

// ---------------------------------------------------------------------------
// Token thresholds
// ---------------------------------------------------------------------------

/// Maximum tokens for "small" documents -- passed in full.
const SMALL_DOC_THRESHOLD: u32 = 4_000;

/// Maximum tokens for "medium" documents -- use summary + excerpts.
const MEDIUM_DOC_THRESHOLD: u32 = 30_000;

/// Target window size for map-reduce chunking of large documents.
const MAP_REDUCE_WINDOW_TOKENS: u32 = 4_000;

/// Maximum number of tags to keep after merging.
const MAX_TAGS: usize = 15;

// ---------------------------------------------------------------------------
// TagGenerator trait (backward compat)
// ---------------------------------------------------------------------------

/// Trait for async tag generation via LLM sub-agent.
///
/// Retained for backward compatibility. New code should prefer
/// [`MetadataExtractor`] which returns full [`DocumentMetadata`].
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
// MetadataExtractor trait
// ---------------------------------------------------------------------------

/// Trait for multi-dimensional metadata extraction via LLM sub-agent.
///
/// Implementations call the `knowledge-metadata` built-in agent to extract
/// structured classification (document type, industry, sub-category,
/// interpreted title, and topic tags) from document content.
#[async_trait]
pub trait MetadataExtractor: Send + Sync {
    /// Extract multi-dimensional metadata from document content.
    ///
    /// Uses L0 summary and L1 section titles as input to the metadata agent.
    /// Returns a populated [`DocumentMetadata`] struct.
    async fn extract_metadata(
        &self,
        content: &str,
        l0_summary: Option<&str>,
        l1_section_titles: &[String],
        original_filename: Option<&str>,
    ) -> Result<DocumentMetadata, KnowledgeError>;
}

// ---------------------------------------------------------------------------
// MetadataParser
// ---------------------------------------------------------------------------

/// Strip all `<think>...</think>` chain-of-thought blocks from LLM output.
///
/// Some models (e.g. `DeepSeek`, Qwen with extended thinking) wrap their
/// reasoning in `<think>` tags before the actual JSON payload. These blocks
/// often contain curly braces that confuse the naive `find('{')` JSON
/// extraction fallback.
///
/// This function handles:
/// - `<think>` appearing after leading whitespace or other preamble text
/// - Multiple consecutive `<think>` blocks
/// - Unclosed `<think>` blocks (returns text up to the opening tag)
fn strip_think_tags(text: &str) -> String {
    let mut result = text.to_owned();
    loop {
        match result.find("<think>") {
            None => break,
            Some(start) => {
                if let Some(end) = result.find("</think>") {
                    // Replace `<think>...</think>` (including the closing tag) with a space.
                    let after = end + "</think>".len();
                    result.replace_range(start..after, " ");
                } else {
                    // Unclosed tag: drop everything from `<think>` onward.
                    result.truncate(start);
                    break;
                }
            }
        }
    }
    result
}

/// Parses structured JSON output from the `knowledge-metadata` agent
/// into a [`DocumentMetadata`] struct.
///
/// Handles common LLM output quirks: markdown code fences, leading text,
/// null fields, partial JSON objects, and `<think>` chain-of-thought blocks.
pub struct MetadataParser;

impl MetadataParser {
    /// Parse LLM output into [`DocumentMetadata`].
    ///
    /// Attempts JSON parsing with fallback for markdown fences,
    /// `<think>` blocks, and non-JSON preamble.
    pub fn parse(llm_output: &str) -> Result<DocumentMetadata, KnowledgeError> {
        tracing::debug!(
            raw_len = llm_output.len(),
            raw_preview = %&llm_output[..floor_char_boundary(llm_output, 300)],
            "MetadataParser: raw LLM output received"
        );

        // Strip <think>...</think> blocks first.
        let stripped = strip_think_tags(llm_output);
        let trimmed = stripped.trim();
        tracing::debug!(
            stripped_len = trimmed.len(),
            stripped_preview = %&trimmed[..floor_char_boundary(trimmed, 300)],
            "MetadataParser: after stripping think tags"
        );

        // Strip markdown code fences if present.
        let cleaned = if trimmed.starts_with("```") {
            let inner = trimmed
                .trim_start_matches("```json")
                .trim_start_matches("```JSON")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            tracing::debug!(
                cleaned_len = inner.len(),
                "MetadataParser: stripped markdown fence"
            );
            inner
        } else {
            trimmed
        };

        // Try direct parse.
        if let Ok(meta) = serde_json::from_str::<DocumentMetadata>(cleaned) {
            tracing::debug!("MetadataParser: direct JSON parse succeeded");
            return Ok(meta);
        }

        // Try finding a JSON object in the output.
        if let Some(start) = cleaned.find('{') {
            if let Some(end) = cleaned.rfind('}') {
                let json_str = &cleaned[start..=end];
                tracing::debug!(
                    json_preview = %&json_str[..floor_char_boundary(json_str, 300)],
                    "MetadataParser: trying extracted JSON object"
                );
                match serde_json::from_str::<DocumentMetadata>(json_str) {
                    Ok(meta) => {
                        tracing::debug!("MetadataParser: extracted JSON object parse succeeded");
                        return Ok(meta);
                    }
                    Err(e) => {
                        tracing::debug!(err = %e, "MetadataParser: extracted JSON object parse failed");
                    }
                }
            }
        }

        tracing::warn!(
            cleaned_preview = %&cleaned[..floor_char_boundary(cleaned, 500)],
            "MetadataParser: all parse strategies exhausted, returning error"
        );
        Err(KnowledgeError::IngestionError {
            message: format!(
                "failed to parse metadata from LLM output (first 200 chars): {}",
                &trimmed[..floor_char_boundary(trimmed, 200)]
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// SummaryGenerator trait
// ---------------------------------------------------------------------------

/// LLM-generated summary output containing L0 and L1 content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSummary {
    /// Concise document summary (L0 level, 100-300 tokens).
    pub l0_summary: String,
    /// Structured L1 sections with title and summary.
    pub l1_sections: Vec<LlmL1Section>,
}

/// A single L1 section from LLM summarization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmL1Section {
    /// Section title.
    pub title: String,
    /// Approximate line range of this section in the source file (e.g. "L0-L349").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<String>,
    /// Section summary.
    pub summary: String,
}

/// Trait for LLM-driven document summarization.
///
/// Implementations call the `knowledge-summarizer` built-in agent which
/// uses `FileRead` tool calls to progressively read large files and
/// generate structured summaries without context window overflow.
#[async_trait]
pub trait SummaryGenerator: Send + Sync {
    /// Generate an LLM-driven summary for a document.
    ///
    /// `file_path` is passed to the summarizer agent so it can use
    /// `FileRead` with `line_offset`/`limit` for progressive reading.
    /// `total_lines` tells the agent the file size for reading strategy.
    async fn generate_summary(
        &self,
        file_path: &str,
        total_lines: usize,
        original_filename: &str,
    ) -> Result<LlmSummary, KnowledgeError>;
}

/// Parses structured JSON output from the `knowledge-summarizer` agent
/// into an [`LlmSummary`] struct.
pub struct SummaryParser;

impl SummaryParser {
    /// Parse LLM output into [`LlmSummary`].
    pub fn parse(llm_output: &str) -> Result<LlmSummary, KnowledgeError> {
        tracing::debug!(
            raw_len = llm_output.len(),
            raw_preview = %&llm_output[..floor_char_boundary(llm_output, 300)],
            "SummaryParser: raw LLM output received"
        );

        // Strip <think>...</think> blocks first.
        let stripped = strip_think_tags(llm_output);
        let trimmed = stripped.trim();
        tracing::debug!(
            stripped_len = trimmed.len(),
            stripped_preview = %&trimmed[..floor_char_boundary(trimmed, 300)],
            "SummaryParser: after stripping think tags"
        );

        // Strip markdown code fences if present.
        let cleaned = if trimmed.starts_with("```") {
            let inner = trimmed
                .trim_start_matches("```json")
                .trim_start_matches("```JSON")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            tracing::debug!(
                cleaned_len = inner.len(),
                "SummaryParser: stripped markdown fence"
            );
            inner
        } else {
            trimmed
        };

        // Try direct parse.
        if let Ok(summary) = serde_json::from_str::<LlmSummary>(cleaned) {
            tracing::debug!("SummaryParser: direct JSON parse succeeded");
            return Ok(summary);
        }

        // Try finding a JSON object in the output.
        if let Some(start) = cleaned.find('{') {
            if let Some(end) = cleaned.rfind('}') {
                let json_str = &cleaned[start..=end];
                tracing::debug!(
                    json_preview = %&json_str[..floor_char_boundary(json_str, 300)],
                    "SummaryParser: trying extracted JSON object"
                );
                match serde_json::from_str::<LlmSummary>(json_str) {
                    Ok(summary) => {
                        tracing::debug!("SummaryParser: extracted JSON object parse succeeded");
                        return Ok(summary);
                    }
                    Err(e) => {
                        tracing::debug!(err = %e, "SummaryParser: extracted JSON object parse failed");
                    }
                }
            }
        }

        tracing::warn!(
            cleaned_preview = %&cleaned[..floor_char_boundary(cleaned, 500)],
            "SummaryParser: all parse strategies exhausted, returning error"
        );
        Err(KnowledgeError::IngestionError {
            message: format!(
                "failed to parse summary from LLM output (first 200 chars): {}",
                &trimmed[..floor_char_boundary(trimmed, 200)]
            ),
        })
    }
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

use crate::chunking::estimate_tokens;

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

    // --- MetadataParser tests ---

    #[test]
    fn test_metadata_parser_valid_json() {
        let output = r#"{
            "document_type": "standards",
            "industry": "cybersecurity",
            "subcategory": "cryptography",
            "interpreted_title": "Applied Cryptography",
            "title_language": "en",
            "topics": ["aes", "rsa"]
        }"#;
        let meta = MetadataParser::parse(output).expect("should parse");
        assert_eq!(meta.document_type, Some("standards".to_string()));
        assert_eq!(meta.industry, Some("cybersecurity".to_string()));
        assert_eq!(meta.subcategory, Some("cryptography".to_string()));
        assert_eq!(
            meta.interpreted_title,
            Some("Applied Cryptography".to_string())
        );
        assert_eq!(meta.topics, vec!["aes", "rsa"]);
    }

    #[test]
    fn test_metadata_parser_with_code_fences() {
        let output = "```json\n{\"document_type\": \"paper\", \"topics\": [\"ml\"]}\n```";
        let meta = MetadataParser::parse(output).expect("should parse");
        assert_eq!(meta.document_type, Some("paper".to_string()));
    }

    #[test]
    fn test_metadata_parser_with_preamble() {
        let output =
            "Here is the metadata:\n{\"document_type\": \"manual\", \"industry\": \"engineering\"}";
        let meta = MetadataParser::parse(output).expect("should parse");
        assert_eq!(meta.document_type, Some("manual".to_string()));
        assert_eq!(meta.industry, Some("engineering".to_string()));
    }

    #[test]
    fn test_metadata_parser_with_null_fields() {
        let output = r#"{"document_type": "paper", "industry": null, "topics": []}"#;
        let meta = MetadataParser::parse(output).expect("should parse");
        assert_eq!(meta.document_type, Some("paper".to_string()));
        assert!(meta.industry.is_none());
        assert!(meta.topics.is_empty());
    }

    #[test]
    fn test_metadata_parser_invalid() {
        let output = "This is not JSON at all";
        assert!(MetadataParser::parse(output).is_err());
    }

    // --- SummaryParser tests ---

    #[test]
    fn test_summary_parser_valid_json() {
        let output = r#"{
            "l0_summary": "A guide to cryptography.",
            "l1_sections": [
                {"title": "Introduction", "summary": "Overview of crypto."},
                {"title": "AES", "summary": "Symmetric encryption."}
            ]
        }"#;
        let summary = SummaryParser::parse(output).expect("should parse");
        assert_eq!(summary.l0_summary, "A guide to cryptography.");
        assert_eq!(summary.l1_sections.len(), 2);
        assert_eq!(summary.l1_sections[0].title, "Introduction");
    }

    #[test]
    fn test_summary_parser_with_code_fences() {
        let output = "```json\n{\"l0_summary\": \"Test\", \"l1_sections\": []}\n```";
        let summary = SummaryParser::parse(output).expect("should parse");
        assert_eq!(summary.l0_summary, "Test");
    }

    #[test]
    fn test_summary_parser_invalid() {
        let output = "Not valid JSON";
        assert!(SummaryParser::parse(output).is_err());
    }

    // --- strip_think_tags tests ---

    #[test]
    fn test_strip_think_tags_basic() {
        let input = "<think>reasoning here</think>\n\n{\"key\": \"value\"}";
        assert_eq!(strip_think_tags(input).trim(), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_think_tags_with_braces_in_thinking() {
        let input =
            "<think>I need to output {\"foo\": \"bar\"} format</think>\n\n{\"actual\": \"data\"}";
        assert_eq!(strip_think_tags(input).trim(), "{\"actual\": \"data\"}");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "{\"key\": \"value\"}";
        assert_eq!(strip_think_tags(input).trim(), "{\"key\": \"value\"}");
    }

    #[test]
    fn test_strip_think_tags_preamble_before_tag() {
        // Preamble text before <think> should be preserved; think block removed.
        let input = "Sure!\n<think>inner reasoning</think>\n{\"key\": \"value\"}";
        let result = strip_think_tags(input);
        assert!(result.contains("{\"key\": \"value\"}"));
        assert!(!result.contains("inner reasoning"));
    }

    #[test]
    fn test_strip_think_tags_multiple_blocks() {
        let input = "<think>first</think>middle<think>second</think>end";
        let result = strip_think_tags(input);
        assert!(!result.contains("first"));
        assert!(!result.contains("second"));
        assert!(result.contains("middle"));
        assert!(result.contains("end"));
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "prefix<think>unclosed reasoning";
        let result = strip_think_tags(input);
        assert_eq!(result.trim(), "prefix");
    }

    #[test]
    fn test_metadata_parser_with_think_tags() {
        let output = "<think>Let me analyze this document. It seems like a standards doc with {json} content.\n</think>\n\n{\"document_type\": \"standards\", \"industry\": \"automotive\", \"topics\": [\"safety\"]}";
        let meta = MetadataParser::parse(output).expect("should parse with think tags");
        assert_eq!(meta.document_type, Some("standards".to_string()));
        assert_eq!(meta.industry, Some("automotive".to_string()));
    }

    #[test]
    fn test_metadata_parser_with_think_tags_and_preamble() {
        // Simulates a model that outputs text before the <think> block.
        let output = "Here is my analysis:\n<think>thinking...</think>\n{\"document_type\": \"standards\", \"industry\": \"automotive\", \"topics\": [\"safety\"]}";
        let meta =
            MetadataParser::parse(output).expect("should parse with think tags and preamble");
        assert_eq!(meta.document_type, Some("standards".to_string()));
    }

    #[test]
    fn test_summary_parser_with_think_tags() {
        let output = "<think>I'll summarize this document now.</think>\n\n{\"l0_summary\": \"Test summary.\", \"l1_sections\": [{\"title\": \"Intro\", \"summary\": \"Overview.\"}]}";
        let summary = SummaryParser::parse(output).expect("should parse with think tags");
        assert_eq!(summary.l0_summary, "Test summary.");
        assert_eq!(summary.l1_sections.len(), 1);
    }

    // --- floor_char_boundary tests ---

    #[test]
    fn test_floor_char_boundary_ascii() {
        let s = "hello world";
        assert_eq!(floor_char_boundary(s, 5), 5);
        assert_eq!(floor_char_boundary(s, 100), s.len());
        assert_eq!(floor_char_boundary(s, 0), 0);
    }

    #[test]
    fn test_floor_char_boundary_multibyte() {
        // Each CJK char is 3 bytes in UTF-8.
        let s = "abcde\u{4EE3}\u{7801}"; // "abcde" (5 bytes) + 2 CJK chars (6 bytes) = 11 bytes
                                         // Byte 5 is the start of first CJK char -> valid boundary
        assert_eq!(floor_char_boundary(s, 5), 5);
        // Byte 6 is inside first CJK char -> floor to 5
        assert_eq!(floor_char_boundary(s, 6), 5);
        // Byte 7 is inside first CJK char -> floor to 5
        assert_eq!(floor_char_boundary(s, 7), 5);
        // Byte 8 is the start of second CJK char -> valid boundary
        assert_eq!(floor_char_boundary(s, 8), 8);
    }

    #[test]
    fn test_summary_parser_no_panic_on_cjk_content() {
        // Build a CJK string longer than 500 bytes to exercise the preview truncation.
        // Each CJK char is 3 bytes, so 200 chars = 600 bytes.
        let cjk: String = std::iter::repeat('\u{4EE3}').take(200).collect();
        let output = format!("Not JSON: {cjk}");
        // Should return Err, but must NOT panic.
        assert!(SummaryParser::parse(&output).is_err());
    }

    #[test]
    fn test_metadata_parser_no_panic_on_cjk_content() {
        let cjk: String = std::iter::repeat('\u{7801}').take(200).collect();
        let output = format!("Not JSON: {cjk}");
        assert!(MetadataParser::parse(&output).is_err());
    }
}
