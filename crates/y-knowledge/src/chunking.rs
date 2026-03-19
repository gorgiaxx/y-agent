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
    /// Document title.
    pub title: String,
    /// Section index within the document.
    pub section_index: usize,
}

/// Chunking algorithm type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkerType {
    /// Simple newline-based splitting (legacy default).
    TextSplit,
    /// Punctuation-aware sentence boundary splitting (MaxKB-inspired).
    SentenceBoundary,
    /// Markdown heading-based splitting.
    HeadingBased,
}

impl Default for ChunkerType {
    fn default() -> Self {
        Self::TextSplit
    }
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
    fn chunk_l2(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = tokens_to_max_chars(self.config.l2_max_tokens, content);

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
        } else if current.len() + 1 + fragment.len() <= max_chars {
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
fn estimate_tokens(text: &str) -> u32 {
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
/// Samples the first 200 characters of `content` to estimate the CJK ratio,
/// then uses that ratio to compute how many characters fit within `max_tokens`.
fn tokens_to_max_chars(max_tokens: u32, content: &str) -> usize {
    // Sample the beginning to estimate CJK ratio.
    let sample: String = content.chars().take(200).collect();
    let total = sample.chars().count().max(1);
    let cjk = sample.chars().filter(|c| is_cjk_char(*c)).count();
    let cjk_ratio = cjk as f64 / total as f64;

    // Weighted chars-per-token: CJK ≈ 0.67 chars/token, Latin ≈ 4.0 chars/token.
    let chars_per_token = cjk_ratio * 0.67 + (1.0 - cjk_ratio) * 4.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_chars = (max_tokens as f64 * chars_per_token) as usize;
    max_chars
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

    /// with_chunker creates correct strategy.
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
}
