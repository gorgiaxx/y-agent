//! L0/L1/L2 multi-resolution chunking strategy.
//!
//! - **L0**: Summary-level chunks (one per document, < 200 tokens)
//! - **L1**: Section-level chunks (one per major section, < 500 tokens)
//! - **L2**: Paragraph-level chunks (granular, < 1000 tokens)
//!
//! Supports multiple chunking algorithms:
//! - **`TextSplit`**: Simple newline-based splitting (legacy)
//! - **`SentenceBoundary`**: Punctuation-aware splitting (`MaxKB`-inspired)
//! - **`HeadingBased`**: Markdown heading-based splitting

use crate::config::KnowledgeConfig;
use serde::{Deserialize, Serialize};

/// Resolution level for chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChunkLevel {
    /// Summary: one chunk per document, compact overview.
    L0,
    /// Section: one chunk per major section.
    L1,
    /// Paragraph: granular, full-detail chunks.
    L2,
}

impl ChunkLevel {
    /// Parse a resolution string (`"l0"`, `"l1"`, `"l2"`) into a `ChunkLevel`.
    ///
    /// Returns `None` for unrecognized strings, which callers treat as
    /// "return all levels" (no level filter).
    pub fn from_resolution(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "l0" => Some(Self::L0),
            "l1" => Some(Self::L1),
            "l2" => Some(Self::L2),
            _ => None,
        }
    }
}

/// A chunk of knowledge content at a specific resolution level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk.
    pub id: String,
    /// The document this chunk belongs to.
    pub document_id: String,
    /// Resolution level.
    pub level: ChunkLevel,
    /// The chunked text content.
    pub content: String,
    /// Estimated token count.
    pub token_estimate: u32,
    /// Source metadata (URL, title, domain, etc.).
    pub metadata: ChunkMetadata,
}

/// Metadata attached to each chunk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Source URL or file path.
    pub source: String,
    /// Domain classification.
    pub domain: String,
    /// All domain classifications for this chunk.
    ///
    /// `domain` is retained for backward compatibility and stores the
    /// primary domain; this field lets retrieval match secondary domains.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Document title.
    pub title: String,
    /// Section index within the document.
    pub section_index: usize,
    /// Knowledge collection this chunk belongs to.
    #[serde(default)]
    pub collection: String,
    /// Index of the parent L1 section that this L2 chunk belongs to.
    ///
    /// `None` for L0/L1 chunks or when L1 alignment has not been computed.
    /// Used to enable resolution-aware retrieval: when `resolution=l1` is
    /// requested, the system can aggregate L2 chunks by their parent L1 section.
    #[serde(default)]
    pub l1_section_index: Option<usize>,
    /// ISO 8601 timestamp of when this chunk was indexed.
    ///
    /// Used with `RetrievalFilter::freshness_after` to exclude stale entries.
    #[serde(default)]
    pub indexed_at: Option<String>,
}

/// Chunking algorithm type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChunkerType {
    /// Simple newline-based splitting (legacy default).
    #[default]
    TextSplit,
    /// Punctuation-aware sentence boundary splitting (MaxKB-inspired).
    SentenceBoundary,
    /// Markdown heading-based splitting.
    HeadingBased,
}

/// Chunks documents at multiple resolution levels.
#[derive(Debug)]
pub struct ChunkingStrategy {
    config: KnowledgeConfig,
    chunker_type: ChunkerType,
}

impl ChunkingStrategy {
    pub fn new(config: KnowledgeConfig) -> Self {
        Self {
            config,
            chunker_type: ChunkerType::TextSplit,
        }
    }

    /// Create a strategy with a specific chunker type.
    pub fn with_chunker(config: KnowledgeConfig, chunker_type: ChunkerType) -> Self {
        Self {
            config,
            chunker_type,
        }
    }

    /// Chunk a document at the specified level.
    pub fn chunk(
        &self,
        document_id: &str,
        content: &str,
        level: ChunkLevel,
        metadata: &ChunkMetadata,
    ) -> Vec<Chunk> {
        match level {
            ChunkLevel::L0 => self.chunk_l0(document_id, content, metadata),
            ChunkLevel::L1 => self.chunk_l1(document_id, content, metadata),
            ChunkLevel::L2 => self.chunk_l2(document_id, content, metadata),
        }
    }

