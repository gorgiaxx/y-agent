//! `FileRead` built-in tool: read file contents from the filesystem.

use async_trait::async_trait;
use std::path::Path;
use tokio::io::AsyncReadExt;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Default and maximum number of lines returned by one `FileRead` call.
const MAX_LINES_TO_READ: usize = 2_000;
/// Approximate token ceiling for returned file content.
const MAX_OUTPUT_TOKENS: usize = 25_000;
/// Simple output token estimate used across context code.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
/// Maximum result size in bytes/chars returned to the LLM after numbering.
const MAX_RESULT_SIZE_CHARS: usize = MAX_OUTPUT_TOKENS * CHARS_PER_TOKEN_ESTIMATE;
/// Whole-file read cap when no explicit range is requested.
const MAX_FILE_READ_SIZE_BYTES: u64 = 256 * 1024;
/// Above this size the reader switches to line-oriented streaming.
const FAST_PATH_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024;
/// Streaming read chunk size.
const STREAM_CHUNK_SIZE: usize = 512 * 1024;
/// Guidance shared by output-limit errors.
const RANGE_OR_SEARCH_HINT: &str = "Use line_offset/offset and limit to read a specific portion, \
    or use Grep to search for specific content before reading the file.";

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
            description: format!(
                "Read file contents at a given path. Returns numbered text and reads at most \
                 {MAX_LINES_TO_READ} lines per call by default. When the target location is \
                 known, pass line_offset (or offset) and limit; for broad lookup, use Glob/Grep \
                 first and then FileRead the specific range."
            ),
            help: Some(format!(
                "Use FileRead for targeted file inspection.\n\
                 - Results are returned with 1-based line numbers.\n\
                 - By default, FileRead returns up to {MAX_LINES_TO_READ} lines from the start.\n\
                 - Use line_offset (0-based) and limit to read specific chunks of larger files.\n\
                 - If you are looking for a symbol, string, or section, use Grep first and then \
                   read only the matching range.\n\
                 - A single call may not request more than {MAX_LINES_TO_READ} lines."
            )),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file to read"
                    },
                    "line_offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Optional line number to start reading from (0-indexed). Defaults to 0."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Alias for line_offset, matching common range-read conventions."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_LINES_TO_READ,
                        "description": "Optional maximum number of lines to read. Defaults to 2000 and cannot exceed 2000."
                    }
                },
                "required": ["path"]
            }),
            result_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "lines": { "type": "integer" },
                    "total_lines": { "type": "integer" },
                    "line_offset": { "type": "integer" },
                    "limit": { "type": "integer" },
                    "encoding": { "type": "string" },
                    "truncated": { "type": "boolean" },
                    "has_more_lines": { "type": "boolean" }
                }
            })),
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

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| file_io_error(&canonical, "read metadata", &e))?;
        if metadata.is_dir() {
            return Err(ToolError::Other {
                message: format!("cannot read directory '{}'", canonical.display()),
            });
        }

        let range = ReadRange::from_arguments(&input.arguments)?;
        if !range.explicit_range && metadata.len() > MAX_FILE_READ_SIZE_BYTES {
            return Err(ToolError::Other {
                message: format!(
                    "File content ({}) exceeds maximum allowed size ({}). {RANGE_OR_SEARCH_HINT}",
                    format_file_size(metadata.len()),
                    format_file_size(MAX_FILE_READ_SIZE_BYTES)
                ),
            });
        }

        let read = read_file_range_as_utf8_impl(&canonical, &metadata, range).await?;
        let numbered_content = format_numbered_lines(&read.lines, range.offset);
        let token_count = estimate_output_tokens(&numbered_content);
        if token_count > MAX_OUTPUT_TOKENS {
            return Err(ToolError::Other {
                message: format!(
                    "File content ({token_count} tokens estimated) exceeds maximum allowed tokens \
                     ({MAX_OUTPUT_TOKENS}). {RANGE_OR_SEARCH_HINT}"
                ),
            });
        }

        let (content, char_truncated) = truncate_content(&numbered_content);
        let line_count = read.lines.len();
        let has_more_lines = range.offset.saturating_add(line_count) < read.total_lines;
        let auto_truncated = range.default_limit_applied && has_more_lines;

        let mut result = serde_json::json!({
            "path": canonical.display().to_string(),
            "content": content,
            "lines": line_count,
            "total_lines": read.total_lines,
            "line_offset": range.offset,
            "limit": range.limit,
            "encoding": read.encoding,
            "total_bytes": read.total_bytes,
            "read_bytes": read.read_bytes,
            "has_more_lines": has_more_lines,
            "default_limit_applied": range.default_limit_applied,
        });
        if auto_truncated || char_truncated {
            result["truncated"] = serde_json::json!(true);
        }

        let warnings = if auto_truncated {
            vec![format!(
                "FileRead returned the first {MAX_LINES_TO_READ} lines. Use line_offset and limit, \
                 or Grep first, to inspect later content."
            )]
        } else {
            Vec::new()
        };

        Ok(ToolOutput {
            success: true,
            content: result,
            warnings,
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

#[derive(Debug, Clone, Copy)]
struct ReadRange {
    offset: usize,
    limit: usize,
    explicit_range: bool,
    default_limit_applied: bool,
}

impl ReadRange {
    fn from_arguments(arguments: &serde_json::Value) -> Result<Self, ToolError> {
        let line_offset = parse_usize_arg(arguments, "line_offset")?;
        let offset_alias = parse_usize_arg(arguments, "offset")?;
        let offset = line_offset.or(offset_alias).unwrap_or(0);
        let explicit_limit = parse_usize_arg(arguments, "limit")?;
        let explicit_range =
            line_offset.is_some() || offset_alias.is_some() || explicit_limit.is_some();
        let limit = explicit_limit.unwrap_or(MAX_LINES_TO_READ);

        if limit == 0 {
            return Err(ToolError::ValidationError {
                message: "'limit' must be greater than 0".into(),
            });
        }
        if limit > MAX_LINES_TO_READ {
            return Err(ToolError::ValidationError {
                message: format!(
                    "'limit' must be at most {MAX_LINES_TO_READ}; read larger files in chunks with \
                     line_offset and limit"
                ),
            });
        }

        Ok(Self {
            offset,
            limit,
            explicit_range,
            default_limit_applied: explicit_limit.is_none(),
        })
    }
}

#[derive(Debug)]
struct FileRangeRead {
    lines: Vec<String>,
    total_lines: usize,
    total_bytes: u64,
    read_bytes: usize,
    encoding: &'static str,
}

/// Internal helper to read only the requested line range while bounding returned content.
async fn read_file_range_as_utf8_impl(
    path: &std::path::Path,
    metadata: &std::fs::Metadata,
    range: ReadRange,
) -> Result<FileRangeRead, ToolError> {
    if metadata.len() < FAST_PATH_MAX_SIZE_BYTES {
        read_file_range_fast(path, metadata, range).await
    } else {
        read_file_range_streaming(path, metadata, range).await
    }
}

async fn read_file_range_fast(
    path: &std::path::Path,
    metadata: &std::fs::Metadata,
    range: ReadRange,
) -> Result<FileRangeRead, ToolError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| file_io_error(path, "read", &e))?;
    let (content, encoding) = decode_bytes_to_utf8(&bytes, path);

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start = std::cmp::min(range.offset, total_lines);
    let end = std::cmp::min(start.saturating_add(range.limit), total_lines);
    let selected = lines[start..end]
        .iter()
        .map(|line| (*line).to_string())
        .collect::<Vec<_>>();
    let read_bytes = selected_output_bytes(&selected);
    enforce_selected_byte_budget(read_bytes)?;

    Ok(FileRangeRead {
        lines: selected,
        total_lines,
        total_bytes: metadata.len(),
        read_bytes,
        encoding,
    })
}

