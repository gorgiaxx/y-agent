//! `file_search` built-in tool: search file contents by pattern.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum number of matches to return.
const MAX_MATCHES: usize = 50;

/// Built-in tool for searching file contents.
pub struct FileSearchTool {
    def: ToolDefinition,
}

impl FileSearchTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("file_search"),
            description: "Search for a text pattern in files within a directory. Returns \
                          matching file paths and line numbers. Useful for finding code patterns, \
                          function definitions, or specific text across a codebase."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Text pattern to search for (case-sensitive substring match)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file path to search in"
                    },
                    "include_ext": {
                        "type": "string",
                        "description": "Only search files with this extension (e.g. 'rs', 'py')"
                    }
                },
                "required": ["pattern", "path"]
            }),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    fn search_file(
        path: &Path,
        pattern: &str,
        results: &mut Vec<serde_json::Value>,
    ) -> Result<(), std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        for (line_num, line) in content.lines().enumerate() {
            if results.len() >= MAX_MATCHES {
                break;
            }
            if line.contains(pattern) {
                results.push(serde_json::json!({
                    "file": path.display().to_string(),
                    "line": line_num + 1,
                    "content": line.trim(),
                }));
            }
        }
        Ok(())
    }

    fn search_dir(
        path: &Path,
        pattern: &str,
        ext_filter: Option<&str>,
        results: &mut Vec<serde_json::Value>,
    ) {
        if results.len() >= MAX_MATCHES {
            return;
        }

        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            if results.len() >= MAX_MATCHES {
                break;
            }

            let entry_path = entry.path();

            // Skip hidden files/dirs.
            if entry_path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }

            if entry_path.is_dir() {
                Self::search_dir(&entry_path, pattern, ext_filter, results);
            } else if entry_path.is_file() {
                // Apply extension filter.
                if let Some(ext) = ext_filter {
                    let file_ext = entry_path
                        .extension()
                        .map(|e| e.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if file_ext != ext {
                        continue;
                    }
                }

                // Skip binary files (heuristic: check first 512 bytes).
                if let Ok(bytes) = std::fs::read(&entry_path) {
                    let check_len = bytes.len().min(512);
                    if bytes[..check_len].contains(&0) {
                        continue;
                    }
                }

                let _ = Self::search_file(&entry_path, pattern, results);
            }
        }
    }
}

impl Default for FileSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileSearchTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let pattern =
            input.arguments["pattern"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'pattern' parameter".into(),
                })?;

        let path_str =
            input.arguments["path"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'path' parameter".into(),
                })?;

        let ext_filter = input.arguments["include_ext"]
            .as_str()
            .map(std::string::ToString::to_string);

        let path = Path::new(path_str);

        if !path.exists() {
            return Err(ToolError::Other {
                message: format!("path does not exist: '{path_str}'"),
            });
        }

        // Run synchronous search in a blocking task.
        let pattern = pattern.to_string();
        let path = path.to_path_buf();
        let results = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            if path.is_file() {
                let _ = Self::search_file(&path, &pattern, &mut results);
            } else {
                Self::search_dir(&path, &pattern, ext_filter.as_deref(), &mut results);
            }
            results
        })
        .await
        .map_err(|e| ToolError::Other {
            message: format!("search task failed: {e}"),
        })?;

        let truncated = results.len() >= MAX_MATCHES;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "matches": results,
                "count": results.len(),
                "truncated": truncated,
            }),
            warnings: if truncated {
                vec![format!("Results capped at {} matches", MAX_MATCHES)]
            } else {
                vec![]
            },
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
            name: ToolName::from_string("file_search"),
            arguments: args,
            session_id: SessionId::new(),
        }
    }

    #[tokio::test]
    async fn test_file_search_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();

        let tool = FileSearchTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "println",
            "path": file_path.to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"], 1);
    }

    #[tokio::test]
    async fn test_file_search_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn hello() {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn world() {}").unwrap();
        std::fs::write(dir.path().join("c.txt"), "fn hello() {}").unwrap();

        let tool = FileSearchTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "fn hello",
            "path": dir.path().to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"], 2);
    }

    #[tokio::test]
    async fn test_file_search_with_ext_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn hello() {}").unwrap();
        std::fs::write(dir.path().join("b.txt"), "fn hello() {}").unwrap();

        let tool = FileSearchTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "fn hello",
            "path": dir.path().to_str().unwrap(),
            "include_ext": "rs"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"], 1);
    }

    #[tokio::test]
    async fn test_file_search_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();

        let tool = FileSearchTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "nonexistent_pattern_xyz",
            "path": dir.path().to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["count"], 0);
    }

    #[test]
    fn test_file_search_definition() {
        let def = FileSearchTool::tool_definition();
        assert_eq!(def.name.as_str(), "file_search");
        assert!(!def.is_dangerous);
    }
}
