//! `file_edit` built-in tool: perform exact string replacements in files.
//!
//! This tool replaces an exact substring (`old_string`) with a new string
//! (`new_string`) inside a file. When `old_string` is empty the tool creates
//! the file with `new_string` as content (parent directories are created
//! automatically).
//!
//! Behavior hooks (read-before-write enforcement, stale-data checks,
//! permission gates) are **not yet implemented** and will be added later.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Built-in tool for performing exact string replacements in files.
pub struct FileEditTool {
    def: ToolDefinition,
}

impl FileEditTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("FileEdit"),
            description: concat!(
                "Perform exact string replacements in files. ",
                "Replace occurrences of `old_string` with `new_string` in the specified file. ",
                "When `old_string` is empty and the file does not exist, the file is created ",
                "with `new_string` as content.",
            )
            .into(),
            help: Some(TOOL_HELP.into()),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": concat!(
                            "The exact string to search for in the file. ",
                            "Must match file content exactly (including whitespace and indentation). ",
                            "If empty, the tool creates a new file with `new_string` as content."
                        )
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement string that will replace `old_string`"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": concat!(
                            "If true, replace all occurrences of `old_string` in the file. ",
                            "If false (default), the edit will fail when multiple matches exist."
                        ),
                        "default": false
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true,
        }
    }
}

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FileEditTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let file_path =
            input.arguments["file_path"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'file_path' parameter".into(),
                })?;

        let old_string =
            input.arguments["old_string"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'old_string' parameter".into(),
                })?;

        let new_string =
            input.arguments["new_string"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'new_string' parameter".into(),
                })?;

        let replace_all = input
            .arguments
            .get("replace_all")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // Reject no-ops where old and new are identical.
        if old_string == new_string {
            return Err(ToolError::ValidationError {
                message: "no changes to make: old_string and new_string are identical".into(),
            });
        }

        let path = Path::new(file_path);

        // --- File creation path (old_string is empty) ---
        if old_string.is_empty() {
            return self.create_file(path, file_path, new_string).await;
        }

        // --- Edit path ---
        self.edit_file(path, file_path, old_string, new_string, replace_all)
            .await
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

impl FileEditTool {
    /// Create a new file, failing if the file already has non-empty content.
    async fn create_file(
        &self,
        path: &Path,
        file_path: &str,
        new_string: &str,
    ) -> Result<ToolOutput, ToolError> {
        // Check whether the file already exists with content.
        match tokio::fs::read_to_string(path).await {
            Ok(existing) if !existing.trim().is_empty() => {
                return Err(ToolError::ValidationError {
                    message: format!(
                        "cannot create new file -- '{file_path}' already exists with content",
                    ),
                });
            }
            Ok(_) | Err(_) => { /* file is empty or does not exist -- proceed */ }
        }

        // Ensure parent directories exist.
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("failed to create directory '{}': {e}", parent.display(),),
                })?;
        }

        tokio::fs::write(path, new_string)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("failed to write '{file_path}': {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "file_path": file_path,
                "action": "created",
                "bytes_written": new_string.len(),
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Edit an existing file by replacing exact string matches.
    async fn edit_file(
        &self,
        path: &Path,
        file_path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<ToolOutput, ToolError> {
        // Read current content (normalise line endings to LF).
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("failed to read '{file_path}': {e}"),
            })?;
        let content = content.replace("\r\n", "\n");

        // Count occurrences of old_string.
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Err(ToolError::ValidationError {
                message: format!(
                    "string to replace not found in file '{file_path}'.\nString: {old_string}",
                ),
            });
        }

        if match_count > 1 && !replace_all {
            return Err(ToolError::ValidationError {
                message: format!(
                    "found {match_count} matches of the string to replace in '{file_path}', \
                     but replace_all is false. Provide more surrounding context to make it \
                     unique, or set replace_all to true.\nString: {old_string}",
                ),
            });
        }

        // Perform the replacement.
        let updated = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back.
        tokio::fs::write(path, &updated)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("failed to write '{file_path}': {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "file_path": file_path,
                "action": "edited",
                "replacements": if replace_all { match_count } else { 1 },
                "replace_all": replace_all,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }
}

