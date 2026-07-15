//! Service-layer adapter for automatic selected-collection knowledge context.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use y_context::{KnowledgeContextRetriever, KnowledgeContextSnippet};
use y_core::embedding::EmbeddingProvider;
use y_knowledge::chunking::ChunkLevel;
use y_knowledge::middleware::{InjectKnowledge, KnowledgeRetrievalRequest};
use y_knowledge::tokenizer::AutoTokenizer;

const MAX_AUTO_KNOWLEDGE_CHUNKS: usize = 5;

pub struct KnowledgeContextRetrievalAdapter {
    knowledge: Arc<Mutex<InjectKnowledge<AutoTokenizer>>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl KnowledgeContextRetrievalAdapter {
    pub fn new(
        knowledge: Arc<Mutex<InjectKnowledge<AutoTokenizer>>>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            knowledge,
            embedding_provider,
        }
    }
}

#[async_trait]
impl KnowledgeContextRetriever for KnowledgeContextRetrievalAdapter {
    async fn retrieve(
        &self,
        query: &str,
        collections: &[String],
    ) -> Result<Vec<KnowledgeContextSnippet>, String> {
        let query_embedding = if let Some(provider) = &self.embedding_provider {
            provider
                .embed(query)
                .await
                .map(|result| Some(result.vector))
                .map_err(|error| format!("failed to embed knowledge context query: {error}"))?
        } else {
            None
        };
        let knowledge = self
            .knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        for collection in collections
            .iter()
            .map(|collection| collection.trim())
            .filter(|collection| !collection.is_empty())
        {
            let request = KnowledgeRetrievalRequest {
                query: query.to_string(),
                domain: None,
                collection: Some(collection.to_string()),
                level: Some(ChunkLevel::L0),
                limit: MAX_AUTO_KNOWLEDGE_CHUNKS,
                relax_domain: false,
            };
            for item in knowledge.retrieve(&request, query_embedding.as_deref()) {
                if seen.insert(item.chunk_id.clone()) {
                    items.push(item);
                }
            }
        }
        items.sort_by(|left, right| {
            right
                .relevance
                .partial_cmp(&left.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        items.truncate(MAX_AUTO_KNOWLEDGE_CHUNKS);

        Ok(items
            .into_iter()
            .map(|item| KnowledgeContextSnippet {
                content: item.content,
                token_estimate: item.token_estimate,
                title: item.title,
                source: item.source,
                collection: item.collection,
                chunk_id: item.chunk_id,
                relevance: item.relevance,
            })
            .collect())
    }
}
