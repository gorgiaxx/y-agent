//! `KnowledgeSearch` built-in tool: search the knowledge base.
//!
//! Allows the LLM to actively query the knowledge base for relevant
//! information when it needs deeper context than auto-injection provides.
//!
//! When an `EmbeddingProvider` is available, the query is embedded before
//! retrieval so that cosine similarity is used instead of text matching.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use y_core::embedding::EmbeddingProvider;

use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;
use y_knowledge::middleware::{InjectKnowledge, KnowledgeContextItem};
use y_knowledge::tokenizer::SimpleTokenizer;

/// Built-in tool for searching the knowledge base.
pub struct KnowledgeSearchTool {
    def: ToolDefinition,
    knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl KnowledgeSearchTool {
    /// Create a new knowledge search tool with a shared knowledge middleware.
    pub fn new(knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>) -> Self {
        Self {
            def: Self::tool_definition(),
            knowledge,
            embedding_provider: None,
        }
    }

    /// Create a knowledge search tool with an embedding provider for
    /// vector-based semantic search.
    pub fn with_embedding(
        knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self {
            def: Self::tool_definition(),
            knowledge,
            embedding_provider: Some(embedding_provider),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("KnowledgeSearch"),
            description: concat!(
                "Search the knowledge base for relevant information. ",
                "The engine combines semantic similarity with keyword matching ",
                "for best results. Formulate your query as a natural question ",
                "or descriptive phrase about what you need -- avoid ",
                "concatenating raw keywords.",
            )
            .into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": concat!(
                            "A natural-language description of the information you need. ",
                            "Write a complete sentence or phrase that captures the MEANING ",
                            "of what you are looking for. The search engine uses both ",
                            "semantic understanding and keyword matching -- descriptive ",
                            "queries perform significantly better than keyword lists. ",
                            "Describe WHAT you want to know, not a list of terms you ",
                            "expect to find in the text.",
                        )
                    },
                    "domain": {
                        "type": "string",
                        "description": concat!(
                            "Optional domain filter to narrow results. ",
                            "Use this to scope the search when you know the ",
                            "relevant domain.",
                        )
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 5, max: 20)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
            result_schema: None,
            category: ToolCategory::Knowledge,
            tool_type: ToolType::BuiltIn,
            capabilities: y_core::runtime::RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    fn format_results(items: &[KnowledgeContextItem]) -> serde_json::Value {
        let results: Vec<serde_json::Value> = items
            .iter()
            .map(|item| {
                let mut obj = serde_json::json!({
                    "title": item.title,
                    "content": item.content,
                    "relevance": format!("{:.2}", item.relevance),
                    "chunk_id": item.chunk_id,
                });
                if let Some(ref summary) = item.summary {
                    obj["summary"] = serde_json::json!(summary);
                }
                if !item.section_titles.is_empty() {
                    let meaningful: Vec<_> = item
                        .section_titles
                        .iter()
                        .filter(|t| !y_knowledge::chunking::is_generic_section_title(t))
                        .collect();
                    if !meaningful.is_empty() {
                        obj["sections"] = serde_json::json!(meaningful);
                    }
                }
                obj
            })
            .collect();

        serde_json::json!({
            "results": results,
            "count": results.len(),
        })
    }
}

#[async_trait]
impl Tool for KnowledgeSearchTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let query =
            input.arguments["query"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'query' parameter".into(),
                })?;

        let domain = input.arguments.get("domain").and_then(|v| v.as_str());
        let _limit = input
            .arguments
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5)
            .min(20) as usize;

        // Embed the query for cosine similarity when a provider is available.
        let query_embedding = if let Some(ref provider) = self.embedding_provider {
            match provider.embed(query).await {
                Ok(result) => Some(result.vector),
                Err(e) => {
                    tracing::warn!("Failed to embed query, falling back to keyword search: {e}");
                    None
                }
            }
        } else {
            None
        };

        let knowledge = self
            .knowledge
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let items = knowledge.retrieve_for_context(query, query_embedding.as_deref(), domain);

        if items.is_empty() {
            return Ok(ToolOutput {
                success: true,
                content: serde_json::json!({
                    "results": [],
                    "count": 0,
                    "message": "No relevant knowledge found for the query."
                }),
                warnings: vec![],
                metadata: serde_json::json!({}),
            });
        }

        Ok(ToolOutput {
            success: true,
            content: Self::format_results(&items),
            warnings: vec![],
            metadata: serde_json::json!({
                "query": query,
                "domain_filter": domain,
            }),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::embedding::{EmbeddingError, EmbeddingResult};
    use y_core::types::SessionId;
    use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
    use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};

    /// Mock embedding provider that returns a fixed 3-dimensional vector.
    struct MockEmbeddingProvider;

    #[async_trait]
    impl EmbeddingProvider for MockEmbeddingProvider {
        async fn embed(&self, _text: &str) -> Result<EmbeddingResult, EmbeddingError> {
            Ok(EmbeddingResult {
                vector: vec![0.1, 0.2, 0.3],
                dimensions: 3,
                model: "mock".into(),
                token_count: 5,
            })
        }
        fn dimensions(&self) -> usize {
            3
        }
        fn model_name(&self) -> &str {
            "mock"
        }
    }

    fn make_tool() -> KnowledgeSearchTool {
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
            content: "Rust error handling uses Result type.".to_string(),
            token_estimate: 10,
            metadata: ChunkMetadata {
                source: "test.md".to_string(),
                domain: "programming".to_string(),
                title: "Rust Basics".to_string(),
                section_index: 0,
            },
        });

        let knowledge = InjectKnowledge::new(retriever);
        KnowledgeSearchTool::new(Arc::new(Mutex::new(knowledge)))
    }

    fn make_tool_with_embedding() -> KnowledgeSearchTool {
        let config = RetrievalConfig {
            min_similarity_threshold: 0.0,
            enable_dedup: false,
            ..Default::default()
        };
        let mut retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        // Index a chunk WITH an embedding vector so cosine similarity works.
        retriever.index_with_embedding(
            Chunk {
                id: "c1".to_string(),
                document_id: "doc-1".to_string(),
                level: ChunkLevel::L2,
                content: "Rust error handling uses Result type.".to_string(),
                token_estimate: 10,
                metadata: ChunkMetadata {
                    source: "test.md".to_string(),
                    domain: "programming".to_string(),
                    title: "Rust Basics".to_string(),
                    section_index: 0,
                },
            },
            vec![0.1, 0.2, 0.3],
            0.9,
        );

        let knowledge = InjectKnowledge::new(retriever);
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(MockEmbeddingProvider);
        KnowledgeSearchTool::with_embedding(Arc::new(Mutex::new(knowledge)), provider)
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("KnowledgeSearch"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_search_finds_results() {
        let tool = make_tool();
        let input = make_input(serde_json::json!({ "query": "Rust error handling" }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert!(output.content["count"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let tool = make_tool();
        let input = make_input(serde_json::json!({ "query": "quantum physics" }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"].as_u64().unwrap(), 0);
    }

    #[tokio::test]
    async fn test_search_missing_query() {
        let tool = make_tool();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_definition() {
        let def = KnowledgeSearchTool::tool_definition();
        assert_eq!(def.name.as_str(), "KnowledgeSearch");
        assert_eq!(def.category, ToolCategory::Knowledge);
        assert!(!def.is_dangerous);
    }

    #[tokio::test]
    async fn test_search_with_embedding_finds_results() {
        let tool = make_tool_with_embedding();
        let input = make_input(serde_json::json!({ "query": "Rust error" }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert!(
            output.content["count"].as_u64().unwrap() > 0,
            "should find results using cosine similarity"
        );
    }

    #[tokio::test]
    async fn test_with_embedding_constructor() {
        let config = RetrievalConfig::default();
        let retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        let knowledge = InjectKnowledge::new(retriever);
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(MockEmbeddingProvider);
        let tool = KnowledgeSearchTool::with_embedding(Arc::new(Mutex::new(knowledge)), provider);
        assert!(tool.embedding_provider.is_some());
    }
}
