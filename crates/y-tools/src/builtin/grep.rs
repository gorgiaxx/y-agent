//! `Grep` built-in tool: powerful search tool built on ripgrep.
//!
//! Uses ripgrep (`rg`) under the hood for performant file content matching.
//! Supports full regex syntax, file type filtering, and multiline matching.
//!
//! Reference: cursor Grep.js tool implementation.

use std::time::Duration;

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 20_000;

/// Default `head_limit` when unspecified.
const DEFAULT_HEAD_LIMIT: u64 = 250;

/// Default timeout for the ripgrep subprocess (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Built-in Grep tool for file content searching.
///
/// Accepts a regex `pattern` and multiple optional arguments like `path`, `glob`, `type`, etc.
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
                        "description": "Glob pattern to filter files (e.g. \"*.js\", \"*.{ts,tsx}\") - maps to rg --glob"
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

    /// Build the ripgrep argument list from tool input parameters.
    fn build_rg_args(input: &ToolInput, mode: &str) -> Vec<String> {
        let mut args = Vec::new();

        // Output mode flags.
        match mode {
            "files_with_matches" => args.push("-l".to_string()),
            "count" => args.push("-c".to_string()),
            _ => {
                // Default content mode: show line numbers unless explicitly disabled.
                let show_line_numbers = input
                    .arguments
                    .get("-n")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                if show_line_numbers {
                    args.push("-n".to_string());
                }
            }
        }

        // Case insensitive.
        if input
            .arguments
            .get("-i")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            args.push("-i".to_string());
        }

        // Multiline mode.
        if input
            .arguments
            .get("multiline")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            args.push("-U".to_string());
            args.push("--multiline-dotall".to_string());
        }

        // Context lines (content mode only).
        if mode == "content" {
            if let Some(c) = input
                .arguments
                .get("context")
                .or_else(|| input.arguments.get("-C"))
                .and_then(serde_json::Value::as_u64)
            {
                args.push(format!("-C{c}"));
            } else {
                if let Some(b) = input
                    .arguments
                    .get("-B")
                    .and_then(serde_json::Value::as_u64)
                {
                    args.push(format!("-B{b}"));
                }
                if let Some(a) = input
                    .arguments
                    .get("-A")
                    .and_then(serde_json::Value::as_u64)
                {
                    args.push(format!("-A{a}"));
                }
            }
        }

        // Glob filter.
        if let Some(glob) = input.arguments.get("Glob").and_then(|v| v.as_str()) {
            args.push("--glob".to_string());
            args.push(glob.to_string());
        }

        // Type filter.
        if let Some(file_type) = input.arguments.get("type").and_then(|v| v.as_str()) {
            args.push("--type".to_string());
            args.push(file_type.to_string());
        }

        // No .gitignore -- search everything.
        args.push("--no-ignore".to_string());
        args.push("--hidden".to_string());

        args
    }

    /// Apply offset and `head_limit` to output lines.
    fn apply_pagination(lines: Vec<&str>, offset: u64, limit: u64) -> (Vec<String>, bool) {
        let offset = offset as usize;
        let total = lines.len();

        let after_offset: Vec<&str> = lines.into_iter().skip(offset).collect();

        if limit == 0 {
            // Unlimited.
            let result: Vec<String> = after_offset
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            (result, false)
        } else {
            let limit = limit as usize;
            let truncated = after_offset.len() > limit;
            let result: Vec<String> = after_offset
                .into_iter()
                .take(limit)
                .map(std::string::ToString::to_string)
                .collect();
            (result, truncated || total > offset + limit)
        }
    }

    /// Truncate content string to fit within the character budget.
    fn truncate_content(content: &str) -> (String, bool) {
        if content.len() <= MAX_RESULT_SIZE_CHARS {
            (content.to_string(), false)
        } else {
            // Find a safe char boundary.
            let mut end = MAX_RESULT_SIZE_CHARS;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = &content[..end];
            (
                format!(
                    "{truncated}\n\n[output truncated: {} chars total, showing first {end}]",
                    content.len()
                ),
                true,
            )
        }
    }

    /// Execute ripgrep via direct subprocess (fallback when no `CommandRunner`).
    async fn execute_rg_direct(args: &[String], search_path: &str) -> Result<String, ToolError> {
        let mut cmd = tokio::process::Command::new("rg");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.arg(search_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        let result = tokio::time::timeout(timeout, cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                // ripgrep exit codes:
                //   0 = matches found
                //   1 = no matches found
                //   2 = partial errors (e.g. permission denied) but stdout still valid
                let code = output.status.code().unwrap_or(-1);
                if code <= 2 {
                    Ok(String::from_utf8_lossy(&output.stdout).to_string())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(ToolError::RuntimeError {
                        name: "Grep".into(),
                        message: format!("rg exited with code {code}: {stderr}"),
                    })
                }
            }
            Ok(Err(e)) => Err(ToolError::RuntimeError {
                name: "Grep".into(),
                message: format!("failed to execute rg: {e}"),
            }),
            Err(_) => Err(ToolError::Timeout {
                timeout_secs: DEFAULT_TIMEOUT_SECS,
            }),
        }
    }

    /// Build the full shell command string for `CommandRunner` execution.
    fn build_rg_command(pattern: &str, args: &[String], search_path: &str) -> String {
        let args_str = args
            .iter()
            .map(|a| shell_escape(a))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "rg {args_str} -- {} {}",
            shell_escape(pattern),
            shell_escape(search_path)
        )
    }
}

