//! `InjectKnowledge` context middleware.
//!
//! Provides domain-triggered knowledge retrieval and context injection
//! at priority 350 (between `InjectMemory` at 300 and `InjectSkills` at 400).
//!
//! Features:
//! - Domain keyword extraction from user messages
//! - Token budget control (default 4000 tokens)
//! - Progressive L0/L1/L2 injection inspired by `OpenViking`'s context layers

use std::collections::HashMap;

use crate::retrieval::{HybridRetriever, RetrievalFilter, RetrievalResult};
use crate::tokenizer::Tokenizer;

/// Priority for `InjectKnowledge` in the context pipeline.
///
/// Between `InjectMemory` (300) and `InjectSkills` (400).
pub const INJECT_KNOWLEDGE_PRIORITY: u32 = 350;

/// Default token budget for knowledge context.
const DEFAULT_KNOWLEDGE_BUDGET: u32 = 4_000;

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Configuration for the knowledge injection middleware.
#[derive(Debug, Clone)]
pub struct InjectKnowledgeConfig {
    /// Maximum token budget for knowledge context.
    pub token_budget: u32,
    /// Maximum number of knowledge chunks to inject.
    pub max_chunks: usize,
    /// Minimum relevance score to include a result.
    pub min_relevance: f64,
    /// Default resolution level (`l0`, `l1`, `l2`).
    pub default_resolution: String,
    /// Number of neighboring chunks to include on each side of a matched
    /// chunk for context expansion. Set to 0 to disable.
    ///
    /// When > 0, the retriever fetches the matched chunk plus `window`
    /// chunks before and after it from the same document, joining them
    /// into a single coherent passage. This gives the LLM enough context
    /// to understand the retrieved passage.
    pub context_window: usize,
}

impl Default for InjectKnowledgeConfig {
    fn default() -> Self {
        Self {
            token_budget: DEFAULT_KNOWLEDGE_BUDGET,
            max_chunks: 5,
            min_relevance: 0.3,
            default_resolution: "l0".to_string(),
            context_window: 2,
        }
    }
}

/// Lightweight metadata for progressive context injection.
///
/// Stored per `document_id` to provide L0 summary, L1 section titles,
/// and LLM-generated tags when formatting retrieval results for LLM context.
#[derive(Debug, Clone)]
pub struct EntryMetadata {
    /// Document title.
    pub title: String,
    /// L0: compact summary (~100 tokens).
    pub summary: Option<String>,
    /// L1: section titles only (not full content, to save memory).
    pub section_titles: Vec<String>,
    /// LLM-generated semantic tags for topic identification.
    pub tags: Vec<String>,
}

/// Knowledge item ready for context injection.
#[derive(Debug, Clone)]
pub struct KnowledgeContextItem {
    /// Content to inject.
    pub content: String,
    /// Estimated token count.
    pub token_estimate: u32,
    /// Source title.
    pub title: String,
    /// Relevance score.
    pub relevance: f64,
    /// Chunk ID for reference.
    pub chunk_id: String,
    /// Document ID that this chunk belongs to.
    pub document_id: String,
    /// Domain classification.
    pub domain: String,
    /// L0 summary of the parent document (if available).
    pub summary: Option<String>,
    /// L1 section titles of the parent document (if available).
    pub section_titles: Vec<String>,
}

/// `InjectKnowledge` middleware — retrieves and injects relevant knowledge.
///
/// This module provides the retrieval + formatting logic. Integration with
/// `ContextProvider` trait (from `y-context`) is done via the bridge in
/// `y-context` or at the service layer.
///
/// Supports progressive context injection inspired by `OpenViking`:
/// - When L0/L1 metadata is registered, injects structured summaries
///   (saving tokens while preserving coverage).
/// - When no metadata is available, falls back to raw L2 chunk injection.
#[derive(Debug)]
pub struct InjectKnowledge<T: Tokenizer> {
    retriever: HybridRetriever<T>,
    config: InjectKnowledgeConfig,
    /// Per-document metadata for progressive L0/L1 injection.
    entry_metadata: HashMap<String, EntryMetadata>,
}

