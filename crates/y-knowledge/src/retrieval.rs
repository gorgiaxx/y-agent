//! Hybrid retriever: vector + BM25 keyword search with blend fusion.
//!
//! Supports three search strategies:
//! - **`SemanticSearch`**: Vector similarity only
//! - **`KeywordSearch`**: BM25 keyword search only
//! - **`Hybrid`**: Blend Search fusion `(1 - cosine_distance) + bm25_score`
//!
//! Features: paragraph-level dedup, `min_similarity_threshold`, quality/freshness boosts.

use crate::bm25::Bm25Index;
use crate::chunking::Chunk;
use crate::tokenizer::Tokenizer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Search Strategy
// ---------------------------------------------------------------------------

/// Search strategy for retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchStrategy {
    /// Vector similarity search only.
    SemanticSearch,
    /// BM25 keyword search only.
    KeywordSearch,
    /// Blend Search: additive fusion of vector + BM25. (`MaxKB`-inspired)
    #[default]
    Hybrid,
}

// ---------------------------------------------------------------------------
// Retrieval Config
// ---------------------------------------------------------------------------

/// Configuration for retrieval operations.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    /// Search strategy.
    pub strategy: SearchStrategy,
    /// Minimum similarity threshold (0.0–1.0). Results below are discarded.
    pub min_similarity_threshold: f32,
    /// Decay rate for freshness boost.
    pub freshness_decay_rate: f64,
    /// Whether to enable paragraph-level dedup.
    pub enable_dedup: bool,
    /// Weight for BM25 score in blend fusion (default: 1.0).
    pub bm25_weight: f64,
    /// Weight for vector score in blend fusion (default: 1.0).
    pub vector_weight: f64,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            strategy: SearchStrategy::Hybrid,
            min_similarity_threshold: 0.65,
            freshness_decay_rate: 0.1,
            enable_dedup: true,
            bm25_weight: 1.0,
            vector_weight: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Retrieval Result
// ---------------------------------------------------------------------------

/// A retrieval result with relevance score and component breakdowns.
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    /// The matched chunk.
    pub chunk: Chunk,
    /// Final blended relevance score.
    pub relevance: f32,
    /// Vector similarity component (if applicable).
    pub vector_score: Option<f32>,
    /// BM25 keyword score component (if applicable).
    pub bm25_score: Option<f64>,
}

// ---------------------------------------------------------------------------
// Retrieval Filter
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Hybrid Retriever
// ---------------------------------------------------------------------------

/// Hybrid retriever combining vector similarity and BM25 keyword search.
///
/// In development/test mode, uses in-memory chunk store.
/// Supports blend search fusion, paragraph-level dedup, and quality/freshness boosts.
#[derive(Debug)]
pub struct HybridRetriever<T: Tokenizer> {
    /// In-memory chunk store.
    chunks: Vec<Chunk>,
    /// BM25 keyword index.
    bm25: Bm25Index<T>,
    /// Retrieval configuration.
    config: RetrievalConfig,
    /// Quality scores per `chunk_id`.
    quality_scores: HashMap<String, f32>,
    /// In-memory embedding vectors per `chunk_id`.
    embeddings: HashMap<String, Vec<f32>>,
}

impl<T: Tokenizer> HybridRetriever<T> {
    /// Create a new retriever with a tokenizer and default config.
    pub fn new(tokenizer: T) -> Self {
        Self {
            chunks: Vec::new(),
            bm25: Bm25Index::new(tokenizer),
            config: RetrievalConfig::default(),
            quality_scores: HashMap::new(),
            embeddings: HashMap::new(),
        }
    }

    /// Create a new retriever with custom config.
    pub fn with_config(tokenizer: T, config: RetrievalConfig) -> Self {
        Self {
            chunks: Vec::new(),
            bm25: Bm25Index::new(tokenizer),
            config,
            quality_scores: HashMap::new(),
            embeddings: HashMap::new(),
        }
    }

    /// Index a chunk for retrieval.
    pub fn index(&mut self, chunk: Chunk) {
        self.bm25.add(&chunk.id, &chunk.content);
        self.chunks.push(chunk);
    }