/// Simple shell escaping: wrap in single quotes if needed.
fn shell_escape(s: &str) -> String {
    if s.contains(' ')
        || s.contains('\'')
        || s.contains('"')
        || s.contains('*')
        || s.contains('?')
        || s.contains('[')
        || s.contains('{')
        || s.contains('\\')
        || s.contains('$')
        || s.contains('(')
        || s.contains(')')
        || s.contains('|')
    {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
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

        let search_path = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let mode = input
            .arguments
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let head_limit = input
            .arguments
            .get("head_limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(DEFAULT_HEAD_LIMIT);

        let offset = input
            .arguments
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        // Build rg argument list.
        let mut rg_args = Self::build_rg_args(&input, mode);

        // The pattern itself goes after `--` to avoid interpretation as flags.
        rg_args.push("--".to_string());
        rg_args.push(pattern.to_string());

        tracing::debug!(
            "Grep tool: pattern={pattern:?}, mode={mode}, search_path={search_path:?}, \
             head_limit={head_limit}, offset={offset}"
        );

        // Execute ripgrep -- prefer CommandRunner if available.
        let raw_output = if let Some(ref runner) = input.command_runner {
            // When using CommandRunner, build a single shell command string.
            // Remove the trailing `-- pattern` from rg_args (we embed them in the command).
            let base_args = &rg_args[..rg_args.len() - 2]; // remove `--` and pattern
            let cmd = Self::build_rg_command(pattern, base_args, search_path);
            let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
            let result = runner.run_command(&cmd, None, timeout).await.map_err(|e| {
                ToolError::RuntimeError {
                    name: "Grep".into(),
                    message: format!("{e}"),
                }
            })?;
            String::from_utf8_lossy(&result.stdout).to_string()
        } else {
            Self::execute_rg_direct(&rg_args, search_path).await?
        };

        // Format the result based on output mode.
        match mode {
            "files_with_matches" => {
                let lines: Vec<&str> = raw_output.lines().filter(|l| !l.is_empty()).collect();
                let num_files = lines.len();
                let (paginated, _truncated) = Self::apply_pagination(lines, offset, head_limit);
                let filenames = paginated;

                Ok(ToolOutput {
                    success: true,
                    content: serde_json::json!({
                        "mode": "files_with_matches",
                        "numFiles": num_files,
                        "filenames": filenames,
                        "appliedLimit": head_limit,
                        "appliedOffset": offset,
                    }),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
            "count" => {
                // rg -c output: file:count per line.
                let lines: Vec<&str> = raw_output.lines().filter(|l| !l.is_empty()).collect();
                let total_matches: u64 = lines
                    .iter()
                    .filter_map(|line| {
                        // Format: "path:count"
                        line.rsplit_once(':')
                            .and_then(|(_, c)| c.parse::<u64>().ok())
                    })
                    .sum();
                let num_files = lines.len();

                let (paginated, _truncated) = Self::apply_pagination(lines, offset, head_limit);
                let content = paginated.join("\n");
                let (content, _) = Self::truncate_content(&content);

                Ok(ToolOutput {
                    success: true,
                    content: serde_json::json!({
                        "mode": "count",
                        "numFiles": num_files,
                        "numMatches": total_matches,
                        "content": content,
                        "appliedLimit": head_limit,
                        "appliedOffset": offset,
                    }),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
            _ => {
                // "content" mode -- raw matched lines.
                let lines: Vec<&str> = raw_output.lines().collect();
                let total_lines = lines.len();
                let (paginated, _truncated) = Self::apply_pagination(lines, offset, head_limit);
                let content = paginated.join("\n");
                let (content, _) = Self::truncate_content(&content);

                // Count unique files from output (lines that match "path:line:content").
                let num_files = paginated
                    .iter()
                    .filter_map(|line| line.split_once(':').map(|(f, _)| f))
                    .collect::<std::collections::HashSet<_>>()
                    .len();

                Ok(ToolOutput {
                    success: true,
                    content: serde_json::json!({
                        "mode": "content",
                        "numFiles": num_files,
                        "numLines": total_lines,
                        "content": content,
                        "appliedLimit": head_limit,
                        "appliedOffset": offset,
                    }),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
        }
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
        // Default mode is files_with_matches.
        assert_eq!(output.content["mode"], "files_with_matches");
    }

    #[tokio::test]
    async fn test_grep_content_mode() {
        let tool = GrepTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "fn main",
            "output_mode": "content"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["mode"], "content");
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

    #[test]
    fn test_build_rg_args_files_mode() {
        let input = make_input(serde_json::json!({
            "pattern": "test",
            "Glob": "*.rs",
            "-i": true
        }));
        let args = GrepTool::build_rg_args(&input, "files_with_matches");
        assert!(args.contains(&"-l".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"--glob".to_string()));
        assert!(args.contains(&"*.rs".to_string()));
    }

    #[test]
    fn test_build_rg_args_content_with_context() {
        let input = make_input(serde_json::json!({
            "pattern": "test",
            "-B": 3,
            "-A": 5
        }));
        let args = GrepTool::build_rg_args(&input, "content");
        assert!(args.contains(&"-n".to_string()));
        assert!(args.contains(&"-B3".to_string()));
        assert!(args.contains(&"-A5".to_string()));
    }

    #[test]
    fn test_apply_pagination() {
        let lines = vec!["a", "b", "c", "d", "e"];
        let (result, truncated) = GrepTool::apply_pagination(lines, 1, 2);
        assert_eq!(result, vec!["b", "c"]);
        assert!(truncated);
    }

    #[test]
    fn test_apply_pagination_no_limit() {
        let lines = vec!["a", "b", "c"];
        let (result, truncated) = GrepTool::apply_pagination(lines, 0, 0);
        assert_eq!(result, vec!["a", "b", "c"]);
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_content_short() {
        let s = "short";
        let (result, truncated) = GrepTool::truncate_content(s);
        assert_eq!(result, "short");
        assert!(!truncated);
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("simple"), "simple");
        assert_eq!(shell_escape("has spaces"), "'has spaces'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
