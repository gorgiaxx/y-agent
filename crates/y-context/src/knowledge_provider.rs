//! Knowledge context provider — auto-injects relevant knowledge into context.
//!
//! Priority 350 (between `InjectMemory` at 300 and `InjectSkills` at 400).
//! Extracts the user query from `ContextRequest`, searches the knowledge base,
//! and injects matching chunks as `ContextCategory::Knowledge` items.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use std::fmt::Write;
use y_core::embedding::EmbeddingProvider;
use y_knowledge::chunking::is_generic_section_title;
use y_knowledge::middleware::{InjectKnowledge, KnowledgeContextItem};
use y_knowledge::tokenizer::SimpleTokenizer;

use crate::middleware_adapter::stage_priorities;
use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Context provider that auto-injects knowledge base content.
///
/// Wraps the `InjectKnowledge` middleware from `y-knowledge` and adapts it
/// to the `ContextProvider` trait for the context assembly pipeline.
///
/// When an `EmbeddingProvider` is configured, the user query is embedded
/// before retrieval so that cosine similarity can be used for semantic search.
pub struct KnowledgeContextProvider {
    knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl KnowledgeContextProvider {
    /// Create a new knowledge context provider.
    pub fn new(knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>) -> Self {
        Self {
            knowledge,
            embedding_provider: None,
        }
    }

    /// Create a knowledge context provider with an embedding provider.
    pub fn with_embedding(
        knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self {
            knowledge,
            embedding_provider: Some(embedding_provider),
        }
    }

    /// Format knowledge items into a context block.
    ///
    /// When L0/L1 metadata is available, items use structured format
    /// (summary + section titles). The LLM is guided to use the
    /// `KnowledgeSearch` tool for full content when needed.
    fn format_knowledge_block(items: &[KnowledgeContextItem]) -> String {
        if items.is_empty() {
            return String::new();
        }

        let has_structured = items.iter().any(|i| i.summary.is_some());

        let mut block = String::from("<knowledge_context>\n");
        if has_structured {
            block.push_str("The following knowledge is relevant to your query. Use KnowledgeSearch tool to get full details for specific sections.\n\n");
        } else {
            block.push_str("The following knowledge items are relevant to the user's query:\n\n");
        }

        for (i, item) in items.iter().enumerate() {
            let _ = writeln!(
                &mut block,
                "--- Knowledge Item {} (relevance: {:.0}%) ---",
                i + 1,
                item.relevance * 100.0
            );
            if !item.title.is_empty() {
                let _ = writeln!(&mut block, "Source: {}", item.title);
            }
            // Structured L0/L1 info (if available).
            if let Some(ref summary) = item.summary {
                let _ = writeln!(&mut block, "Summary: {summary}");
            }
            // Skip generic fallback titles ("Section 1", "Section 2", ...)
            // that carry no information and waste tokens.
            let meaningful: Vec<_> = item
                .section_titles
                .iter()
                .filter(|t| !is_generic_section_title(t))
                .collect();
            if !meaningful.is_empty() {
                block.push_str("Sections:\n");
                for (j, title) in meaningful.iter().enumerate() {
                    let _ = writeln!(&mut block, "  {}. {}", j + 1, title);
                }
            }
            // L2 raw content (fallback when no structured info).
            if item.summary.is_none() {
                block.push_str(&item.content);
                block.push('\n');
            }
            block.push('\n');
        }

        block.push_str("</knowledge_context>");
        block
    }
}

#[async_trait]
impl ContextProvider for KnowledgeContextProvider {
    fn name(&self) -> &'static str {
        "inject_knowledge"
    }

    fn priority(&self) -> u32 {
        stage_priorities::INJECT_KNOWLEDGE
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        // Extract the user query from the request context.
        let (user_query, collections) = match &ctx.request {
            Some(req) if !req.user_query.is_empty() => {
                (req.user_query.clone(), req.knowledge_collections.clone())
            }
            _ => return Ok(()), // No query available, skip.
        };

        // Skip knowledge retrieval entirely when the user has not explicitly
        // selected any collection via slash command. This avoids unnecessary
        // embedding API calls for every chat message.
        if collections.is_empty() {
            tracing::debug!("knowledge retrieval: skipped (no collections selected)");
            return Ok(());
        }

        // Generate query embedding if an embedding provider is configured.
        let query_embedding = if let Some(ref provider) = self.embedding_provider {
            match provider.embed(&user_query).await {
                Ok(result) => Some(result.vector),
                Err(e) => {
                    tracing::warn!("Failed to embed query, falling back to keyword search: {e}");
                    None
                }
            }
        } else {
            None
        };

        // Retrieve relevant knowledge, filtering by selected collections.
        let knowledge = self
            .knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let items = knowledge.retrieve_for_context(&user_query, query_embedding.as_deref(), None);

        if items.is_empty() {
            tracing::info!(
                query = %user_query,
                collections = ?collections,
                "knowledge retrieval: no matching chunks found"
            );
            return Ok(());
        }

        // Format and inject.
        let content = Self::format_knowledge_block(&items);
        let token_estimate: u32 = items.iter().map(|i| i.token_estimate).sum();

        ctx.add(ContextItem {
            category: ContextCategory::Knowledge,
            content,
            token_estimate,
            priority: self.priority(),
        });

        tracing::info!(
            query = %user_query,
            collections = ?collections,
            items = items.len(),
            tokens = token_estimate,
            "knowledge retrieval: injected context"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
    use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};

    fn make_provider() -> KnowledgeContextProvider {
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
            content: "Rust error handling uses the Result type.".to_string(),
            token_estimate: 10,
            metadata: ChunkMetadata {
                source: "test.md".to_string(),
                domain: "programming".to_string(),
                title: "Rust Basics".to_string(),
                section_index: 0,
            },
        });

        let knowledge = InjectKnowledge::new(retriever);
        KnowledgeContextProvider::new(Arc::new(Mutex::new(knowledge)))
    }

    #[tokio::test]
    async fn test_injects_knowledge_when_query_matches() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "How does Rust handle errors?".to_string(),
                knowledge_collections: vec!["default".to_string()],
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert!(!ctx.items.is_empty(), "should inject knowledge items");
        assert_eq!(ctx.items[0].category, ContextCategory::Knowledge);
        assert!(ctx.items[0].content.contains("knowledge_context"));
    }

    #[tokio::test]
    async fn test_skips_when_no_collections_selected() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "How does Rust handle errors?".to_string(),
                knowledge_collections: vec![], // no collections selected
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert!(
            ctx.items.is_empty(),
            "should skip when no collections selected"
        );
    }

    #[tokio::test]
    async fn test_skips_when_no_query() {
        let provider = make_provider();
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty(), "should skip without query");
    }

    #[tokio::test]
    async fn test_skips_when_no_match() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "quantum physics theory".to_string(),
                knowledge_collections: vec!["default".to_string()],
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty(), "should skip for unrelated query");
    }

    #[test]
    fn test_priority_and_name() {
        let provider = make_provider();
        assert_eq!(provider.name(), "inject_knowledge");
        assert_eq!(provider.priority(), 350);
    }
}
