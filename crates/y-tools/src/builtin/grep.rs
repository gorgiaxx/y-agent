//! `Grep` built-in tool: powerful search tool built on ripgrep libraries.
//!
//! Uses `grep-searcher`, `grep-regex`, and `ignore` crates for performant,
//! in-process file content matching. Supports full regex syntax, file type
//! filtering, glob filtering, and multiline matching.
//!
//! Reference: cursor Grep.js tool implementation.

use std::collections::HashSet;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{Searcher, SearcherBuilder};
use ignore::overrides::OverrideBuilder;
use ignore::types::TypesBuilder;
use ignore::WalkBuilder;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use super::path_utils::resolve_read_path;

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 10_000;

/// Default `head_limit` when unspecified.
const DEFAULT_HEAD_LIMIT: u64 = 250;

/// Default timeout for the search (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Sets the inner `AtomicBool` to `true` when dropped, signalling
/// the blocking search thread to stop early.
struct DropGuard(Option<Arc<AtomicBool>>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        if let Some(flag) = self.0.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

/// Built-in Grep tool for file content searching.
///
/// Accepts a regex `pattern` and multiple optional arguments like `path`, `glob`, `type`, etc.
/// Uses ripgrep library crates for fast, in-process file content searching.
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
                        "description": "File or directory to search in (rg PATH). Defaults to the session workspace if available, otherwise the current working directory. Relative paths are resolved against the session workspace."
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
                        "description": "Limit output to first N lines/entries, equivalent to \"| head -N\". Works across all output modes: content (limits output lines), files_with_matches (limits file paths), count (limits count entries). Defaults to 250 when unspecified. Pass 0 for unlimited (use sparingly -- large result sets waste context)."
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

    /// Execute the grep search using ripgrep library crates.
    ///
    /// Returns raw output lines appropriate for the given mode.
    fn execute_search(
        params: &SearchParams,
        cancelled: &AtomicBool,
    ) -> Result<Vec<String>, ToolError> {
        let search_path = Path::new(&params.search_path);

        // Build regex matcher.
        let matcher = {
            let mut builder = RegexMatcherBuilder::new();
            if params.case_insensitive {
                builder.case_insensitive(true);
            }
            if params.multiline {
                builder.multi_line(true).dot_matches_new_line(true);
            }
            builder
                .build(&params.pattern)
                .map_err(|e| ToolError::RuntimeError {
                    name: "Grep".into(),
                    message: format!("invalid regex pattern '{}': {e}", params.pattern),
                })?
        };

        // Build directory walker.
        let mut walk_builder = WalkBuilder::new(search_path);
        walk_builder
            .hidden(false) // show hidden files (equivalent to --hidden)
            .standard_filters(false); // disable all ignore filters (equivalent to --no-ignore)

        // Apply glob filter if specified.
        if let Some(ref glob) = params.glob_filter {
            let mut override_builder = OverrideBuilder::new(search_path);
            override_builder
                .add(glob)
                .map_err(|e| ToolError::RuntimeError {
                    name: "Grep".into(),
                    message: format!("invalid glob filter '{glob}': {e}"),
                })?;
            let overrides = override_builder
                .build()
                .map_err(|e| ToolError::RuntimeError {
                    name: "Grep".into(),
                    message: format!("failed to build glob filter: {e}"),
                })?;
            walk_builder.overrides(overrides);
        }

        // Apply file type filter if specified.
        if let Some(ref file_type) = params.type_filter {
            let mut types_builder = TypesBuilder::new();
            types_builder.add_defaults();
            types_builder.select(file_type);
            let types = types_builder.build().map_err(|e| ToolError::RuntimeError {
                name: "Grep".into(),
                message: format!("invalid file type '{file_type}': {e}"),
            })?;
            walk_builder.types(types);
        }

        // Build searcher with context settings.
        let mut searcher_builder = SearcherBuilder::new();
        if params.multiline {
            searcher_builder.multi_line(true);
        }
        if params.mode == "content" {
            searcher_builder.line_number(params.show_line_numbers);
            if let Some(c) = params.context {
                searcher_builder.before_context(c as usize);
                searcher_builder.after_context(c as usize);
            } else {
                if let Some(b) = params.before_context {
                    searcher_builder.before_context(b as usize);
                }
                if let Some(a) = params.after_context {
                    searcher_builder.after_context(a as usize);
                }
            }
        }

        let results: Mutex<Vec<String>> = Mutex::new(Vec::new());

        // Search a single file, returns whether it had matches.
        let search_file = |path: &Path, searcher: &mut Searcher| -> bool {
            let mut file_had_match = false;
            match &params.mode[..] {
                "files_with_matches" => {
                    let sink = UTF8(|_line_num, _line| {
                        file_had_match = true;
                        Ok(false) // stop after first match
                    });
                    let _ = searcher.search_path(&matcher, path, sink);
                    if file_had_match {
                        if let Ok(mut r) = results.lock() {
                            r.push(path.to_string_lossy().to_string());
                        }
                    }
                }
                "count" => {
                    let mut count: u64 = 0;
                    let sink = UTF8(|_line_num, line| {
                        let mut line_count: u64 = 0;
                        let _ = matcher.find_iter(line.as_bytes(), |_m| {
                            line_count += 1;
                            true
                        });
                        count += line_count;
                        file_had_match = true;
                        Ok(true)
                    });
                    let _ = searcher.search_path(&matcher, path, sink);
                    if count > 0 {
                        if let Ok(mut r) = results.lock() {
                            r.push(format!("{}:{count}", path.to_string_lossy()));
                        }
                    }
                }
                _ => {
                    // "content" mode
                    let path_str = path.to_string_lossy().to_string();
                    let sink = UTF8(|line_num, line| {
                        file_had_match = true;
                        let formatted = if params.show_line_numbers {
                            format!("{path_str}:{line_num}:{}", line.trim_end_matches('\n'))
                        } else {
                            format!("{path_str}:{}", line.trim_end_matches('\n'))
                        };
                        if let Ok(mut r) = results.lock() {
                            r.push(formatted);
                        }
                        Ok(true)
                    });
                    let _ = searcher.search_path(&matcher, path, sink);
                }
            }
            file_had_match
        };

        // If the search path is a single file, search it directly.
        if search_path.is_file() {
            let mut searcher = searcher_builder.build();
            search_file(search_path, &mut searcher);
        } else {
            // Walk directory and search each file.
            let mut searcher = searcher_builder.build();
            for entry in walk_builder.build() {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(ToolError::Cancelled);
                }
                match entry {
                    Ok(entry) => {
                        if entry.file_type().is_none_or(|ft| ft.is_dir()) {
                            continue;
                        }
                        search_file(entry.path(), &mut searcher);
                    }
                    Err(e) => {
                        tracing::debug!("Grep walk error (skipping): {e}");
                    }
                }
            }
        }

        let output = results.into_inner().map_err(|e| ToolError::RuntimeError {
            name: "Grep".into(),
            message: format!("failed to collect results: {e}"),
        })?;
        Ok(output)
    }
}

