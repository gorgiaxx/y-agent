//! `FileRead` built-in tool: read file contents from the filesystem.

use async_trait::async_trait;
use std::path::Path;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 10_000;

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
            name: ToolName::from_string("FileRead"),
            description: "Read file contents at a given path.".into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file to read"
                    },
                    "line_offset": {
                        "type": "integer",
                        "description": "Optional line number to start reading from (0-indexed). Defaults to 0."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Optional maximum number of lines to read. Defaults to reading the entire file."
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

        // Read file content with encoding support natively.
        let (content, encoding) =
            read_file_as_utf8_impl(&canonical)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("failed to read '{}': {}", canonical.display(), e),
                })?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let offset = input
            .arguments
            .get("line_offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        let limit = input
            .arguments
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize);

        let start = std::cmp::min(offset, total_lines);
        let end = if let Some(l) = limit {
            std::cmp::min(start + l, total_lines)
        } else {
            total_lines
        };

        let sliced_content = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}\t{line}", start + i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let line_count = end - start;

        let (content, truncated) = truncate_content(&sliced_content);

        let mut result = serde_json::json!({
            "path": canonical.display().to_string(),
            "content": content,
            "lines": line_count,
            "total_lines": total_lines,
            "encoding": encoding,
        });
        if truncated {
            result["truncated"] = serde_json::json!(true);
        }

        Ok(ToolOutput {
            success: true,
            content: result,
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

/// Internal helper to read a file and decode to UTF-8 using `chardetng` if necessary.
async fn read_file_as_utf8_impl(
    path: &std::path::Path,
) -> Result<(String, &'static str), std::io::Error> {
    let bytes = tokio::fs::read(path).await?;

    // Fast path: if it's already valid UTF-8, return directly.
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Ok((s.to_string(), "UTF-8"));
    }

    // Detect encoding using chardetng.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(&bytes, true);
    let encoding = detector.guess(None, true);

    let (cow, actual_encoding, had_errors) = encoding.decode(&bytes);

    if had_errors {
        tracing::warn!(
            path = %path.display(),
            encoding = actual_encoding.name(),
            "encoding conversion had replacement characters"
        );
    }

    tracing::debug!(
        path = %path.display(),
        encoding = actual_encoding.name(),
        "converted file to UTF-8"
    );

    Ok((cow.into_owned(), actual_encoding.name()))
}

/// Truncate content to `MAX_RESULT_SIZE_CHARS`, returning (content, `was_truncated`).
fn truncate_content(content: &str) -> (String, bool) {
    if content.len() <= MAX_RESULT_SIZE_CHARS {
        return (content.to_string(), false);
    }
    let mut end = MAX_RESULT_SIZE_CHARS;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let total_chars = content.chars().count();
    (
        format!(
            "{}\n[output truncated: {total_chars} chars total, showing first {end}]",
            &content[..end]
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("FileRead"),
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
        let content_text = output.content["content"].as_str().unwrap();
        assert!(content_text.contains("1\tline 1"));
        assert!(content_text.contains("2\tline 2"));
        assert_eq!(output.content["lines"], 2);
        assert_eq!(output.content["total_lines"], 2);
        assert_eq!(output.content["encoding"], "UTF-8");
    }

    #[tokio::test]
    async fn test_file_read_with_offset_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test2.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "line 3").unwrap();
        writeln!(f, "line 4").unwrap();

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "line_offset": 1,
            "limit": 2
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        let content_obj = &output.content;
        assert_eq!(content_obj["lines"].as_u64().unwrap(), 2);
        assert_eq!(content_obj["total_lines"].as_u64().unwrap(), 4);
        let text = content_obj["content"].as_str().unwrap();
        assert!(text.contains("2\tline 2"));
        assert!(text.contains("3\tline 3"));
        assert!(!text.contains("1\tline 1"));
        assert!(!text.contains("4\tline 4"));
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
        assert_eq!(def.name.as_str(), "FileRead");
        assert_eq!(def.category, ToolCategory::FileSystem);
        assert!(!def.is_dangerous);
    }

    #[test]
    fn test_truncate_content_short() {
        let (result, truncated) = truncate_content("hello");
        assert_eq!(result, "hello");
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_content_exceeds_limit() {
        let s = "a".repeat(15_000);
        let (result, truncated) = truncate_content(&s);
        assert!(truncated);
        assert!(result.contains("[output truncated:"));
        assert!(result.len() < 10_200);
    }

    #[test]
    fn test_truncate_content_multibyte() {
        let s = "你好".repeat(10_000);
        let (result, truncated) = truncate_content(&s);
        assert!(truncated);
        assert!(result.contains("[output truncated:"));
    }
}