    /// Index a chunk with an associated quality score.
    pub fn index_with_quality(&mut self, chunk: Chunk, quality_score: f32) {
        self.quality_scores.insert(chunk.id.clone(), quality_score);
        self.index(chunk);
    }

    /// Remove all chunks belonging to a document from the retriever.
    ///
    /// Cleans up the chunks vec, BM25 index, quality scores, and embeddings.
    /// Uses bulk removal for the BM25 index to avoid O(chunks × terms) scanning.
    /// Returns the number of chunks removed.
    pub fn remove_by_document(&mut self, document_id: &str) -> usize {
        // Collect chunk IDs into a HashSet for O(1) lookups.
        let chunk_ids: std::collections::HashSet<String> = self
            .chunks
            .iter()
            .filter(|c| c.document_id == document_id)
            .map(|c| c.id.clone())
            .collect();

        let removed = chunk_ids.len();
        if removed == 0 {
            return 0;
        }

        // Bulk-remove from BM25 index (single-pass over inverted index).
        self.bm25.remove_bulk(&chunk_ids);

        // Remove quality scores and embeddings.
        for id in &chunk_ids {
            self.quality_scores.remove(id);
            self.embeddings.remove(id);
        }

        // Remove from chunks vec.
        self.chunks.retain(|c| c.document_id != document_id);

        removed
    }

    /// Index a chunk with an embedding vector and quality score.
    pub fn index_with_embedding(&mut self, chunk: Chunk, embedding: Vec<f32>, quality_score: f32) {
        self.embeddings.insert(chunk.id.clone(), embedding);
        self.index_with_quality(chunk, quality_score);
    }

    /// Index multiple chunks with quality scores in a single call.
    ///
    /// More efficient than calling [`index_with_quality`] in a loop:
    /// pre-reserves capacity and uses bulk BM25 indexing to reduce
    /// allocator pressure and per-chunk overhead.
    pub fn index_batch_with_quality(&mut self, chunks: Vec<Chunk>, quality_score: f32) {
        self.chunks.reserve(chunks.len());

        // Build BM25 batch.
        let bm25_batch: Vec<(&str, &str)> = chunks
            .iter()
            .map(|c| (c.id.as_str(), c.content.as_str()))
            .collect();
        self.bm25.add_bulk(&bm25_batch);

        // Store chunks and quality scores.
        for chunk in chunks {
            self.quality_scores.insert(chunk.id.clone(), quality_score);
            self.chunks.push(chunk);
        }
    }

    /// Index multiple chunks with embeddings and quality scores in a single call.
    pub fn index_batch_with_embeddings(
        &mut self,
        chunks: Vec<Chunk>,
        embeddings: Vec<Vec<f32>>,
        quality_score: f32,
    ) {
        self.chunks.reserve(chunks.len());

        let bm25_batch: Vec<(&str, &str)> = chunks
            .iter()
            .map(|c| (c.id.as_str(), c.content.as_str()))
            .collect();
        self.bm25.add_bulk(&bm25_batch);

        for (chunk, embedding) in chunks.into_iter().zip(embeddings) {
            self.embeddings.insert(chunk.id.clone(), embedding);
            self.quality_scores.insert(chunk.id.clone(), quality_score);
            self.chunks.push(chunk);
        }
    }

    /// Check whether the retriever has any stored embeddings.
    pub fn has_embeddings(&self) -> bool {
        !self.embeddings.is_empty()
    }

    /// Get a reference to all stored embeddings (for persistence).
    pub fn embeddings(&self) -> &HashMap<String, Vec<f32>> {
        &self.embeddings
    }

    /// Search using the configured strategy.
    pub fn search(&self, query: &str, filter: &RetrievalFilter) -> Vec<RetrievalResult> {
        self.search_with_embedding(query, None, filter)
    }

