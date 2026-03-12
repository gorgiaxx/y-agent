//! L0/L1/L2 multi-resolution chunking strategy.
//!
//! - **L0**: Summary-level chunks (one per document, < 200 tokens)
//! - **L1**: Section-level chunks (one per major section, < 500 tokens)
//! - **L2**: Paragraph-level chunks (granular, < 1000 tokens)

use crate::config::KnowledgeConfig;

/// Resolution level for chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkLevel {
    /// Summary: one chunk per document, compact overview.
    L0,
    /// Section: one chunk per major section.
    L1,
    /// Paragraph: granular, full-detail chunks.
    L2,
}

/// A chunk of knowledge content at a specific resolution level.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone, Default)]
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

/// Chunks documents at multiple resolution levels.
#[derive(Debug)]
pub struct ChunkingStrategy {
    config: KnowledgeConfig,
}

impl ChunkingStrategy {
    pub fn new(config: KnowledgeConfig) -> Self {
        Self { config }
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
        let max_chars = (self.config.l0_max_tokens * 4) as usize;
        let summary = if content.len() > max_chars {
            &content[..max_chars]
        } else {
            content
        };

        vec![Chunk {
            id: format!("{document_id}-L0-0"),
            document_id: document_id.to_string(),
            level: ChunkLevel::L0,
            content: summary.to_string(),
            token_estimate: estimate_tokens(summary),
            metadata: metadata.clone(),
        }]
    }

    /// L1: Split by double newlines (sections).
    fn chunk_l1(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = (self.config.l1_max_tokens * 4) as usize;

        content
            .split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(i, section)| {
                let text = if section.len() > max_chars {
                    &section[..max_chars]
                } else {
                    section
                };
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

    /// L2: Split by single newlines (paragraphs).
    fn chunk_l2(&self, document_id: &str, content: &str, metadata: &ChunkMetadata) -> Vec<Chunk> {
        let max_chars = (self.config.l2_max_tokens * 4) as usize;

        content
            .split('\n')
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(i, para)| {
                let text = if para.len() > max_chars {
                    &para[..max_chars]
                } else {
                    para
                };
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

fn estimate_tokens(text: &str) -> u32 {
    let chars = u32::try_from(text.len()).unwrap_or(u32::MAX);
    chars.div_ceil(4)
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
}
