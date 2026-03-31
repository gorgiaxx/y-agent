//! `Grep` built-in tool: powerful search tool built on ripgrep.
//!
//! Uses ripgrep (`rg`) under the hood for performant file content matching.
//! Supports full regex syntax, file type filtering, and multiline matching.
//!
//! Reference: cursor Grep.js tool implementation.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 20_000;

/// Built-in Grep tool for file content searching.
///
/// Accepts a regex `pattern` and multiple optional arguments like `path`, `Glob`, `type`, etc.
/// Delegates to ripgrep for fast, ignore-aware file content searching.
pub struct GrepTool {
    def: ToolDefinition,
}

impl GrepTool {
    /// Create a new `GrepTool`.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `Grep`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("Grep"),
            description: "A powerful search tool built on ripgrep.\n\n\
                Usage:\n\
                - ALWAYS use Grep for search tasks. NEVER invoke `Grep` or `rg` as a Bash command. The Grep tool has been optimized for correct permissions and access.\n\
                - Supports full regex syntax (e.g., \"log.*Error\", \"function\\\\s+\\\\w+\")\n\
                - Filter files with Glob parameter (e.g., \"*.js\", \"**/*.tsx\") or type parameter (e.g., \"js\", \"py\", \"rust\")\n\
                - Output modes: \"content\" shows matching lines, \"files_with_matches\" shows only file paths (default), \"count\" shows match counts\n\
                - Use Search tool for open-ended searches requiring multiple rounds\n\
                - Pattern syntax: Uses ripgrep (not grep) - literal braces need escaping (use `interface\\{\\}` to find `interface{}` in Go code)\n\
                - Multiline matching: By default patterns match within single lines only. For cross-line patterns like `struct \\{[\\s\\S]*?field`, use `multiline: true`"
                .into(),
            help: Some(
                "Use this tool to search file contents using regex.\n\
                 Provides functionality equivalent to ripgrep (rg)."
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The regular expression pattern to search for in file contents"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in (rg PATH). Defaults to current working directory."
                    },
                    "Glob": {
                        "type": "string",
                        "description": "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\") - maps to rg --Glob"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "Output mode: \"content\" shows matching lines (supports -A/-B/-C context, -n line numbers, head_limit), \"files_with_matches\" shows file paths (supports head_limit), \"count\" shows match counts (supports head_limit). Defaults to \"files_with_matches\"."
                    },
                    "-B": {
                        "type": "number",
                        "description": "Number of lines to show before each match (rg -B). Requires output_mode: \"content\", ignored otherwise."
                    },
                    "-A": {
                        "type": "number",
                        "description": "Number of lines to show after each match (rg -A). Requires output_mode: \"content\", ignored otherwise."
                    },
                    "-C": {
                        "type": "number",
                        "description": "Alias for context."
                    },
                    "context": {
                        "type": "number",
                        "description": "Number of lines to show before and after each match (rg -C). Requires output_mode: \"content\", ignored otherwise."
                    },
                    "-n": {
                        "type": "boolean",
                        "description": "Show line numbers in output (rg -n). Requires output_mode: \"content\", ignored otherwise. Defaults to true."
                    },
                    "-i": {
                        "type": "boolean",
                        "description": "Case insensitive search (rg -i)"
                    },
                    "type": {
                        "type": "string",
                        "description": "File type to search (rg --type). Common types: js, py, rust, go, java, etc. More efficient than include for standard file types."
                    },
                    "head_limit": {
                        "type": "number",
                        "description": "Limit output to first N lines/entries, equivalent to \"| head -N\". Works across all output modes: content (limits output lines), files_with_matches (limits file paths), count (limits count entries). Defaults to 250 when unspecified. Pass 0 for unlimited (use sparingly — large result sets waste context)."
                    },
                    "offset": {
                        "type": "number",
                        "description": "Skip first N lines/entries before applying head_limit, equivalent to \"| tail -n +N | head -N\". Works across all output modes. Defaults to 0."
                    },
                    "multiline": {
                        "type": "boolean",
                        "description": "Enable multiline mode where . matches newlines and patterns can span lines (rg -U --multiline-dotall). Default: false."
                    }
                },
                "required": ["pattern"]
            }),
            result_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "description": "Output mode used for formatting the result"
                    },
                    "numFiles": {
                        "type": "integer",
                        "description": "Number of matching files"
                    },
                    "filenames": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of matched file names, empty when mode is content or count"
                    },
                    "content": {
                        "type": "string",
                        "description": "Matched contents or aggregated output based on output format"
                    },
                    "numLines": {
                        "type": "integer",
                        "description": "Number of matched lines (content mode only)"
                    },
                    "numMatches": {
                        "type": "integer",
                        "description": "Total number of matches (count mode only)"
                    },
                    "appliedLimit": {
                        "type": "integer",
                        "description": "The head_limit applied, if any"
                    },
                    "appliedOffset": {
                        "type": "integer",
                        "description": "The offset applied, if any"
                    }
                }
            })),
            category: ToolCategory::Search,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GrepTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let pattern = input
            .arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'pattern' is required".into(),
            })?;

        let _search_path = input.arguments.get("path").and_then(|v| v.as_str());

        // -- Behavior placeholder --
        // Actual ripgrep execution will be implemented later.
        // For now, return the planned execution descriptor so the orchestrator
        // and tests can validate the tool's argument preparation logic.
        let _ = MAX_RESULT_SIZE_CHARS;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "grep_search",
                "pattern": pattern,
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
    use super::*;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_grep_001".into(),
            name: ToolName::from_string("Grep"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_grep_basic_pattern() {
        let tool = GrepTool::new();
        let input = make_input(serde_json::json!({"pattern": "fn main"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "grep_search");
        assert_eq!(output.content["pattern"], "fn main");
    }

    #[tokio::test]
    async fn test_grep_missing_pattern_fails() {
        let tool = GrepTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_grep_definition() {
        let def = GrepTool::tool_definition();
        assert_eq!(def.name.as_str(), "Grep");
        assert_eq!(def.category, ToolCategory::Search);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("pattern"));
        assert!(props.contains_key("path"));
        assert!(props.contains_key("Glob"));
        assert!(props.contains_key("output_mode"));
        assert!(props.contains_key("-B"));
        let required = def.parameters["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }
}
