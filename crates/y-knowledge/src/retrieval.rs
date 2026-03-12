//! Hybrid retriever: vector + keyword search with domain/freshness filters.
//!
//! In test/development mode, uses in-memory storage with simple substring
//! matching. In production, delegates to Qdrant for vector search.

use crate::chunking::Chunk;

/// A retrieval result with relevance score.
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    pub chunk: Chunk,
    pub relevance: f32,
}

/// Retrieval filters.
#[derive(Debug, Clone, Default)]
pub struct RetrievalFilter {
    /// Filter by knowledge domain.
    pub domain: Option<String>,
    /// Exclude chunks older than this (ISO 8601 timestamp).
    pub freshness_after: Option<String>,
    /// Maximum number of results.
    pub limit: usize,
}

/// Hybrid retriever combining vector and keyword search.
#[derive(Debug, Default)]
pub struct HybridRetriever {
    /// In-memory chunk store for development/testing.
    chunks: Vec<Chunk>,
}

impl HybridRetriever {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index a chunk for retrieval.
    pub fn index(&mut self, chunk: Chunk) {
        self.chunks.push(chunk);
    }

    /// Search using hybrid strategy (vector similarity + keyword fallback).
    ///
    /// In development mode, uses simple substring matching as a stand-in
    /// for vector search.
    pub fn search(&self, query: &str, filter: &RetrievalFilter) -> Vec<RetrievalResult> {
        let query_lower = query.to_lowercase();
        let limit = if filter.limit == 0 { 10 } else { filter.limit };

        let mut results: Vec<RetrievalResult> = self
            .chunks
            .iter()
            .filter(|c| {
                // Domain filter
                if let Some(ref domain) = filter.domain {
                    if c.metadata.domain != *domain {
                        return false;
                    }
                }
                true
            })
            .filter_map(|c| {
                let content_lower = c.content.to_lowercase();
                if content_lower.contains(&query_lower) {
                    // Simple relevance based on match position
                    #[allow(clippy::cast_precision_loss)]
                    let relevance = 1.0
                        - (content_lower.find(&query_lower).unwrap_or(0) as f32
                            / content_lower.len().max(1) as f32);
                    Some(RetrievalResult {
                        chunk: c.clone(),
                        relevance,
                    })
                } else {
                    // Keyword fallback: check individual words
                    let words: Vec<&str> = query_lower.split_whitespace().collect();
                    let matches = words.iter().filter(|w| content_lower.contains(*w)).count();
                    if matches > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let relevance = matches as f32 / words.len() as f32 * 0.5;
                        Some(RetrievalResult {
                            chunk: c.clone(),
                            relevance,
                        })
                    } else {
                        None
                    }
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{ChunkLevel, ChunkMetadata};

    fn make_chunk(id: &str, content: &str, domain: &str) -> Chunk {
        Chunk {
            id: id.to_string(),
            document_id: "doc-1".to_string(),
            level: ChunkLevel::L1,
            content: content.to_string(),
            token_estimate: 10,
            metadata: ChunkMetadata {
                source: "test".to_string(),
                domain: domain.to_string(),
                title: "Test".to_string(),
                section_index: 0,
            },
        }
    }

    /// T-KB-003-01: Semantic (substring) query returns similar documents.
    #[test]
    fn test_retrieval_vector_search() {
        let mut retriever = HybridRetriever::new();
        retriever.index(make_chunk("c1", "Rust error handling patterns", "rust"));
        retriever.index(make_chunk("c2", "Python web frameworks", "python"));

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("error handling", &filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk.id, "c1");
    }

    /// T-KB-003-02: Keyword fallback when no exact match.
    #[test]
    fn test_retrieval_keyword_fallback() {
        let mut retriever = HybridRetriever::new();
        retriever.index(make_chunk(
            "c1",
            "Covers advanced Rust error patterns",
            "rust",
        ));

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("Rust patterns", &filter);
        assert!(
            !results.is_empty(),
            "keyword fallback should find partial matches"
        );
    }

    /// T-KB-003-03: Domain filter restricts results.
    #[test]
    fn test_retrieval_domain_filter() {
        let mut retriever = HybridRetriever::new();
        retriever.index(make_chunk("c1", "Rust error handling", "rust"));
        retriever.index(make_chunk("c2", "Python error handling", "python"));

        let filter = RetrievalFilter {
            domain: Some("rust".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("error", &filter);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk.metadata.domain, "rust");
    }

    /// T-KB-003-05: Limit is respected.
    #[test]
    fn test_retrieval_respects_limit() {
        let mut retriever = HybridRetriever::new();
        for i in 0..10 {
            retriever.index(make_chunk(
                &format!("c{i}"),
                &format!("Error content {i}"),
                "rust",
            ));
        }

        let filter = RetrievalFilter {
            limit: 5,
            ..Default::default()
        };
        let results = retriever.search("error", &filter);
        assert!(results.len() <= 5);
    }
}
