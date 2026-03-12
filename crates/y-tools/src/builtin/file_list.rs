//! `file_list` built-in tool: list directory contents.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Built-in tool for listing directory contents.
pub struct FileListTool {
    def: ToolDefinition,
}

impl FileListTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("file_list"),
            description: "List the contents of a directory. Returns file and directory names \
                          with their types and sizes. Use this to explore the file system \
                          structure."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the directory to list (default: current directory)"
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

impl Default for FileListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileListTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let path_str =
            input.arguments["path"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'path' parameter".into(),
                })?;

        let path = Path::new(path_str);

        if !path.exists() {
            return Err(ToolError::Other {
                message: format!("path does not exist: '{path_str}'"),
            });
        }

        if !path.is_dir() {
            return Err(ToolError::Other {
                message: format!("path is not a directory: '{path_str}'"),
            });
        }

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(path)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("failed to read directory '{path_str}': {e}"),
            })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| ToolError::Other {
            message: format!("failed to read entry: {e}"),
        })? {
            let name = entry.file_name().to_string_lossy().to_string();
            let metadata = entry.metadata().await.ok();
            let file_type = if metadata.as_ref().is_some_and(std::fs::Metadata::is_dir) {
                "directory"
            } else if metadata.as_ref().is_some_and(std::fs::Metadata::is_symlink) {
                "symlink"
            } else {
                "file"
            };
            let size = metadata.as_ref().map_or(0, std::fs::Metadata::len);

            entries.push(serde_json::json!({
                "name": name,
                "type": file_type,
                "size": size,
            }));
        }

        // Sort by name for deterministic output.
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "path": path_str,
                "entries": entries,
                "count": entries.len(),
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
            name: ToolName::from_string("file_list"),
            arguments: args,
            session_id: SessionId::new(),
        }
    }

    #[tokio::test]
    async fn test_file_list_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = FileListTool::new();
        let input = make_input(serde_json::json!({
            "path": dir.path().to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"], 3);
    }

    #[tokio::test]
    async fn test_file_list_nonexistent() {
        let tool = FileListTool::new();
        let input = make_input(serde_json::json!({
            "path": "/tmp/y_agent_nonexistent_dir_12345"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_list_not_a_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();

        let tool = FileListTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap()
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_file_list_definition() {
        let def = FileListTool::tool_definition();
        assert_eq!(def.name.as_str(), "file_list");
        assert!(!def.is_dangerous);
    }
}
