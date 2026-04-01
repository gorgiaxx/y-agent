//! `ToolSearch` meta-tool: unified capability discovery.
//!
//! This is the primary mechanism for lazy tool loading and capability
//! discovery. The LLM sees a compact taxonomy root and calls `ToolSearch`
//! to retrieve definitions for the capabilities it needs.
//!
//! Supports three discovery modes:
//! - `category` -- browse the taxonomy tree by category path
//! - `tool` -- get the full schema of a specific tool by name
//! - `query` -- keyword search across all tools, skills, and agents

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// The `ToolSearch` meta-tool.
///
/// When invoked by the LLM, it returns matching tool definitions from
/// the registry, which are then added to the active set.
pub struct ToolSearchTool {
    def: ToolDefinition,
}

impl ToolSearchTool {
    /// Create a new `ToolSearch` tool.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `ToolSearch`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ToolSearch"),
            description: "Discover capabilities (tools, skills, agents) by keyword, \
                category, or name."
                .into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword search across tools, skills, and agents"
                    },
                    "category": {
                        "type": "string",
                        "description": "Category path to browse (e.g., 'file', 'shell', 'meta')"
                    },
                    "tool": {
                        "type": "string",
                        "description": "Specific tool name to retrieve full schema for"
                    }
                }
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
        let query = input.arguments.get("query").and_then(|v| v.as_str());
        let category = input.arguments.get("category").and_then(|v| v.as_str());
        let tool = input.arguments.get("tool").and_then(|v| v.as_str());

        // At least one parameter must be provided.
        if query.is_none() && category.is_none() && tool.is_none() {
            return Err(ToolError::ValidationError {
                message: "at least one of 'query', 'category', or 'tool' must be provided".into(),
            });
        }

        // Determine the search action type for the orchestrator.
        let action = if tool.is_some() {
            "get_tool"
        } else if category.is_some() {
            "browse_category"
        } else {
            "search"
        };

        // The actual search/lookup is performed externally by the orchestrator,
        // which has access to the registry and taxonomy. This tool validates
        // input and returns a descriptor indicating what should be performed.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": action,
                "query": query,
                "category": category,
                "tool": tool,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
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
    use y_core::types::SessionId;

    use super::*;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("ToolSearch"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_tool_search_with_query() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({"query": "file"}));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["action"], "search");
        assert_eq!(output.content["query"], "file");
    }

    #[tokio::test]
    async fn test_tool_search_with_category() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({"category": "file"}));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["action"], "browse_category");
        assert_eq!(output.content["category"], "file");
    }

    #[tokio::test]
    async fn test_tool_search_with_tool_name() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({"tool": "FileRead"}));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["action"], "get_tool");
        assert_eq!(output.content["tool"], "FileRead");
    }

    #[tokio::test]
    async fn test_tool_search_no_params_fails() {
        let tool = ToolSearchTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_tool_search_definition() {
        let def = ToolSearchTool::tool_definition();
        assert_eq!(def.name.as_str(), "ToolSearch");
        assert_eq!(def.category, ToolCategory::Custom);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        // Should have query, category, and tool properties.
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("query"));
        assert!(props.contains_key("category"));
        assert!(props.contains_key("tool"));
    }
}