// ---------------------------------------------------------------------------
// Help text
// ---------------------------------------------------------------------------

const TOOL_HELP: &str = "\
Performs exact string replacements in files.

Usage notes:
- You must read the target file (e.g. with `FileRead`) before editing. \
  Ensure you preserve the exact indentation (tabs/spaces) as it appears in the \
  file content. Never include line-number prefixes in old_string or new_string.
- Prefer editing existing files. Only create new files when explicitly required.
- The edit will FAIL if `old_string` is not unique in the file. Either provide \
  a larger string with more surrounding context to make it unique, or set \
  `replace_all` to true.
- Use `replace_all` for bulk renaming (e.g. renaming a variable across the \
  entire file).
- When `old_string` is empty, the tool creates the file with `new_string` as \
  content (fails if the file already has non-empty content).";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("FileEdit"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    // -- Successful edits --

    #[tokio::test]
    async fn test_single_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["replacements"], 1);

        let result = std::fs::read_to_string(&file).unwrap();
        assert_eq!(result, "goodbye world");
    }

    #[tokio::test]
    async fn test_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "aaa bbb aaa ccc aaa").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "aaa",
            "new_string": "xxx",
            "replace_all": true
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["replacements"], 3);

        let result = std::fs::read_to_string(&file).unwrap();
        assert_eq!(result, "xxx bbb xxx ccc xxx");
    }

    #[tokio::test]
    async fn test_multiline_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        let original = "fn main() {\n    println!(\"old\");\n}\n";
        std::fs::write(&file, original).unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "    println!(\"old\");",
            "new_string": "    println!(\"new\");\n    eprintln!(\"debug\");"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);

        let result = std::fs::read_to_string(&file).unwrap();
        assert!(result.contains("println!(\"new\")"));
        assert!(result.contains("eprintln!(\"debug\")"));
    }

    // -- File creation --

    #[tokio::test]
    async fn test_create_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new_file.txt");

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "",
            "new_string": "brand new content"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "created");

        let result = std::fs::read_to_string(&file).unwrap();
        assert_eq!(result, "brand new content");
    }

    #[tokio::test]
    async fn test_create_new_file_with_nested_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a").join("b").join("c.txt");

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "",
            "new_string": "nested content"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);

        let result = std::fs::read_to_string(&file).unwrap();
        assert_eq!(result, "nested content");
    }

    #[tokio::test]
    async fn test_create_fails_if_file_has_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("existing.txt");
        std::fs::write(&file, "I exist").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "",
            "new_string": "overwrite attempt"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    // -- Error conditions --

    #[tokio::test]
    async fn test_no_op_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "same",
            "new_string": "same"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_string_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "nonexistent",
            "new_string": "replacement"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multiple_matches_without_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "aaa bbb aaa").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "aaa",
            "new_string": "xxx"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_not_found_for_edit() {
        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": "/tmp/y_agent_nonexistent_edit_98765.txt",
            "old_string": "something",
            "new_string": "else"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_missing_file_path_param() {
        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "old_string": "a",
            "new_string": "b"
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    // -- Definition --

    #[test]
    fn test_definition() {
        let def = FileEditTool::tool_definition();
        assert_eq!(def.name.as_str(), "FileEdit");
        assert_eq!(def.category, ToolCategory::FileSystem);
        assert!(def.is_dangerous);
        assert!(def.help.is_some());
    }

    // -- CRLF normalisation --

    #[tokio::test]
    async fn test_crlf_normalised() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("crlf.txt");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            f.write_all(b"line1\r\nline2\r\nline3").unwrap();
        }

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "line2",
            "new_string": "replaced"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);

        let result = std::fs::read_to_string(&file).unwrap();
        assert!(result.contains("replaced"));
    }
}