/// Decode bytes to UTF-8 using `chardetng` if necessary.
fn decode_bytes_to_utf8(bytes: &[u8], path: &std::path::Path) -> (String, &'static str) {
    let bytes = strip_utf8_bom(bytes);

    // Fast path: if it's already valid UTF-8, return directly.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), "UTF-8");
    }

    // Detect encoding using chardetng.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);

    let (cow, actual_encoding, had_errors) = encoding.decode(bytes);

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

    (cow.into_owned(), actual_encoding.name())
}

async fn read_file_range_streaming(
    path: &std::path::Path,
    metadata: &std::fs::Metadata,
    range: ReadRange,
) -> Result<FileRangeRead, ToolError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| file_io_error(path, "open", &e))?;
    let mut buffer = vec![0_u8; STREAM_CHUNK_SIZE];
    let mut selected_lines = Vec::new();
    let mut partial = Vec::new();
    let mut total_bytes = 0_u64;
    let mut selected_bytes = 0_usize;
    let mut line_index = 0_usize;
    let mut first_chunk = true;
    let mut last_byte_was_newline = false;
    let mut lossy = false;

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .await
            .map_err(|e| file_io_error(path, "read", &e))?;
        if bytes_read == 0 {
            break;
        }

        let mut start = 0;
        let chunk = &buffer[..bytes_read];
        total_bytes = total_bytes.saturating_add(bytes_read as u64);
        last_byte_was_newline = chunk.last().is_some_and(|b| *b == b'\n');

        if first_chunk {
            first_chunk = false;
            if chunk.starts_with(&[0xEF, 0xBB, 0xBF]) {
                start = 3;
            }
        }

        for (idx, byte) in chunk.iter().enumerate().skip(start) {
            if *byte != b'\n' {
                continue;
            }

            if line_in_range(line_index, range) {
                partial.extend_from_slice(&chunk[start..idx]);
                push_selected_line(
                    &mut selected_lines,
                    &mut selected_bytes,
                    &mut partial,
                    &mut lossy,
                )?;
            } else {
                partial.clear();
            }

            line_index = line_index.saturating_add(1);
            start = idx + 1;
        }

        if start < bytes_read {
            if line_in_range(line_index, range) {
                partial.extend_from_slice(&chunk[start..bytes_read]);
                enforce_partial_byte_budget(selected_lines.len(), selected_bytes, partial.len())?;
            } else {
                partial.clear();
            }
        }
    }

    if total_bytes > 0 && !last_byte_was_newline {
        if line_in_range(line_index, range) {
            push_selected_line(
                &mut selected_lines,
                &mut selected_bytes,
                &mut partial,
                &mut lossy,
            )?;
        }
        line_index = line_index.saturating_add(1);
    }

    Ok(FileRangeRead {
        lines: selected_lines,
        total_lines: line_index,
        total_bytes: metadata.len(),
        read_bytes: selected_bytes,
        encoding: if lossy { "UTF-8 (lossy)" } else { "UTF-8" },
    })
}

