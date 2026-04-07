//! `Glob` built-in tool: fast file pattern matching.
//!
//! Uses ripgrep (`rg --files --glob`) under the hood for performant
//! file discovery across any codebase size. Supports standard glob
//! patterns (e.g. `**/*.rs`, `src/**/*.ts`) and returns matching file
//! paths sorted by modification time.
//!
//! Reference: cursor Glob.js tool implementation.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 100_000;

/// Default timeout for the ripgrep subprocess (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Built-in Glob tool for file pattern matching.
///
/// Accepts a glob `pattern` and an optional `path` (defaults to cwd).
/// Delegates to ripgrep for fast, ignore-aware file discovery.
pub struct GlobTool {
    def: ToolDefinition,
}

impl GlobTool {
    /// Create a new `GlobTool`.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `Glob`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("Glob"),
            description: "Fast file pattern matching tool that works with any codebase size. \
                Supports Glob patterns like \"**/*.rs\" or \"src/**/*.ts\". Returns matching \
                file paths sorted by modification time."
                .into(),
            help: Some(
                "Use this tool to find files by name or path pattern.\n\n\
                 - Supports standard Glob syntax: `*`, `**`, `?`, `[...]`\n\
                 - Results are sorted by modification time (most recent first)\n\
                 - When doing open-ended searches that may require multiple rounds \
                   of globbing and grepping, consider using the Agent tool instead\n\n\
                 Examples:\n\
                 - `**/*.rs` -- all Rust files\n\
                 - `src/**/*.ts` -- TypeScript files under src/\n\
                 - `**/test_*.py` -- Python test files anywhere\n\
                 - `config/*.toml` -- TOML files in the config/ directory"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "The Glob pattern to match files against"
                    },
                    "path": {
                        "type": "string",
                        "description": "The directory to search in. Defaults to the current \
                            working directory if omitted. Must be a valid directory path \
                            if provided."
                    }
                },
                "required": ["pattern"]
            }),
            result_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "matches": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of matched file paths (absolute)"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of matched files"
                    },
                    "search_path": {
                        "type": "string",
                        "description": "The directory that was searched"
                    },
                    "truncated": {
                        "type": "boolean",
                        "description": "Whether the result was truncated due to size limits"
                    }
                }
            })),
            category: ToolCategory::Search,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    /// Resolve an absolute glob pattern into a base directory and relative pattern.
    ///
    /// When the user provides an absolute path like `/home/user/src/**/*.rs`,
    /// we split it into base dir `/home/user/src` and pattern `**/*.rs`.
    fn parse_absolute_glob(pattern: &str) -> Option<(PathBuf, String)> {
        let path = Path::new(pattern);
        if !path.is_absolute() {
            return None;
        }

        // Walk components until we hit a glob metacharacter.
        let mut base = PathBuf::new();
        let mut remaining_parts = Vec::new();
        let mut found_glob = false;

        for component in path.components() {
            let s = component.as_os_str().to_string_lossy();
            if !found_glob && !contains_glob_meta(&s) {
                base.push(component);
            } else {
                found_glob = true;
                remaining_parts.push(s.to_string());
            }
        }

        if found_glob && !remaining_parts.is_empty() {
            let relative_pattern = remaining_parts.join("/");
            Some((base, relative_pattern))
        } else {
            None
        }
    }

    /// Build the ripgrep argument list for file globbing.
    fn build_rg_args(pattern: &str, no_ignore: bool, hidden: bool) -> Vec<String> {
        let mut args = vec![
            "--files".to_string(),
            "--glob".to_string(),
            pattern.to_string(),
        ];

        if no_ignore {
            args.push("--no-ignore".to_string());
        }
        if hidden {
            args.push("--hidden".to_string());
        }

        args
    }

    /// Build the full shell command string for ripgrep execution.
    fn build_rg_command(rg_args: &[String], search_path: &str) -> String {
        let args_str = rg_args
            .iter()
            .map(|a| shell_escape(a))
            .collect::<Vec<_>>()
            .join(" ");
        format!("rg {args_str} {}", shell_escape(search_path))
    }

    /// Execute ripgrep via direct subprocess (fallback when no `CommandRunner`).
    async fn execute_rg_direct(rg_args: &[String], search_path: &str) -> Result<String, ToolError> {
        let mut cmd = tokio::process::Command::new("rg");
        for arg in rg_args {
            cmd.arg(arg);
        }
        cmd.arg(search_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let timeout = std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS);
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
                        name: "Glob".into(),
                        message: format!("rg exited with code {code}: {stderr}"),
                    })
                }
            }
            Ok(Err(e)) => Err(ToolError::RuntimeError {
                name: "Glob".into(),
                message: format!("failed to execute rg: {e}"),
            }),
            Err(_) => Err(ToolError::Timeout {
                timeout_secs: DEFAULT_TIMEOUT_SECS,
            }),
        }
    }

    /// Parse ripgrep stdout into a list of absolute file paths, sorted by mtime
    /// (most recent first).
    fn parse_and_sort_matches(raw_output: &str, search_path: &str) -> Vec<String> {
        let base = Path::new(search_path);
        let mut entries: Vec<(String, std::time::SystemTime)> = raw_output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|line| {
                let p = if Path::new(line).is_absolute() {
                    PathBuf::from(line)
                } else {
                    base.join(line)
                };
                let mtime = p
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                (p.to_string_lossy().to_string(), mtime)
            })
            .collect();

        // Sort newest first.
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.into_iter().map(|(path, _)| path).collect()
    }

    /// Truncate the result to fit within the character budget.
    fn truncate_matches(matches: Vec<String>) -> (Vec<String>, bool) {
        let mut total_chars = 0usize;
        let mut truncated = false;
        let mut result = Vec::new();

        for m in matches {
            // +1 for the newline separator in the serialised form.
            let entry_len = m.len() + 1;
            if total_chars + entry_len > MAX_RESULT_SIZE_CHARS {
                truncated = true;
                break;
            }
            total_chars += entry_len;
            result.push(m);
        }

        (result, truncated)
    }
}

