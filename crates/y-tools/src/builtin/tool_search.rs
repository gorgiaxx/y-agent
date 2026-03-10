//! `tool_search` meta-tool: allows the LLM to search for and activate tools.
//!
//! This is the primary mechanism for lazy tool loading. The LLM sees
//! a compact index of all tools and calls `tool_search` to retrieve
//! full definitions for the tools it needs.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// The `tool_search` meta-tool.
///
/// When invoked by the LLM with a search query, it returns matching tool
/// definitions from the registry, which are then added to the active set.
pub struct ToolSearchTool {
    def: ToolDefinition,
}

impl ToolSearchTool {
    /// Create a new `tool_search` tool.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `tool_search`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("tool_search"),
            description: "Search for available tools by keyword or category. Returns full tool definitions that can be used in subsequent calls.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query to match tool names and descriptions"
                    },
                    "category": {
                        "type": "string",
                        "description": "Optional category filter (e.g., 'filesystem', 'network', 'shell')"
                    }
                },
                "required": ["query"]
            }),
            result_schema: None,
            category: ToolCategory::Custom,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for ToolSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let query = input.arguments["query"]
            .as_str()
            .ok_or_else(|| ToolError::ValidationError {
                message: "missing 'query' parameter".into(),
            })?;

        // The actual search is performed externally by the orchestrator,
        // which has access to the registry. This tool just validates input
        // and returns a placeholder indicating the search should be performed.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "search",
                "query": query,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    use y_core::types::SessionId;

    use super::*;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("tool_search"),
            arguments: args,
            session_id: SessionId::new(),
        }
    }

    #[tokio::test]
    async fn test_tool_search_returns_results_placeholder() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({"query": "file"}));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["action"], "search");
        assert_eq!(output.content["query"], "file");
    }

    #[tokio::test]
    async fn test_tool_search_missing_query_fails() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_search_definition() {
        let def = ToolSearchTool::tool_definition();
        assert_eq!(def.name.as_str(), "tool_search");
        assert_eq!(def.category, ToolCategory::Custom);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
    }
}
