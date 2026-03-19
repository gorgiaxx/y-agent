//! `knowledge_search` built-in tool: search the knowledge base.
//!
//! Allows the LLM to actively query the knowledge base for relevant
//! information when it needs deeper context than auto-injection provides.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

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
}

impl KnowledgeSearchTool {
    /// Create a new knowledge search tool with a shared knowledge middleware.
    pub fn new(knowledge: Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>) -> Self {
        Self {
            def: Self::tool_definition(),
            knowledge,
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("knowledge_search"),
            description: "Search the knowledge base for relevant information. Use this tool \
                          when you need specific knowledge from imported documents, technical \
                          references, or domain-specific content. Returns the most relevant \
                          knowledge chunks with their source and relevance score."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query describing what knowledge you need"
                    },
                    "domain": {
                        "type": "string",
                        "description": "Optional domain filter (e.g. 'programming', 'science')"
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
                    obj["sections"] = serde_json::json!(item.section_titles);
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
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(20) as usize;

        let knowledge = self.knowledge.lock().unwrap_or_else(|e| e.into_inner());
        let items = knowledge.retrieve_for_context(query, None, domain);

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::SessionId;
    use y_knowledge::chunking::{Chunk, ChunkLevel, ChunkMetadata};
    use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};

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

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("knowledge_search"),
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
        assert_eq!(def.name.as_str(), "knowledge_search");
        assert_eq!(def.category, ToolCategory::Knowledge);
        assert!(!def.is_dangerous);
    }
}