fn parse_usize_arg(arguments: &serde_json::Value, name: &str) -> Result<Option<usize>, ToolError> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(raw) = value.as_u64() else {
        return Err(ToolError::ValidationError {
            message: format!("'{name}' must be a non-negative integer"),
        });
    };
    usize::try_from(raw)
        .map(Some)
        .map_err(|_| ToolError::ValidationError {
            message: format!("'{name}' is too large"),
        })
}

fn line_in_range(line_index: usize, range: ReadRange) -> bool {
    line_index >= range.offset && line_index < range.offset.saturating_add(range.limit)
}

fn push_selected_line(
    selected_lines: &mut Vec<String>,
    selected_bytes: &mut usize,
    partial: &mut Vec<u8>,
    lossy: &mut bool,
) -> Result<(), ToolError> {
    if partial.ends_with(b"\r") {
        partial.pop();
    }

    let sep = usize::from(!selected_lines.is_empty());
    let next_bytes = selected_bytes
        .saturating_add(sep)
        .saturating_add(partial.len());
    enforce_selected_byte_budget(next_bytes)?;
    *selected_bytes = next_bytes;

    let decoded = String::from_utf8(std::mem::take(partial)).unwrap_or_else(|err| {
        *lossy = true;
        String::from_utf8_lossy(err.as_bytes()).into_owned()
    });
    selected_lines.push(decoded);
    Ok(())
}

fn enforce_partial_byte_budget(
    selected_line_count: usize,
    selected_bytes: usize,
    partial_len: usize,
) -> Result<(), ToolError> {
    let sep = usize::from(selected_line_count > 0);
    enforce_selected_byte_budget(
        selected_bytes
            .saturating_add(sep)
            .saturating_add(partial_len),
    )
}

fn enforce_selected_byte_budget(bytes: usize) -> Result<(), ToolError> {
    if bytes > MAX_FILE_READ_SIZE_BYTES as usize {
        return Err(ToolError::Other {
            message: format!(
                "Selected file content ({}) exceeds maximum allowed size ({}). \
                 {RANGE_OR_SEARCH_HINT}",
                format_file_size(bytes as u64),
                format_file_size(MAX_FILE_READ_SIZE_BYTES)
            ),
        });
    }
    Ok(())
}

