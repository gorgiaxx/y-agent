//! Progressive loader: L0 → L1 → L2 on-demand loading with token budget.

use crate::chunking::{Chunk, ChunkLevel, ChunkMetadata, ChunkingStrategy};
use crate::config::KnowledgeConfig;

/// Progressively loads knowledge at increasing resolution levels.
///
/// Starts with L0 summaries. Users can drill down to L1 sections
/// and L2 full content on demand, all within a token budget.
#[derive(Debug)]
pub struct ProgressiveLoader {
    strategy: ChunkingStrategy,
}

impl ProgressiveLoader {
    pub fn new(config: KnowledgeConfig) -> Self {
        Self {
            strategy: ChunkingStrategy::new(config),
        }
    }

    /// Load chunks at L0 level (summaries).
    pub fn load_l0(
        &self,
        document_id: &str,
        content: &str,
        metadata: &ChunkMetadata,
    ) -> Vec<Chunk> {
        self.strategy
            .chunk(document_id, content, ChunkLevel::L0, metadata)
    }

    /// Load chunks at L1 level (sections).
    pub fn load_l1(
        &self,
        document_id: &str,
        content: &str,
        metadata: &ChunkMetadata,
    ) -> Vec<Chunk> {
        self.strategy
            .chunk(document_id, content, ChunkLevel::L1, metadata)
    }

    /// Load chunks at L2 level (full detail).
    pub fn load_l2(
        &self,
        document_id: &str,
        content: &str,
        metadata: &ChunkMetadata,
    ) -> Vec<Chunk> {
        self.strategy
            .chunk(document_id, content, ChunkLevel::L2, metadata)
    }

    /// Load chunks within a token budget, starting from L0 and upgrading.
    pub fn load_within_budget(
        &self,
        document_id: &str,
        content: &str,
        metadata: &ChunkMetadata,
        budget_tokens: u32,
    ) -> Vec<Chunk> {
        // Try L0 first
        let l0 = self.load_l0(document_id, content, metadata);
        let l0_total: u32 = l0.iter().map(|c| c.token_estimate).sum();
        if l0_total > budget_tokens {
            // L0 already exceeds budget — return truncated
            return Self::fit_budget(l0, budget_tokens);
        }

        // Try L1
        let l1 = self.load_l1(document_id, content, metadata);
        let l1_total: u32 = l1.iter().map(|c| c.token_estimate).sum();
        if l1_total <= budget_tokens {
            // L1 fits — try L2
            let l2 = self.load_l2(document_id, content, metadata);
            let l2_total: u32 = l2.iter().map(|c| c.token_estimate).sum();
            if l2_total <= budget_tokens {
                return l2;
            }
            return Self::fit_budget(l2, budget_tokens);
        }

        // L1 doesn't fully fit — return what fits
        Self::fit_budget(l1, budget_tokens)
    }

    /// Take chunks until the budget is exhausted.
    fn fit_budget(chunks: Vec<Chunk>, budget: u32) -> Vec<Chunk> {
        let mut total = 0u32;
        chunks
            .into_iter()
            .take_while(|c| {
                total += c.token_estimate;
                total <= budget
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_content() -> String {
        "Summary of document.\n\nSection 1 content here.\n\nSection 2 content here.".to_string()
    }

    fn test_metadata() -> ChunkMetadata {
        ChunkMetadata {
            source: "test".to_string(),
            domain: "test".to_string(),
            title: "Test".to_string(),
            section_index: 0,
        }
    }

    /// T-KB-002-01: Initial query returns L0 summaries.
    #[test]
    fn test_progressive_l0_first() {
        let loader = ProgressiveLoader::new(KnowledgeConfig::default());
        let chunks = loader.load_l0("doc-1", &test_content(), &test_metadata());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].level, ChunkLevel::L0);
    }

    /// T-KB-002-02: Detail request returns L1 sections.
    #[test]
    fn test_progressive_l1_on_demand() {
        let loader = ProgressiveLoader::new(KnowledgeConfig::default());
        let chunks = loader.load_l1("doc-1", &test_content(), &test_metadata());
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.level == ChunkLevel::L1));
    }

    /// T-KB-002-03: Deep dive returns L2 full content.
    #[test]
    fn test_progressive_l2_full() {
        let loader = ProgressiveLoader::new(KnowledgeConfig::default());
        let chunks = loader.load_l2("doc-1", &test_content(), &test_metadata());
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.level == ChunkLevel::L2));
    }

    /// T-KB-002-04: Budget constraint is respected.
    #[test]
    fn test_progressive_token_budget() {
        let loader = ProgressiveLoader::new(KnowledgeConfig::default());
        let content = "Section A.\n\nSection B full content with many more words.\n\nSection C.";
        let chunks = loader.load_within_budget("doc-1", content, &test_metadata(), 500);
        let total: u32 = chunks.iter().map(|c| c.token_estimate).sum();
        assert!(total <= 500, "total {total} exceeds budget 500");
    }
}