/// Simple shell escaping: wrap in single quotes, escape inner quotes.
fn shell_escape(s: &str) -> String {
    if s.contains(' ')
        || s.contains('\'')
        || s.contains('"')
        || s.contains('*')
        || s.contains('?')
        || s.contains('[')
        || s.contains('{')
    {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

/// Check if a path component contains glob metacharacters.
fn contains_glob_meta(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[') || s.contains('{')
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GlobTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let pattern = input
            .arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'pattern' is required".into(),
            })?;

        let search_path = input.arguments.get("path").and_then(|v| v.as_str());

        // Resolve the target directory and effective glob pattern.
        let (resolved_path, glob_pattern) = if Path::new(pattern).is_absolute() {
            // Absolute pattern: split into base dir + relative glob.
            if let Some((base, rel)) = Self::parse_absolute_glob(pattern) {
                (base.to_string_lossy().to_string(), rel)
            } else {
                // The pattern is a full absolute path with no globs.
                let p = search_path.unwrap_or(".").to_string();
                (p, pattern.to_string())
            }
        } else {
            let p = search_path.unwrap_or(".").to_string();
            (p, pattern.to_string())
        };

        // Build rg arguments (behavior defaults: no_ignore=true, hidden=true).
        let rg_args = Self::build_rg_args(&glob_pattern, true, true);

        tracing::debug!(
            "Glob tool: pattern={glob_pattern:?}, search_path={resolved_path:?}, rg_args={rg_args:?}"
        );

        // Execute ripgrep -- prefer CommandRunner if available, fall back to direct.
        let raw_output = if let Some(ref runner) = input.command_runner {
            let cmd = Self::build_rg_command(&rg_args, &resolved_path);
            let timeout = std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS);
            let result = runner.run_command(&cmd, None, timeout).await.map_err(|e| {
                ToolError::RuntimeError {
                    name: "Glob".into(),
                    message: format!("{e}"),
                }
            })?;
            String::from_utf8_lossy(&result.stdout).to_string()
        } else {
            Self::execute_rg_direct(&rg_args, &resolved_path).await?
        };

        // Parse, sort by mtime, and truncate.
        let all_matches = Self::parse_and_sort_matches(&raw_output, &resolved_path);
        let total_count = all_matches.len();
        let (matches, truncated) = Self::truncate_matches(all_matches);

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "matches": matches,
                "count": total_count,
                "search_path": resolved_path,
                "truncated": truncated,
            }),
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