    /// L0: Produce a single summary chunk (truncated to max tokens).
    fn chunk_l0(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = tokens_to_max_chars(self.config.l0_max_tokens, content);
        let summary = truncate_to_chars(content, max_chars);

        vec![Chunk {
            id: format!("{document_id}-L0-0"),
            document_id: document_id.to_string(),
            level: ChunkLevel::L0,
            content: summary.to_string(),
            token_estimate: estimate_tokens(summary),
            metadata: metadata.clone(),
        }]
    }

    /// L1: Split by sections (algorithm depends on chunker type).
    fn chunk_l1(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = tokens_to_max_chars(self.config.l1_max_tokens, content);

        let sections = match self.chunker_type {
            ChunkerType::TextSplit => split_by_double_newline(content),
            ChunkerType::SentenceBoundary => split_by_sentence_boundary(content, max_chars),
            ChunkerType::HeadingBased => split_by_headings(content),
        };

        sections
            .into_iter()
            .enumerate()
            .map(|(i, section)| {
                let text = truncate_to_chars(&section, max_chars);
                let mut meta = metadata.clone();
                meta.section_index = i;
                Chunk {
                    id: format!("{document_id}-L1-{i}"),
                    document_id: document_id.to_string(),
                    level: ChunkLevel::L1,
                    content: text.to_string(),
                    token_estimate: estimate_tokens(text),
                    metadata: meta,
                }
            })
            .collect()
    }