    /// Search with an optional query embedding for real vector similarity.
    ///
    /// When `query_embedding` is `Some` and stored chunk embeddings exist,
    /// semantic search uses cosine similarity instead of text matching.
    pub fn search_with_embedding(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        filter: &RetrievalFilter,
    ) -> Vec<RetrievalResult> {
        let limit = if filter.limit == 0 { 10 } else { filter.limit };

        let mut results = match self.config.strategy {
            SearchStrategy::KeywordSearch => self.keyword_search(query, filter),
            SearchStrategy::SemanticSearch => {
                self.semantic_search_with_embedding(query, query_embedding, filter)
            }
            SearchStrategy::Hybrid => {
                self.blend_search_with_embedding(query, query_embedding, filter)
            }
        };

        // Apply quality boost.
        for result in &mut results {
            if let Some(&quality) = self.quality_scores.get(&result.chunk.id) {
                let boost = quality.sqrt();
                result.relevance *= boost;
            }
        }

        // Apply min similarity threshold.
        results.retain(|r| r.relevance >= self.config.min_similarity_threshold);

        // Paragraph-level dedup: keep highest score per (document_id, section_index).
        if self.config.enable_dedup {
            results = Self::dedup_by_section(results);
            // Content-level dedup: remove near-duplicate content across documents.
            results = Self::dedup_by_content(results);
        }

        // Sort by score descending.
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(limit);
        results
    }

    /// BM25 keyword search only.
    fn keyword_search(&self, query: &str, filter: &RetrievalFilter) -> Vec<RetrievalResult> {
        let bm25_results = self.bm25.search(query, 100);
        let bm25_map: HashMap<&str, f64> = bm25_results
            .iter()
            .map(|r| (r.chunk_id.as_str(), r.score))
            .collect();

        self.chunks
            .iter()
            .filter(|c| Self::matches_filter(c, filter))
            .filter_map(|c| {
                bm25_map.get(c.id.as_str()).map(|&score| RetrievalResult {
                    chunk: c.clone(),
                    relevance: score as f32,
                    vector_score: None,
                    bm25_score: Some(score),
                })
            })
            .collect()
    }