impl<T: Tokenizer> InjectKnowledge<T> {
    /// Create a new middleware with a retriever and default config.
    pub fn new(retriever: HybridRetriever<T>) -> Self {
        Self {
            retriever,
            config: InjectKnowledgeConfig::default(),
            entry_metadata: HashMap::new(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(retriever: HybridRetriever<T>, config: InjectKnowledgeConfig) -> Self {
        Self {
            retriever,
            config,
            entry_metadata: HashMap::new(),
        }
    }

    /// Register L0/L1 metadata for a document.
    ///
    /// Called during ingestion and startup reindexing so that retrieval
    /// results can include structured summaries instead of raw chunks.
    pub fn register_entry_metadata(&mut self, document_id: &str, metadata: EntryMetadata) {
        self.entry_metadata
            .insert(document_id.to_string(), metadata);
    }

    /// Remove metadata for a document (e.g., on entry deletion).
    pub fn remove_entry_metadata(&mut self, document_id: &str) {
        self.entry_metadata.remove(document_id);
    }

    /// Retrieve and format knowledge items for context injection.
    ///
    /// Called during context assembly. When `query_embedding` is provided,
    /// it is passed to the retriever for real cosine similarity search.
    ///
    /// When `context_window > 0`, each matched chunk is expanded with
    /// its neighboring chunks from the same document, providing the LLM
    /// with enough surrounding text to understand the passage in context.
    ///
    /// When L0/L1 metadata is available for a matched document, the result
    /// is formatted with structured summary + section titles instead of
    /// raw chunk text, saving tokens and improving LLM comprehension.
    pub fn retrieve_for_context(
        &self,
        user_query: &str,
        query_embedding: Option<&[f32]>,
        domain_hint: Option<&str>,
    ) -> Vec<KnowledgeContextItem> {
        let filter = RetrievalFilter {
            domain: domain_hint.map(String::from),
            limit: self.config.max_chunks,
            ..Default::default()
        };

        let results = self
            .retriever
            .search_with_embedding(user_query, query_embedding, &filter);

        // Filter by minimum relevance.
        let filtered: Vec<&RetrievalResult> = results
            .iter()
            .filter(|r| r.relevance >= self.config.min_relevance)
            .collect();

        // Deduplicate by document_id: only inject one item per document.
        // When metadata is available we inject the structured L0/L1 summary,
        // so duplicate chunks from the same document are redundant.
        let mut seen_documents: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Track which document+section ranges have already been included
        // to avoid injecting overlapping content from neighboring expansions.
        let mut seen_sections: std::collections::HashSet<(String, usize)> =
            std::collections::HashSet::new();

        // Format within budget.
        let mut items = Vec::new();
        let mut remaining_budget = self.config.token_budget;

        for result in filtered {
            let doc_id = &result.chunk.document_id;
            let metadata = self.entry_metadata.get(doc_id);

            // If we have L0/L1 metadata, deduplicate by document.
            if metadata.is_some() && !seen_documents.insert(doc_id.clone()) {
                continue; // Already injected this document's summary
            }

            // Format content based on available metadata.
            let content = if let Some(meta) = metadata {
                Self::format_structured(result, meta)
            } else if self.config.context_window > 0 {
                let neighbors = self
                    .retriever
                    .get_neighboring_chunks(&result.chunk.id, self.config.context_window);

                // Filter out sections we've already included.
                let new_neighbors: Vec<&&crate::chunking::Chunk> = neighbors
                    .iter()
                    .filter(|c| {
                        !seen_sections.contains(&(c.document_id.clone(), c.metadata.section_index))
                    })
                    .collect();

                if new_neighbors.is_empty() {
                    seen_sections.insert((
                        result.chunk.document_id.clone(),
                        result.chunk.metadata.section_index,
                    ));
                    Self::format_chunk(result)
                } else {
                    for chunk in &new_neighbors {
                        seen_sections
                            .insert((chunk.document_id.clone(), chunk.metadata.section_index));
                    }

                    let joined: String = new_neighbors
                        .iter()
                        .map(|c| c.content.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");

                    format!(
                        "[Knowledge: {} (relevance: {:.2})]\n{}",
                        result.chunk.metadata.title, result.relevance, joined
                    )
                }
            } else {
                seen_sections.insert((
                    result.chunk.document_id.clone(),
                    result.chunk.metadata.section_index,
                ));
                Self::format_chunk(result)
            };

            let tokens = estimate_tokens(&content);

            if tokens > remaining_budget {
                break;
            }

            remaining_budget = remaining_budget.saturating_sub(tokens);

            let (summary, section_titles) = if let Some(meta) = metadata {
                (meta.summary.clone(), meta.section_titles.clone())
            } else {
                (None, Vec::new())
            };

            items.push(KnowledgeContextItem {
                content,
                token_estimate: tokens,
                title: result.chunk.metadata.title.clone(),
                relevance: result.relevance,
                chunk_id: result.chunk.id.clone(),
                document_id: result.chunk.document_id.clone(),
                domain: result.chunk.metadata.domain.clone(),
                summary,
                section_titles,
            });
        }

        items
    }

    /// Format a retrieval result using structured L0/L1 metadata.
    ///
    /// Produces a compact representation: L0 summary + L1 section titles,
    /// guiding the LLM to use `KnowledgeSearch` for full content.
    fn format_structured(result: &RetrievalResult, meta: &EntryMetadata) -> String {
        use crate::chunking::is_generic_section_title;
        use std::fmt::Write;
        let mut out = format!(
            "[Knowledge: {} (relevance: {:.2})]",
            result.chunk.metadata.title, result.relevance,
        );

        // L0 summary
        if let Some(ref summary) = meta.summary {
            write!(&mut out, "\nSummary: {summary}").unwrap();
        }

        // L1 section titles -- skip generic fallbacks ("Section 1", "Section 2", ...)
        // that carry no information and waste tokens.
        let meaningful: Vec<_> = meta
            .section_titles
            .iter()
            .filter(|t| !is_generic_section_title(t))
            .collect();
        if !meaningful.is_empty() {
            out.push_str("\nSections:");
            for (i, title) in meaningful.iter().enumerate() {
                write!(&mut out, "\n  {}. {}", i + 1, title).unwrap();
            }
        }

        // Tags -- show LLM-generated semantic tags so the main LLM can
        // understand entry topics and refine search queries.
        if !meta.tags.is_empty() {
            let tags_str = meta.tags.join(", ");
            write!(&mut out, "\nTags: {tags_str}").unwrap();
        }

        out
    }

    /// Format a retrieval result for context injection (L2 fallback).
    fn format_chunk(result: &RetrievalResult) -> String {
        format!(
            "[Knowledge: {} (relevance: {:.2})]\n{}",
            result.chunk.metadata.title, result.relevance, result.chunk.content
        )
    }

    /// Get the priority for this middleware.
    pub const fn priority(&self) -> u32 {
        INJECT_KNOWLEDGE_PRIORITY
    }

    /// Get the name of this middleware.
    pub fn name(&self) -> &'static str {
        "inject_knowledge"
    }

    /// Get a reference to the inner retriever.
    pub fn retriever(&self) -> &HybridRetriever<T> {
        &self.retriever
    }

    /// Get a mutable reference to the inner retriever for indexing.
    pub fn retriever_mut(&mut self) -> &mut HybridRetriever<T> {
        &mut self.retriever
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::{Chunk, ChunkLevel, ChunkMetadata};
    use crate::retrieval::RetrievalConfig;
    use crate::tokenizer::SimpleTokenizer;

    fn make_middleware() -> InjectKnowledge<SimpleTokenizer> {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        retriever.index(Chunk {
            id: "c1".to_string(),
            document_id: "doc-1".to_string(),
            level: ChunkLevel::L2,
            content: "Rust error handling uses the Result type for recoverable errors and panic for unrecoverable ones.".to_string(),
            token_estimate: 20,
            metadata: ChunkMetadata {
                source: "test".to_string(),
                domain: "rust".to_string(),
                title: "Rust Error Handling".to_string(),
                section_index: 0,
            },
        });
        retriever.index(Chunk {
            id: "c2".to_string(),
            document_id: "doc-2".to_string(),
            level: ChunkLevel::L2,
            content: "Python exception handling uses try-except blocks for error management."
                .to_string(),
            token_estimate: 15,
            metadata: ChunkMetadata {
                source: "test".to_string(),
                domain: "python".to_string(),
                title: "Python Exceptions".to_string(),
                section_index: 0,
            },
        });

        InjectKnowledge::new(retriever)
    }

    #[test]
    fn test_middleware_name_and_priority() {
        let mw = make_middleware();
        assert_eq!(mw.name(), "inject_knowledge");
        assert_eq!(mw.priority(), 350);
    }

    #[test]
    fn test_retrieve_for_context_finds_results() {
        let mw = make_middleware();
        let items = mw.retrieve_for_context("Rust error handling", None, None);
        assert!(!items.is_empty(), "should find relevant knowledge");
        assert!(items[0].content.contains("Rust"));
    }

    #[test]
    fn test_retrieve_for_context_with_domain_filter() {
        let mw = make_middleware();
        let items = mw.retrieve_for_context("error", None, Some("rust"));
        for item in &items {
            // All results should be from rust domain.
            assert!(
                item.content.contains("Rust"),
                "expected rust domain content"
            );
        }
    }

    #[test]
    fn test_retrieve_for_context_respects_budget() {
        let config = InjectKnowledgeConfig {
            token_budget: 10, // Very small budget.
            ..Default::default()
        };
        let retriever_config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: false,
            ..Default::default()
        };
        let retriever = HybridRetriever::with_config(SimpleTokenizer::new(), retriever_config);
        let mw = InjectKnowledge::with_config(retriever, config);
        let items = mw.retrieve_for_context("anything", None, None);
        let total_tokens: u32 = items.iter().map(|i| i.token_estimate).sum();
        assert!(total_tokens <= 10, "should respect budget");
    }

    #[test]
    fn test_retrieve_for_context_empty_query() {
        let mw = make_middleware();
        let items = mw.retrieve_for_context("quantum physics", None, None);
        // No matches for unrelated query.
        assert!(
            items.is_empty(),
            "should find no results for unrelated query"
        );
    }

    #[test]
    fn test_knowledge_context_item_format() {
        let mw = make_middleware();
        let items = mw.retrieve_for_context("Rust error", None, None);
        if let Some(item) = items.first() {
            assert!(item.content.starts_with("[Knowledge:"));
            assert!(item.token_estimate > 0);
            assert!(!item.chunk_id.is_empty());
        }
    }

    #[test]
    fn test_default_config() {
        let config = InjectKnowledgeConfig::default();
        assert_eq!(config.token_budget, 4000);
        assert_eq!(config.max_chunks, 5);
        assert_eq!(config.min_relevance, 0.3);
        assert_eq!(config.default_resolution, "l0");
        assert_eq!(config.context_window, 2);
    }

    #[test]
    fn test_retrieve_with_entry_metadata_structured() {
        let mut mw = make_middleware();

        // Register L0/L1 metadata for doc-1.
        mw.register_entry_metadata(
            "doc-1",
            EntryMetadata {
                title: "Rust Error Handling".to_string(),
                summary: Some(
                    "Guide covering Result type, ? operator, and custom errors.".to_string(),
                ),
                section_titles: vec![
                    "Result Type".to_string(),
                    "The ? Operator".to_string(),
                    "Custom Errors".to_string(),
                ],
                tags: vec![],
            },
        );

        let items = mw.retrieve_for_context("Rust error handling", None, None);
        assert!(!items.is_empty(), "should find results");

        let item = &items[0];
        // Should use structured format (not raw L2 chunk text).
        assert!(
            item.content.contains("Summary:"),
            "should contain L0 summary, got: {}",
            item.content
        );
        assert!(
            item.content.contains("Sections:"),
            "should contain L1 sections, got: {}",
            item.content
        );
        assert!(
            item.content.contains("Result Type"),
            "should list section titles"
        );
        assert!(
            item.content.contains("The ? Operator"),
            "should list section titles"
        );
        // Should NOT contain the raw L2 chunk content.
        assert!(
            !item.content.contains("recoverable errors and panic"),
            "should not contain raw L2 text when metadata available"
        );
        // Meta fields should be populated.
        assert!(item.summary.is_some());
        assert!(!item.section_titles.is_empty());
    }

    #[test]
    fn test_retrieve_without_metadata_uses_l2_fallback() {
        let mw = make_middleware();
        // No metadata registered — should fall back to L2 chunk injection.
        let items = mw.retrieve_for_context("Rust error handling", None, None);
        assert!(!items.is_empty());

        let item = &items[0];
        // Should contain the raw L2 chunk text.
        assert!(
            item.content.contains("Result type"),
            "should contain L2 content"
        );
        // Meta fields should be empty.
        assert!(item.summary.is_none());
        assert!(item.section_titles.is_empty());
    }

    #[test]
    fn test_register_and_remove_metadata() {
        let mut mw = make_middleware();

        mw.register_entry_metadata(
            "doc-1",
            EntryMetadata {
                title: "Test".to_string(),
                summary: Some("Test summary".to_string()),
                section_titles: vec![],
                tags: vec![],
            },
        );

        // Should use structured format.
        let items = mw.retrieve_for_context("Rust error", None, None);
        assert!(!items.is_empty());
        assert!(items[0].content.contains("Summary:"));

        // Remove and verify fallback.
        mw.remove_entry_metadata("doc-1");
        let items = mw.retrieve_for_context("Rust error", None, None);
        assert!(!items.is_empty());
        assert!(
            !items[0].content.contains("Summary:"),
            "should fall back after removal"
        );
    }
}