    /// L2: Split by paragraphs (algorithm depends on chunker type).
    ///
    /// Uses `effective_l2_max_tokens()` which caps the chunk size at the
    /// embedding model's context window when embedding is enabled.
    fn chunk_l2(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = tokens_to_max_chars(self.config.effective_l2_max_tokens(), content);

        let paragraphs = match self.chunker_type {
            ChunkerType::SentenceBoundary => {
                // For L2, use smaller chunk target.
                split_by_sentence_boundary(content, max_chars / 2)
            }
            ChunkerType::HeadingBased | ChunkerType::TextSplit => split_by_single_newline(content),
        };

        paragraphs
            .into_iter()
            .enumerate()
            .map(|(i, para)| {
                let text = truncate_to_chars(&para, max_chars);
                let mut meta = metadata.clone();
                meta.section_index = i;
                Chunk {
                    id: format!("{document_id}-L2-{i}"),
                    document_id: document_id.to_string(),
                    level: ChunkLevel::L2,
                    content: text.to_string(),
                    token_estimate: estimate_tokens(text),
                    metadata: meta,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Truncation helper
// ---------------------------------------------------------------------------

/// Truncate a string to at most `max_chars` **characters** (not bytes).
///
/// Returns the original string if it's already within limit.
/// Safe for CJK / multi-byte text — never splits inside a character.
fn truncate_to_chars(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        return s;
    }
    // Find the byte offset of the `max_chars`-th character.
    let byte_offset = s
        .char_indices()
        .nth(max_chars)
        .map_or(s.len(), |(idx, _)| idx);
    &s[..byte_offset]
}

// ---------------------------------------------------------------------------
// Splitting helpers
// ---------------------------------------------------------------------------

/// Split by double newlines (legacy `TextSplit`).
fn split_by_double_newline(content: &str) -> Vec<String> {
    content
        .split("\n\n")
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .collect()
}

/// Split by single newlines (legacy `TextSplit` L2).
fn split_by_single_newline(content: &str) -> Vec<String> {
    content
        .split('\n')
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .collect()
}

/// Sentence boundary splitting (`MaxKB` `MarkChunkHandle` inspired).
///
/// Splits on Chinese/English sentence-ending punctuation (`。！？；.!?;`)
/// and newlines, then merges small fragments up to `max_chunk_chars`.
fn split_by_sentence_boundary(content: &str, max_chunk_chars: usize) -> Vec<String> {
    // Split on sentence boundaries.
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in content.chars() {
        current.push(ch);
        if is_sentence_end(ch) || ch == '\n' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }
    // Push any remaining text.
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    // Merge small sentences into chunks up to max size.
    merge_into_chunks(sentences, max_chunk_chars)
}

/// Check if a character is a sentence-ending punctuation mark.
fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';')
}

/// Merge a list of fragments into chunks, each up to `max_chars`.
fn merge_into_chunks(fragments: Vec<String>, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for fragment in fragments {
        if current.is_empty() {
            current = fragment;
        } else if current.chars().count() + 1 + fragment.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(&fragment);
        } else {
            chunks.push(current);
            current = fragment;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Split by Markdown headings (`#`, `##`, etc.).
///
/// Each heading starts a new section. Content before the first heading
/// (if any) becomes its own section.
fn split_by_headings(content: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current_section = String::new();

    for line in content.lines() {
        if line.starts_with('#') {
            // Save previous section if non-empty.
            let trimmed = current_section.trim().to_string();
            if !trimmed.is_empty() {
                sections.push(trimmed);
            }
            current_section = line.to_string();
            current_section.push('\n');
        } else {
            current_section.push_str(line);
            current_section.push('\n');
        }
    }

    let trimmed = current_section.trim().to_string();
    if !trimmed.is_empty() {
        sections.push(trimmed);
    }

    sections
}

/// Estimate token count for a text string.
///
/// Uses a mixed heuristic: CJK characters count as ~1.5 tokens each
/// (most embedding tokenizers split them into 1-2 BPE tokens),
/// while ASCII/Latin characters use the ~4-chars-per-token rule.
///
/// This is the canonical token estimator for the `y-knowledge` crate.
/// All modules should use this instead of local implementations to
/// ensure consistent token accounting.
pub fn estimate_tokens(text: &str) -> u32 {
    let mut cjk_chars = 0u32;
    let mut other_bytes = 0u32;

    for ch in text.chars() {
        if is_cjk_char(ch) {
            cjk_chars += 1;
        } else {
            other_bytes += u32::try_from(ch.len_utf8()).unwrap_or(0);
        }
    }

    // CJK: ~1.5 tokens per character; Latin/ASCII: ~4 bytes per token.
    let cjk_tokens = (cjk_chars * 3).div_ceil(2);
    let latin_tokens = other_bytes.div_ceil(4);
    cjk_tokens + latin_tokens
}

/// Convert a token limit to a character limit, accounting for CJK content.
///
/// Samples from three positions (start, middle, end) of `content` to estimate
/// the CJK ratio, then uses that ratio to compute how many characters fit
/// within `max_tokens`. Multi-position sampling avoids bias when a document
/// has an English abstract but Chinese body (or vice versa).
fn tokens_to_max_chars(max_tokens: u32, content: &str) -> usize {
    let total_chars = content.chars().count();
    let sample_size = 200.min(total_chars);

    // Sample start, middle, end.
    let samples = [
        0,
        total_chars.saturating_sub(sample_size) / 2,
        total_chars.saturating_sub(sample_size),
    ];
    let mut total_sampled = 0usize;
    let mut cjk_count = 0usize;

    for &offset in &samples {
        let s: String = content.chars().skip(offset).take(sample_size).collect();
        total_sampled += s.chars().count();
        cjk_count += s.chars().filter(|c| is_cjk_char(*c)).count();
    }

    let cjk_ratio = cjk_count as f64 / total_sampled.max(1) as f64;
    let chars_per_token = cjk_ratio * 0.67 + (1.0 - cjk_ratio) * 4.0;
    (f64::from(max_tokens) * chars_per_token) as usize
}

/// Check if a character is in the CJK Unified Ideographs range.
fn is_cjk_char(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}' |     // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}' |     // CJK Extension A
        '\u{3040}'..='\u{309F}' |     // Hiragana
        '\u{30A0}'..='\u{30FF}' |     // Katakana
        '\u{F900}'..='\u{FAFF}' |     // CJK Compatibility Ideographs
        '\u{20000}'..='\u{2A6DF}'     // CJK Extension B
    )
}

/// Merge adjacent chunks when count exceeds `max_count`.
///
/// Groups consecutive chunks into evenly-sized batches and joins them
/// with newlines. This keeps the total content intact while reducing
/// chunk count to at most `max_count`.
pub fn coalesce_chunks(chunks: Vec<String>, max_count: usize) -> Vec<String> {
    if chunks.len() <= max_count || max_count == 0 {
        return chunks;
    }
    // Divide into uniform groups of `group_size` adjacent chunks.
    let group_size = chunks.len().div_ceil(max_count);
    chunks
        .chunks(group_size)
        .map(|group| group.join("\n"))
        .collect()
}

/// Extract a section title from the first line of a chunk's content.
///
/// If the first non-empty line is a Markdown heading (`# …`, `## …`, etc.),
/// the heading text is returned. Otherwise falls back to `"Section {index + 1}"`.
pub fn extract_section_title(content: &str, index: usize) -> String {
    if let Some(first_line) = content.lines().find(|l| !l.trim().is_empty()) {
        let trimmed = first_line.trim();
        if trimmed.starts_with('#') {
            let title = trimmed.trim_start_matches('#').trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    format!("Section {}", index + 1)
}

/// Check whether a section title is a generic fallback (e.g. `"Section 1"`).
///
/// Generic titles carry no information and waste tokens when injected into
/// LLM context. Callers should skip rendering titles that match this pattern.
pub fn is_generic_section_title(title: &str) -> bool {
    let trimmed = title.trim();
    trimmed
        .strip_prefix("Section ")
        .is_some_and(|rest| rest.parse::<u32>().is_ok())
}

/// Compute the parent L1 section index for each L2 chunk.
///
/// Both L1 sections and L2 chunks are sequential splits of the same document.
/// This function maps each L2 chunk to its parent L1 section by tracking
/// cumulative character positions: the midpoint of each L2 chunk is mapped
/// to the L1 section whose range contains that midpoint.
///
/// Returns a vector with one entry per L2 chunk, containing the L1 section
/// index (`Some(n)`) or `None` if no sections are available.
pub fn compute_l1_alignment(
    l1_sections: &[(usize, usize)],
    l2_chunks: &[String],
) -> Vec<Option<usize>> {
    if l1_sections.is_empty() {
        return vec![None; l2_chunks.len()];
    }

    // Compute cumulative end positions for L1 sections.
    // l1_boundaries[i] = (cumulative_byte_end, section_index)
    let mut l1_boundaries: Vec<(usize, usize)> = Vec::with_capacity(l1_sections.len());
    let mut pos = 0usize;
    for &(index, content_len) in l1_sections {
        pos += content_len;
        l1_boundaries.push((pos, index));
    }

    // For each L2 chunk, find which L1 section boundary contains its midpoint.
    let mut l2_pos = 0usize;
    l2_chunks
        .iter()
        .map(|chunk| {
            let chunk_mid = l2_pos + chunk.len() / 2;
            l2_pos += chunk.len();

            l1_boundaries
                .iter()
                .find(|(end, _)| chunk_mid < *end)
                .map(|(_, idx)| *idx)
                .or_else(|| l1_boundaries.last().map(|(_, idx)| *idx))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_doc() -> String {
        "This is the introduction to the document.\n\nSection one covers basics of Rust.\nRust is a systems programming language.\n\nSection two covers advanced topics.\nAdvanced topics include async and unsafe.".to_string()
    }

    fn test_metadata() -> ChunkMetadata {
        ChunkMetadata {
            source: "https://example.com/doc".to_string(),
            domain: "rust".to_string(),
            title: "Test Document".to_string(),
            section_index: 0,
            ..Default::default()
        }
    }

    /// T-KB-001-01: L0 produces a single summary chunk.
    #[test]
    fn test_chunking_l0_produces_summary() {
        let strategy = ChunkingStrategy::new(KnowledgeConfig::default());
        let chunks = strategy.chunk("doc-1", &test_doc(), ChunkLevel::L0, &test_metadata());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].level, ChunkLevel::L0);
    }

    /// T-KB-001-02: L1 produces section-level chunks.
    #[test]
    fn test_chunking_l1_produces_sections() {
        let strategy = ChunkingStrategy::new(KnowledgeConfig::default());
        let chunks = strategy.chunk("doc-1", &test_doc(), ChunkLevel::L1, &test_metadata());
        assert!(
            chunks.len() >= 3,
            "expected at least 3 sections, got {}",
            chunks.len()
        );
        assert!(chunks.iter().all(|c| c.level == ChunkLevel::L1));
    }

    /// T-KB-001-03: L2 produces paragraph-level chunks.
    #[test]
    fn test_chunking_l2_produces_full() {
        let strategy = ChunkingStrategy::new(KnowledgeConfig::default());
        let chunks = strategy.chunk("doc-1", &test_doc(), ChunkLevel::L2, &test_metadata());
        assert!(
            chunks.len() >= 5,
            "expected at least 5 paragraphs, got {}",
            chunks.len()
        );
        assert!(chunks.iter().all(|c| c.level == ChunkLevel::L2));
    }

    /// T-KB-001-04: Each chunk respects max tokens.
    #[test]
    fn test_chunking_respects_max_tokens() {
        let config = KnowledgeConfig {
            l0_max_tokens: 10, // Very small
            ..Default::default()
        };
        let strategy = ChunkingStrategy::new(config);
        let long_text = "x".repeat(1000);
        let chunks = strategy.chunk("doc-1", &long_text, ChunkLevel::L0, &test_metadata());
        assert!(chunks[0].token_estimate <= 11, "chunk exceeds max tokens");
    }

    /// T-KB-001-05: Metadata is preserved on each chunk.
    #[test]
    fn test_chunking_preserves_metadata() {
        let strategy = ChunkingStrategy::new(KnowledgeConfig::default());
        let meta = test_metadata();
        let chunks = strategy.chunk("doc-1", &test_doc(), ChunkLevel::L1, &meta);
        assert!(chunks.iter().all(|c| c.metadata.domain == "rust"));
        assert!(chunks
            .iter()
            .all(|c| c.metadata.source == "https://example.com/doc"));
    }

    // --- Sentence Boundary Chunker tests ---

    /// Sentence boundary chunker splits on punctuation.
    #[test]
    fn test_sentence_boundary_splits_on_punctuation() {
        let config = KnowledgeConfig::default();
        let strategy = ChunkingStrategy::with_chunker(config, ChunkerType::SentenceBoundary);
        let text = "First sentence. Second sentence! Third sentence?";
        let chunks = strategy.chunk("doc-1", text, ChunkLevel::L1, &test_metadata());
        assert!(!chunks.is_empty(), "should produce chunks");
        // All content should be present.
        let total_len: usize = chunks.iter().map(|c| c.content.len()).sum();
        assert!(total_len > 0);
    }

    /// Sentence boundary chunker handles Chinese punctuation.
    #[test]
    fn test_sentence_boundary_chinese_punctuation() {
        let text = "第一句话。第二句话！第三句话？最后一句。";
        let sentences = split_by_sentence_boundary(text, 100);
        assert!(!sentences.is_empty());
        // All original text should be covered.
        let joined: String = sentences.join(" ");
        assert!(joined.contains("第一句话"));
        assert!(joined.contains("最后一句"));
    }

    /// Sentence boundary chunker merges small fragments.
    #[test]
    fn test_sentence_boundary_merges_small() {
        let text = "A. B. C. D. E.";
        let chunks = split_by_sentence_boundary(text, 100);
        // Small fragments should be merged into fewer chunks.
        assert!(
            chunks.len() < 5,
            "expected merging, got {} chunks",
            chunks.len()
        );
    }

    // --- Heading Based Chunker tests ---

    /// Heading chunker splits on Markdown headings.
    #[test]
    fn test_heading_chunker_splits_on_headings() {
        let text = "Intro paragraph.\n\n# Section 1\n\nContent of section 1.\n\n## Section 1.1\n\nSubsection content.\n\n# Section 2\n\nContent of section 2.";
        let config = KnowledgeConfig::default();
        let strategy = ChunkingStrategy::with_chunker(config, ChunkerType::HeadingBased);
        let chunks = strategy.chunk("doc-1", text, ChunkLevel::L1, &test_metadata());
        assert!(
            chunks.len() >= 3,
            "expected at least 3 heading sections, got {}",
            chunks.len()
        );
    }

    /// Heading chunker preserves content before first heading.
    #[test]
    fn test_heading_chunker_preserves_intro() {
        let text = "This is before any heading.\n\n# First Heading\n\nContent.";
        let sections = split_by_headings(text);
        assert!(sections.len() >= 2);
        assert!(sections[0].contains("before any heading"));
        assert!(sections[1].contains("First Heading"));
    }

    /// Heading chunker handles document with no headings.
    #[test]
    fn test_heading_chunker_no_headings() {
        let text = "Just a plain paragraph without any headings.";
        let sections = split_by_headings(text);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].contains("plain paragraph"));
    }

    // --- ChunkerType tests ---

    /// `with_chunker` creates correct strategy.
    #[test]
    fn test_with_chunker() {
        let config = KnowledgeConfig::default();
        let strategy = ChunkingStrategy::with_chunker(config, ChunkerType::HeadingBased);
        assert_eq!(strategy.chunker_type, ChunkerType::HeadingBased);
    }

    /// Chunk serialization/deserialization roundtrip.
    #[test]
    fn test_chunk_serialization() {
        let chunk = Chunk {
            id: "test-1".to_string(),
            document_id: "doc-1".to_string(),
            level: ChunkLevel::L1,
            content: "test content".to_string(),
            token_estimate: 3,
            metadata: test_metadata(),
        };
        let json = serde_json::to_string(&chunk).expect("serialize");
        let deserialized: Chunk = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.id, "test-1");
        assert_eq!(deserialized.level, ChunkLevel::L1);
    }

    // --- extract_section_title tests ---

    #[test]
    fn test_extract_section_title_h1() {
        assert_eq!(extract_section_title("# Hello\nsome content", 0), "Hello");
    }

    #[test]
    fn test_extract_section_title_h2() {
        assert_eq!(
            extract_section_title("## Sub Heading\ncontent here", 0),
            "Sub Heading"
        );
    }

    #[test]
    fn test_extract_section_title_fallback() {
        assert_eq!(
            extract_section_title("plain text without heading", 0),
            "Section 1"
        );
        assert_eq!(extract_section_title("plain text", 2), "Section 3");
    }

    #[test]
    fn test_extract_section_title_empty() {
        assert_eq!(extract_section_title("", 0), "Section 1");
    }

    #[test]
    fn test_extract_section_title_blank_heading() {
        // A line that is just "##" with no title text
        assert_eq!(extract_section_title("##\ncontent", 0), "Section 1");
    }

    // --- is_generic_section_title tests ---

    #[test]
    fn test_is_generic_section_title_matches() {
        assert!(is_generic_section_title("Section 1"));
        assert!(is_generic_section_title("Section 16"));
        assert!(is_generic_section_title("Section 100"));
        // Whitespace tolerance.
        assert!(is_generic_section_title("  Section 3  "));
    }

    #[test]
    fn test_is_generic_section_title_rejects_real_titles() {
        assert!(!is_generic_section_title("Introduction"));
        assert!(!is_generic_section_title("Result Type"));
        assert!(!is_generic_section_title("The ? Operator"));
        // Not just "Section N" pattern.
        assert!(!is_generic_section_title("Section Overview"));
        assert!(!is_generic_section_title("Section"));
        assert!(!is_generic_section_title("Section 1.1"));
        assert!(!is_generic_section_title(""));
    }

    // --- L1 alignment tests ---

    #[test]
    fn test_l1_alignment_empty_sections() {
        let chunks = vec!["hello world".to_string(), "foo bar".to_string()];
        let result = compute_l1_alignment(&[], &chunks);
        assert_eq!(result, vec![None, None]);
    }

    #[test]
    fn test_l1_alignment_empty_chunks() {
        let sections = vec![(0, 100), (1, 200)];
        let result = compute_l1_alignment(&sections, &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_l1_alignment_single_section() {
        let sections = vec![(0, 500)];
        let chunks = vec!["a".repeat(100), "b".repeat(200), "c".repeat(200)];
        let result = compute_l1_alignment(&sections, &chunks);
        // All chunks should map to section 0.
        assert_eq!(result, vec![Some(0), Some(0), Some(0)]);
    }

    #[test]
    fn test_l1_alignment_multi_section() {
        // 3 sections of 100, 200, 100 chars.
        let sections = vec![(0, 100), (1, 200), (2, 100)];
        // 4 chunks: first in section 0, next two in section 1, last in section 2.
        let chunks = vec![
            "a".repeat(100), // 0..100 -> midpoint 50 -> section 0
            "b".repeat(100), // 100..200 -> midpoint 150 -> section 1
            "c".repeat(100), // 200..300 -> midpoint 250 -> section 1
            "d".repeat(100), // 300..400 -> midpoint 350 -> section 2
        ];
        let result = compute_l1_alignment(&sections, &chunks);
        assert_eq!(result, vec![Some(0), Some(1), Some(1), Some(2)]);
    }

    #[test]
    fn test_l1_alignment_chunks_exceed_sections() {
        // Section covers 100 chars, but chunks total 200.
        let sections = vec![(0, 100)];
        let chunks = vec!["a".repeat(100), "b".repeat(100)];
        let result = compute_l1_alignment(&sections, &chunks);
        // Both should map to section 0 (last boundary fallback).
        assert_eq!(result, vec![Some(0), Some(0)]);
    }

    // --- ChunkLevel::from_resolution tests ---

    #[test]
    fn test_chunk_level_from_resolution() {
        assert_eq!(ChunkLevel::from_resolution("l0"), Some(ChunkLevel::L0));
        assert_eq!(ChunkLevel::from_resolution("L0"), Some(ChunkLevel::L0));
        assert_eq!(ChunkLevel::from_resolution("l1"), Some(ChunkLevel::L1));
        assert_eq!(ChunkLevel::from_resolution("l2"), Some(ChunkLevel::L2));
        assert_eq!(ChunkLevel::from_resolution("L2"), Some(ChunkLevel::L2));
        assert_eq!(ChunkLevel::from_resolution("all"), None);
        assert_eq!(ChunkLevel::from_resolution(""), None);
    }
}
