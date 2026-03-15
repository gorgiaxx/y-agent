//! `file_read` built-in tool: read file contents from the filesystem.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Built-in tool for reading files.
pub struct FileReadTool {
    def: ToolDefinition,
}

impl FileReadTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("file_read"),
            description: "Read the contents of a file at the given path. Returns the file \
                          content as a string. Use this to examine source code, configuration \
                          files, or any text file."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file to read"
                    }
                },
                "required": ["path"]
            }),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileReadTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let path_str =
            input.arguments["path"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'path' parameter".into(),
                })?;

        let path = Path::new(path_str);

        // Resolve to canonical path for security.
        let canonical = path.canonicalize().map_err(|e| ToolError::Other {
            message: format!("cannot resolve path '{path_str}': {e}"),
        })?;

        // Read file content.
        let content =
            tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("failed to read '{}': {}", canonical.display(), e),
                })?;

        let line_count = content.lines().count();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "path": canonical.display().to_string(),
                "content": content,
                "lines": line_count,
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
    use std::io::Write;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("file_read"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_file_read_success() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert!(output.content["content"]
            .as_str()
            .unwrap()
            .contains("line 1"));
        assert_eq!(output.content["lines"], 2);
    }

    #[tokio::test]
    async fn test_file_read_missing_path_param() {
        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_read_nonexistent_file() {
        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": "/tmp/y_agent_nonexistent_file_12345.txt"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_file_read_definition() {
        let def = FileReadTool::tool_definition();
        assert_eq!(def.name.as_str(), "file_read");
        assert_eq!(def.category, ToolCategory::FileSystem);
        assert!(!def.is_dangerous);
    }
}
