//! `FileWrite` built-in tool: write content to a file.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Built-in tool for writing files.
pub struct FileWriteTool {
    def: ToolDefinition,
}

impl FileWriteTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("FileWrite"),
            description: "Write content to a file, creating parent directories as needed.".into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true,
        }
    }
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileWriteTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let path_str =
            input.arguments["path"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'path' parameter".into(),
                })?;

        let content =
            input.arguments["content"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'content' parameter".into(),
                })?;

        let path = Path::new(path_str);

        // Create parent directories if needed.
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("failed to create directory '{}': {}", parent.display(), e),
                })?;
        }

        // Write the file.
        tokio::fs::write(path, content)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("failed to write '{path_str}': {e}"),
            })?;

        let bytes_written = content.len();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "path": path_str,
                "bytes_written": bytes_written,
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
            call_id: "call_001".into(),
            name: ToolName::from_string("FileWrite"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_file_write_success() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("output.txt");

        let tool = FileWriteTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "content": "hello world"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["bytes_written"], 11);

        let read = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(read, "hello world");
    }

    #[tokio::test]
    async fn test_file_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("a").join("b").join("c.txt");

        let tool = FileWriteTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "content": "nested"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);

        let read = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(read, "nested");
    }

    #[tokio::test]
    async fn test_file_write_missing_content() {
        let tool = FileWriteTool::new();
        let input = make_input(serde_json::json!({"path": "/tmp/test.txt"}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_file_write_definition() {
        let def = FileWriteTool::tool_definition();
        assert_eq!(def.name.as_str(), "FileWrite");
        assert!(def.is_dangerous);
    }
}