#[cfg(test)]
mod tests {
    use y_core::types::SessionId;

    use super::*;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_glob_001".into(),
            name: ToolName::from_string("Glob"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_glob_basic_pattern() {
        // This test runs against the real filesystem -- we use a pattern
        // that matches at least one file in the project root.
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({"pattern": "**/*.rs"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        // Should find at least one .rs file.
        let count = output.content["count"].as_u64().unwrap();
        assert!(count > 0, "expected matches for **/*.rs, got 0");
        assert!(!output.content["matches"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_glob_with_search_path() {
        // Use a controlled temp directory to avoid permission issues.
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_string_lossy().to_string();
        std::fs::write(tmp.path().join("hello.txt"), "hello").unwrap();

        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "*",
            "path": tmp_path
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["search_path"], tmp_path);
        let count = output.content["count"].as_u64().unwrap();
        assert!(count >= 1, "expected at least 1 match in tempdir");
    }

    #[tokio::test]
    async fn test_glob_absolute_pattern_split() {
        // Verify the path-splitting logic still works.
        let (base, rel) = GlobTool::parse_absolute_glob("/home/user/src/**/*.ts").unwrap();
        assert_eq!(base, PathBuf::from("/home/user/src"));
        assert_eq!(rel, "**/*.ts");
    }

    #[tokio::test]
    async fn test_glob_missing_pattern_fails() {
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_glob_rg_args_include_flags() {
        let rg_args = GlobTool::build_rg_args("**/*.rs", true, true);
        assert!(rg_args.contains(&"--files".to_string()));
        assert!(rg_args.contains(&"--glob".to_string()));
        assert!(rg_args.contains(&"--no-ignore".to_string()));
        assert!(rg_args.contains(&"--hidden".to_string()));
    }

    #[test]
    fn test_glob_definition() {
        let def = GlobTool::tool_definition();
        assert_eq!(def.name.as_str(), "Glob");
        assert_eq!(def.category, ToolCategory::Search);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("pattern"));
        assert!(props.contains_key("path"));
        let required = def.parameters["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("pattern")));
    }

    #[test]
    fn test_parse_absolute_glob() {
        let result = GlobTool::parse_absolute_glob("/home/user/src/**/*.rs");
        assert!(result.is_some());
        let (base, pattern) = result.unwrap();
        assert_eq!(base, PathBuf::from("/home/user/src"));
        assert_eq!(pattern, "**/*.rs");
    }

    #[test]
    fn test_parse_absolute_glob_no_meta() {
        let result = GlobTool::parse_absolute_glob("/home/user/src/main.rs");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_relative_glob_returns_none() {
        let result = GlobTool::parse_absolute_glob("src/**/*.rs");
        assert!(result.is_none());
    }

    #[test]
    fn test_contains_glob_meta() {
        assert!(contains_glob_meta("*.rs"));
        assert!(contains_glob_meta("test?"));
        assert!(contains_glob_meta("[a-z]"));
        assert!(contains_glob_meta("{foo,bar}"));
        assert!(!contains_glob_meta("main.rs"));
    }

    #[test]
    fn test_truncate_matches() {
        let matches: Vec<String> = (0..10).map(|i| format!("/path/to/file_{i}.rs")).collect();
        let (result, truncated) = GlobTool::truncate_matches(matches);
        assert!(!truncated);
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("simple"), "simple");
        assert_eq!(shell_escape("**/*.rs"), "'**/*.rs'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