fn selected_output_bytes(lines: &[String]) -> usize {
    lines
        .iter()
        .map(String::len)
        .sum::<usize>()
        .saturating_add(lines.len().saturating_sub(1))
}

fn format_numbered_lines(lines: &[String], offset: usize) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}\t{line}", offset + i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn estimate_output_tokens(content: &str) -> usize {
    content.chars().count().div_ceil(CHARS_PER_TOKEN_ESTIMATE)
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn file_io_error(path: &std::path::Path, action: &str, error: &std::io::Error) -> ToolError {
    ToolError::Other {
        message: format!("failed to {action} '{}': {error}", path.display()),
    }
}

fn format_file_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;

    if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} bytes")
    }
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
            working_dir: None,
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
    async fn test_file_read_default_caps_at_max_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("long.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        for i in 1..=(MAX_LINES_TO_READ + 5) {
            writeln!(f, "line {i}").unwrap();
        }

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap()
        }));
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(
            output.content["lines"].as_u64().unwrap(),
            MAX_LINES_TO_READ as u64
        );
        assert_eq!(
            output.content["total_lines"].as_u64().unwrap(),
            (MAX_LINES_TO_READ + 5) as u64
        );
        assert_eq!(output.content["truncated"].as_bool(), Some(true));
        let text = output.content["content"].as_str().unwrap();
        assert!(text.contains("2000\tline 2000"));
        assert!(!text.contains("2001\tline 2001"));
    }

    #[tokio::test]
    async fn test_file_read_whole_large_file_fails_with_search_hint() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("large.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        for i in 0..4000 {
            writeln!(f, "{i:04} {}", "x".repeat(80)).unwrap();
        }

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap()
        }));
        let err = tool.execute(input).await.unwrap_err().to_string();

        assert!(err.contains("exceeds maximum allowed size"));
        assert!(err.contains("line_offset"));
        assert!(err.contains("Grep"));
    }

    #[tokio::test]
    async fn test_file_read_targeted_range_can_read_large_file_slice() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("large_slice.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        for i in 0..4000 {
            writeln!(f, "{i:04} {}", "x".repeat(80)).unwrap();
        }

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "line_offset": 2500,
            "limit": 3
        }));
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["lines"].as_u64().unwrap(), 3);
        assert_eq!(output.content["total_lines"].as_u64().unwrap(), 4000);
        let text = output.content["content"].as_str().unwrap();
        assert!(text.contains("2501\t2500 "));
        assert!(text.contains("2503\t2502 "));
        assert!(!text.contains("2500\t2499 "));
        assert!(!text.contains("2504\t2503 "));
    }

    #[tokio::test]
    async fn test_file_read_offset_alias_reads_targeted_range() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("offset_alias.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "line 3").unwrap();

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "offset": 1,
            "limit": 1
        }));
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["lines"].as_u64().unwrap(), 1);
        let text = output.content["content"].as_str().unwrap();
        assert!(text.contains("2\tline 2"));
        assert!(!text.contains("1\tline 1"));
    }

    #[tokio::test]
    async fn test_file_read_token_budget_error_points_to_search() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("too_many_tokens.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "{}", "x".repeat((MAX_OUTPUT_TOKENS as usize + 1) * 4)).unwrap();

        let tool = FileReadTool::new();
        let input = make_input(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "limit": 1
        }));
        let err = tool.execute(input).await.unwrap_err().to_string();

        assert!(err.contains("exceeds maximum allowed tokens"));
        assert!(err.contains("Grep"));
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
        assert!(def.description.contains("2000"));
        assert!(def.description.contains("Grep"));
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("offset"));
        assert_eq!(props["limit"]["maximum"], MAX_LINES_TO_READ);
    }

    #[test]
    fn test_truncate_content_short() {
        let (result, truncated) = truncate_content("hello");
        assert_eq!(result, "hello");
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_content_exceeds_limit() {
        let s = "a".repeat(MAX_RESULT_SIZE_CHARS + 1);
        let (result, truncated) = truncate_content(&s);
        assert!(truncated);
        assert!(result.contains("[output truncated:"));
        assert!(result.len() < MAX_RESULT_SIZE_CHARS + 200);
    }

    #[test]
    fn test_truncate_content_multibyte() {
        let s = "你好".repeat(MAX_RESULT_SIZE_CHARS);
        let (result, truncated) = truncate_content(&s);
        assert!(truncated);
        assert!(result.contains("[output truncated:"));
    }
}