    /// Semantic (vector) search with an optional query embedding vector.
    ///
    /// When a `query_embedding` is provided and chunk embeddings exist, uses
    /// real cosine similarity. Otherwise falls back to text similarity.
    fn semantic_search_with_embedding(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        filter: &RetrievalFilter,
    ) -> Vec<RetrievalResult> {
        let query_lower = query.to_lowercase();

        self.chunks
            .iter()
            .filter(|c| Self::matches_filter(c, filter))
            .filter_map(|c| {
                let score =
                    if let (Some(qe), Some(ce)) = (query_embedding, self.embeddings.get(&c.id)) {
                        cosine_similarity(qe, ce)
                    } else {
                        let content_lower = c.content.to_lowercase();
                        Self::compute_text_similarity(&query_lower, &content_lower)
                    };

                if score > 0.0 {
                    Some(RetrievalResult {
                        chunk: c.clone(),
                        relevance: score,
                        vector_score: Some(score),
                        bm25_score: None,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Blend search: additive fusion of semantic + BM25 scores.
    ///
    /// `final_score = vector_weight * semantic_score + bm25_weight * normalized_bm25_score`
    ///
    /// When `query_embedding` is provided and chunk embeddings exist, uses
    /// real cosine similarity for the semantic component.
    fn blend_search_with_embedding(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        filter: &RetrievalFilter,
    ) -> Vec<RetrievalResult> {
        let query_lower = query.to_lowercase();

        // Get BM25 scores.
        let bm25_results = self.bm25.search(query, 100);
        let bm25_map: HashMap<&str, f64> = bm25_results
            .iter()
            .map(|r| (r.chunk_id.as_str(), r.score))
            .collect();

        // Normalize BM25 scores to 0-1 range.
        let max_bm25 = bm25_results.iter().map(|r| r.score).fold(0.0_f64, f64::max);

        self.chunks
            .iter()
            .filter(|c| Self::matches_filter(c, filter))
            .filter_map(|c| {
                // Semantic score: use cosine similarity when embeddings available.
                let semantic =
                    if let (Some(qe), Some(ce)) = (query_embedding, self.embeddings.get(&c.id)) {
                        cosine_similarity(qe, ce)
                    } else {
                        let content_lower = c.content.to_lowercase();
                        Self::compute_text_similarity(&query_lower, &content_lower)
                    };

                // BM25 score (normalized).
                let bm25 = bm25_map.get(c.id.as_str()).copied().unwrap_or(0.0);
                let bm25_normalized = if max_bm25 > 0.0 { bm25 / max_bm25 } else { 0.0 };

                let blended = (self.config.vector_weight * f64::from(semantic)
                    + self.config.bm25_weight * bm25_normalized)
                    as f32;

                if blended > 0.0 {
                    Some(RetrievalResult {
                        chunk: c.clone(),
                        relevance: blended,
                        vector_score: Some(semantic),
                        bm25_score: Some(bm25),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Compute text similarity (development substitute for vector cosine similarity).
    fn compute_text_similarity(query: &str, content: &str) -> f32 {
        if content.contains(query) {
            // Exact substring match — high score.
            1.0
        } else {
            // Word overlap score.
            let query_words: Vec<&str> = query.split_whitespace().collect();
            if query_words.is_empty() {
                return 0.0;
            }
            let matches = query_words.iter().filter(|w| content.contains(**w)).count();

            let score = matches as f32 / query_words.len() as f32;
            score * 0.8 // Scale down partial matches.
        }
    }

    /// Check if a chunk matches the given filter.
    fn matches_filter(chunk: &Chunk, filter: &RetrievalFilter) -> bool {
        if let Some(ref domain) = filter.domain {
            if chunk.metadata.domain != *domain {
                return false;
            }
        }
        true
    }

    /// Paragraph-level dedup: keep highest score per (`document_id`, `section_index`).
    ///
    /// Inspired by `MaxKB` DISTINCT ON approach.
    fn dedup_by_section(results: Vec<RetrievalResult>) -> Vec<RetrievalResult> {
        let mut best: HashMap<(String, usize), RetrievalResult> = HashMap::new();

        for result in results {
            let key = (
                result.chunk.document_id.clone(),
                result.chunk.metadata.section_index,
            );

            match best.get(&key) {
                Some(existing) if existing.relevance >= result.relevance => {
                    // Keep existing higher score.
                }
                _ => {
                    best.insert(key, result);
                }
            }
        }

        best.into_values().collect()
    }

    /// Content-level dedup: remove near-duplicate chunks across different documents.
    ///
    /// Computes a simplified content fingerprint (first N chars, lowercased,
    /// whitespace-normalised) and keeps only the highest-scoring result per
    /// fingerprint. This catches the common case where the same passage
    /// appears in multiple ingested files.
    fn dedup_by_content(results: Vec<RetrievalResult>) -> Vec<RetrievalResult> {
        /// Number of characters to use for fingerprinting.
        const FINGERPRINT_LEN: usize = 100;

        fn fingerprint(text: &str) -> String {
            text.chars()
                .filter(|c| !c.is_whitespace())
                .flat_map(char::to_lowercase)
                .take(FINGERPRINT_LEN)
                .collect()
        }

        let mut best: HashMap<String, RetrievalResult> = HashMap::new();

        for result in results {
            let fp = fingerprint(&result.chunk.content);
            match best.get(&fp) {
                Some(existing) if existing.relevance >= result.relevance => {
                    // Keep existing higher score.
                }
                _ => {
                    best.insert(fp, result);
                }
            }
        }

        best.into_values().collect()
    }

    /// Retrieve neighboring chunks for context window expansion.
    ///
    /// Given a chunk ID, returns the matched chunk plus surrounding chunks
    /// from the same document, ordered by `section_index`. The `window`
    /// parameter controls how many neighbors on each side to include.
    ///
    /// This implements the "parent/surrounding context" RAG pattern:
    /// retrieve on small chunks for precision, but return broader context
    /// for LLM comprehension.
    pub fn get_neighboring_chunks(&self, chunk_id: &str, window: usize) -> Vec<&Chunk> {
        // Find the target chunk.
        let Some(target) = self.chunks.iter().find(|c| c.id == chunk_id) else {
            return Vec::new();
        };

        let doc_id = &target.document_id;
        let target_section = target.metadata.section_index;

        // Determine the range of section indices to include.
        let min_section = target_section.saturating_sub(window);
        let max_section = target_section.saturating_add(window);

        // Collect all chunks from the same document within the window.
        let mut neighbors: Vec<&Chunk> = self
            .chunks
            .iter()
            .filter(|c| {
                c.document_id == *doc_id
                    && c.metadata.section_index >= min_section
                    && c.metadata.section_index <= max_section
            })
            .collect();

        // Sort by section index to maintain reading order.
        neighbors.sort_by_key(|c| c.metadata.section_index);
        neighbors
    }
}

// ---------------------------------------------------------------------------
// Cosine similarity
// ---------------------------------------------------------------------------

/// Compute cosine similarity between two vectors.
///
/// Returns a value in `[-1.0, 1.0]`. For normalized embedding vectors this
/// is equivalent to the dot product. Returns `0.0` for zero-length or
/// mismatched-dimension vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

// ---------------------------------------------------------------------------
// SummaryGenerator trait (LLM-assisted L0/L1, Sprint 3.3 placeholder)
// ---------------------------------------------------------------------------

/// Trait for LLM-assisted summary generation.
///
/// Implementation deferred to Sprint 4+ when LLM provider integration
/// is fully available. Current chunking uses truncation for L0/L1.
#[async_trait::async_trait]
pub trait SummaryGenerator: Send + Sync {
    /// Generate a ~100 token L0 summary of the content.
    async fn generate_l0_summary(
        &self,
        content: &str,
    ) -> Result<String, crate::error::KnowledgeError>;

    /// Generate a ~500 token L1 overview of the content.
    async fn generate_l1_overview(
        &self,
        content: &str,
    ) -> Result<String, crate::error::KnowledgeError>;

    /// Generate FAQ questions for retrieval augmentation.
    async fn generate_faq(
        &self,
        content: &str,
    ) -> Result<Vec<String>, crate::error::KnowledgeError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{ChunkLevel, ChunkMetadata};
    use crate::tokenizer::SimpleTokenizer;

    fn make_chunk(id: &str, content: &str, domain: &str) -> Chunk {
        make_chunk_with_section(id, content, domain, 0)
    }

    fn make_chunk_with_section(id: &str, content: &str, domain: &str, section: usize) -> Chunk {
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
                section_index: section,
            },
        }
    }

    fn make_retriever() -> HybridRetriever<SimpleTokenizer> {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0, // Disable threshold for basic tests.
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        retriever.index(make_chunk("c1", "Rust error handling patterns", "rust"));
        retriever.index(make_chunk("c2", "Python web frameworks", "python"));
        retriever.index(make_chunk("c3", "Rust async runtime with tokio", "rust"));
        retriever
    }

    // --- Basic Search Tests ---

    #[test]
    fn test_retrieval_keyword_search() {
        let mut retriever = make_retriever();
        retriever.config.strategy = SearchStrategy::KeywordSearch;

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("Rust error", &filter);
        assert!(!results.is_empty(), "keyword search should find results");
        assert!(results[0].bm25_score.is_some());
    }

    #[test]
    fn test_retrieval_semantic_search() {
        let mut retriever = make_retriever();
        retriever.config.strategy = SearchStrategy::SemanticSearch;

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("error handling", &filter);
        assert!(!results.is_empty(), "semantic search should find results");
        assert!(results[0].vector_score.is_some());
    }

    #[test]
    fn test_retrieval_hybrid_blend() {
        let retriever = make_retriever();
        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("Rust", &filter);
        assert!(!results.is_empty(), "blend search should find results");
        // Blend results should have both scores.
        assert!(results[0].vector_score.is_some());
        assert!(results[0].bm25_score.is_some());
    }

    // --- Domain Filter ---

    #[test]
    fn test_retrieval_domain_filter() {
        let retriever = make_retriever();
        let filter = RetrievalFilter {
            domain: Some("rust".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("error", &filter);
        for r in &results {
            assert_eq!(r.chunk.metadata.domain, "rust");
        }
    }

    // --- Limit ---

    #[test]
    fn test_retrieval_respects_limit() {
        let retriever = make_retriever();
        let filter = RetrievalFilter {
            limit: 1,
            ..Default::default()
        };
        let results = retriever.search("Rust", &filter);
        assert!(results.len() <= 1);
    }

    // --- Paragraph Dedup ---

    #[test]
    fn test_retrieval_paragraph_dedup() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: true,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);

        // Two chunks from the same document + section.
        retriever.index(make_chunk_with_section(
            "c1",
            "Rust error handling step 1",
            "rust",
            0,
        ));
        retriever.index(make_chunk_with_section(
            "c2",
            "Rust error handling step 2",
            "rust",
            0,
        ));
        // Different section.
        retriever.index(make_chunk_with_section(
            "c3",
            "Rust error recovery",
            "rust",
            1,
        ));

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("Rust error", &filter);

        // After dedup, should have at most 2 results (one per section).
        assert!(
            results.len() <= 2,
            "dedup should keep at most one per section, got {}",
            results.len()
        );
    }

    // --- Threshold ---

    #[test]
    fn test_retrieval_threshold_filters_low_scores() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.9, // Very high threshold.
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        retriever.index(make_chunk("c1", "Rust programming", "rust"));
        retriever.index(make_chunk("c2", "Python programming", "python"));

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        // "Rust" alone may not score above 0.9 in blend mode.
        let results = retriever.search("Rust", &filter);
        for r in &results {
            assert!(
                r.relevance >= 0.9,
                "results should be above threshold, got {}",
                r.relevance
            );
        }
    }

    // --- Quality Boost ---

    #[test]
    fn test_retrieval_quality_boost() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);

        // Same content, different quality.
        let c1 = make_chunk("c1", "Rust error handling guide", "rust");
        let c2 = make_chunk("c2", "Rust error handling tutorial", "rust");
        retriever.index_with_quality(c1, 1.0); // High quality.
        retriever.index_with_quality(c2, 0.1); // Low quality.

        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search("Rust error", &filter);
        assert!(results.len() >= 2);
        // Higher quality should rank first.
        assert_eq!(
            results[0].chunk.id, "c1",
            "higher quality should rank first"
        );
    }

    // --- Search Strategy ---

    #[test]
    fn test_search_strategy_default_is_hybrid() {
        assert_eq!(SearchStrategy::default(), SearchStrategy::Hybrid);
    }

    // --- Cosine Similarity ---

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = super::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6, "identical vectors should be 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = super::cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6, "orthogonal vectors should be 0.0");
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = super::cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6, "opposite vectors should be -1.0");
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(super::cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_mismatched() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        assert_eq!(super::cosine_similarity(&a, &b), 0.0);
    }

    // --- Embedding-aware Search ---

    #[test]
    fn test_search_with_embedding_cosine() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            strategy: SearchStrategy::SemanticSearch,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);

        // Two chunks with known embeddings.
        let c1 = make_chunk("c1", "machine learning basics", "ml");
        let c2 = make_chunk("c2", "cooking recipes for pasta", "cooking");

        // c1 embedding is close to query, c2 is orthogonal.
        retriever.index_with_embedding(c1, vec![0.9, 0.1, 0.0], 1.0);
        retriever.index_with_embedding(c2, vec![0.0, 0.1, 0.9], 1.0);

        assert!(retriever.has_embeddings());

        // Query embedding close to c1.
        let query_embedding = vec![0.95, 0.05, 0.0];
        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results = retriever.search_with_embedding("ml", Some(&query_embedding), &filter);

        assert!(!results.is_empty());
        assert_eq!(
            results[0].chunk.id, "c1",
            "closest embedding should rank first"
        );
        assert!(
            results[0].vector_score.unwrap() > results.last().unwrap().vector_score.unwrap(),
            "first result should have higher vector score"
        );
    }

    #[test]
    fn test_blend_search_with_embedding() {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            strategy: SearchStrategy::Hybrid,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);

        let c1 = make_chunk("c1", "Rust error handling patterns", "rust");
        let c2 = make_chunk("c2", "Python web frameworks", "python");

        retriever.index_with_embedding(c1, vec![0.8, 0.2], 1.0);
        retriever.index_with_embedding(c2, vec![0.1, 0.9], 1.0);

        let query_embedding = vec![0.85, 0.15];
        let filter = RetrievalFilter {
            limit: 10,
            ..Default::default()
        };
        let results =
            retriever.search_with_embedding("Rust error", Some(&query_embedding), &filter);

        assert!(!results.is_empty());
        // Blend should have both vector and BM25 scores.
        assert!(results[0].vector_score.is_some());
        assert!(results[0].bm25_score.is_some());
    }
}