/// Parameters for the grep search, extracted from `ToolInput` for Send + 'static.
struct SearchParams {
    pattern: String,
    search_path: String,
    mode: String,
    case_insensitive: bool,
    multiline: bool,
    glob_filter: Option<String>,
    type_filter: Option<String>,
    show_line_numbers: bool,
    context: Option<u64>,
    before_context: Option<u64>,
    after_context: Option<u64>,
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
            })?
            .to_string();

        let search_path_arg = input
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|value| !value.is_empty());
        let search_path = resolve_read_path(
            "Grep",
            search_path_arg,
            input.working_dir.as_deref(),
            &input.additional_read_dirs,
        )?
        .to_string_lossy()
        .to_string();

        let mode = input
            .arguments
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches")
            .to_string();

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

        let case_insensitive = input
            .arguments
            .get("-i")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let multiline = input
            .arguments
            .get("multiline")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let show_line_numbers = input
            .arguments
            .get("-n")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        let glob_filter = input
            .arguments
            .get("Glob")
            .and_then(|v| v.as_str())
            .map(String::from);

        let type_filter = input
            .arguments
            .get("type")
            .and_then(|v| v.as_str())
            .map(String::from);

        let ctx_lines = input
            .arguments
            .get("context")
            .or_else(|| input.arguments.get("-C"))
            .and_then(serde_json::Value::as_u64);

        let before_context = input
            .arguments
            .get("-B")
            .and_then(serde_json::Value::as_u64);

        let after_context = input
            .arguments
            .get("-A")
            .and_then(serde_json::Value::as_u64);

        let params = SearchParams {
            pattern,
            search_path: search_path.clone(),
            mode: mode.clone(),
            case_insensitive,
            multiline,
            glob_filter,
            type_filter,
            show_line_numbers,
            context: ctx_lines,
            before_context,
            after_context,
        };

        tracing::debug!(
            "Grep tool: pattern={:?}, mode={mode}, search_path={search_path:?}, \
             head_limit={head_limit}, offset={offset}",
            params.pattern
        );

        // Execute search in a blocking task with timeout.
        // The AtomicBool is set when this future is dropped (e.g. by
        // tokio::select! on a CancellationToken) so the blocking thread
        // stops iterating promptly instead of running to completion.
        let timeout = std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = Arc::clone(&cancelled);
        let guard = DropGuard(Some(cancelled));
        let raw_lines = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || Self::execute_search(&params, &cancelled_clone)),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        })?
        .map_err(|e| ToolError::RuntimeError {
            name: "Grep".into(),
            message: format!("search task failed: {e}"),
        })??;
        drop(guard);

        // Format the result based on output mode.
        match mode.as_str() {
            "files_with_matches" => {
                let num_files = raw_lines.len();
                let line_refs: Vec<&str> = raw_lines.iter().map(String::as_str).collect();
                let (paginated, _truncated) = Self::apply_pagination(line_refs, offset, head_limit);
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
                // raw_lines format: "path:count" per line.
                let total_matches: u64 = raw_lines
                    .iter()
                    .filter_map(|line| {
                        line.rsplit_once(':')
                            .and_then(|(_, c)| c.parse::<u64>().ok())
                    })
                    .sum();
                let num_files = raw_lines.len();

                let line_refs: Vec<&str> = raw_lines.iter().map(String::as_str).collect();
                let (paginated, _truncated) = Self::apply_pagination(line_refs, offset, head_limit);
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
                let total_lines = raw_lines.len();
                let line_refs: Vec<&str> = raw_lines.iter().map(String::as_str).collect();
                let (paginated, _truncated) = Self::apply_pagination(line_refs, offset, head_limit);
                let content = paginated.join("\n");
                let (content, _) = Self::truncate_content(&content);

                // Count unique files from output (lines that match "path:line:content").
                let num_files = paginated
                    .iter()
                    .filter_map(|line| line.split_once(':').map(|(f, _)| f))
                    .collect::<HashSet<_>>()
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
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        }
    }

    fn make_input_with_working_dir(args: serde_json::Value, working_dir: &Path) -> ToolInput {
        let mut input = make_input(args);
        input.working_dir = Some(working_dir.display().to_string());
        input
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
    async fn test_grep_defaults_to_injected_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace.path().join("__grep_working_dir_unique__.txt");
        std::fs::write(&file_path, "needle_from_injected_workspace").unwrap();

        let tool = GrepTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({"pattern": "needle_from_injected_workspace"}),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["numFiles"], 1);
        assert_eq!(
            output.content["filenames"][0],
            file_path.display().to_string()
        );
    }

    #[tokio::test]
    async fn test_grep_resolves_relative_path_against_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace
            .path()
            .join("website")
            .join("__grep_relative_path_unique__.txt");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "needle_from_relative_path").unwrap();

        let tool = GrepTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "pattern": "needle_from_relative_path",
                "path": "website"
            }),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["numFiles"], 1);
        assert_eq!(
            output.content["filenames"][0],
            file_path.display().to_string()
        );
    }

    #[tokio::test]
    async fn test_grep_rejects_search_outside_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("__grep_outside_unique__.txt");
        std::fs::write(&outside_file, "needle_outside_workspace").unwrap();

        let tool = GrepTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "pattern": "needle_outside_workspace",
                "path": outside.path().display().to_string()
            }),
            workspace.path(),
        );
        let result = tool.execute(input).await;

        assert!(matches!(
            result,
            Err(ToolError::PermissionDenied { name, .. }) if name == "Grep"
        ));
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
    fn test_build_rg_args_content_with_context() {
        // Verify that context parameters are properly parsed.
        let input = make_input(serde_json::json!({
            "pattern": "test",
            "-B": 3,
            "-A": 5
        }));
        let before = input
            .arguments
            .get("-B")
            .and_then(serde_json::Value::as_u64);
        let after = input
            .arguments
            .get("-A")
            .and_then(serde_json::Value::as_u64);
        assert_eq!(before, Some(3));
        assert_eq!(after, Some(5));
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
}
