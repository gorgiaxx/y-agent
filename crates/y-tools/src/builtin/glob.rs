//! `Glob` built-in tool: fast file pattern matching.
//!
//! Uses the `ignore` and `globset` crates (from ripgrep) for performant,
//! in-process file discovery. Supports standard glob patterns
//! (e.g. `**/*.rs`, `src/**/*.ts`) and returns matching file paths
//! sorted by modification time.
//!
//! Reference: cursor Glob.js tool implementation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use super::path_utils::{resolve_read_path, DropGuard};

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 10_000;

/// Default number of file paths returned in one response.
const DEFAULT_MAX_RESULTS: usize = 50;

/// Hard ceiling for caller-requested result counts.
const MAX_RESULT_LIMIT: usize = 1_000;

/// Default timeout for the search (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Built-in Glob tool for file pattern matching.
///
/// Accepts a glob `pattern` and an optional `path` (defaults to cwd).
/// Uses the `ignore` crate for fast, in-process file discovery.
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
                        "description": "The directory to search in. Defaults to the session \
                            workspace if available, otherwise the current working directory. \
                            Relative paths are resolved against the session workspace."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_RESULT_LIMIT,
                        "default": DEFAULT_MAX_RESULTS,
                        "description": "Maximum number of matched file paths to return. Defaults \
                            to 50 to keep tool results compact; count still reports the total \
                            matches found."
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
                    "returned_count": {
                        "type": "integer",
                        "description": "Number of file paths returned after result limits"
                    },
                    "result_limit": {
                        "type": "integer",
                        "description": "Maximum number of file paths requested for this call"
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

    /// Execute the glob search using the `ignore` crate's `WalkBuilder`.
    fn execute_walk(
        glob_pattern: &str,
        search_path: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<String>, ToolError> {
        let search_dir = Path::new(search_path);

        // Build an override matcher for the glob pattern.
        let mut override_builder = OverrideBuilder::new(search_dir);
        override_builder
            .add(glob_pattern)
            .map_err(|e| ToolError::RuntimeError {
                name: "Glob".into(),
                message: format!("invalid glob pattern '{glob_pattern}': {e}"),
            })?;
        let overrides = override_builder
            .build()
            .map_err(|e| ToolError::RuntimeError {
                name: "Glob".into(),
                message: format!("failed to build glob matcher: {e}"),
            })?;

        // Configure walker: show hidden files, ignore no ignore files.
        let walker = WalkBuilder::new(search_dir)
            .hidden(false) // show hidden files (equivalent to --hidden)
            .standard_filters(false) // disable all ignore filters (equivalent to --no-ignore)
            .overrides(overrides)
            .build();

        let mut entries: Vec<(String, std::time::SystemTime)> = Vec::new();
        for result in walker {
            if cancelled.load(Ordering::Relaxed) {
                return Err(ToolError::Cancelled);
            }
            match result {
                Ok(entry) => {
                    // Skip directories -- only collect files.
                    if entry.file_type().is_none_or(|ft| ft.is_dir()) {
                        continue;
                    }
                    let path = entry.into_path();
                    let abs_path = if path.is_absolute() {
                        path
                    } else {
                        search_dir.join(&path)
                    };
                    let mtime = abs_path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    entries.push((abs_path.to_string_lossy().to_string(), mtime));
                }
                Err(e) => {
                    tracing::debug!("Glob walk error (skipping): {e}");
                }
            }
        }

        // Sort newest first.
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(entries.into_iter().map(|(path, _)| path).collect())
    }

    /// Truncate the result to fit within the character budget.
    fn truncate_matches(matches: Vec<String>, max_matches: usize) -> (Vec<String>, bool) {
        let mut total_chars = 0usize;
        let mut truncated = false;
        let mut result = Vec::new();

        for m in matches {
            if result.len() >= max_matches {
                truncated = true;
                break;
            }
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

    fn parse_max_results(arguments: &serde_json::Value) -> Result<usize, ToolError> {
        let Some(value) = arguments.get("max_results") else {
            return Ok(DEFAULT_MAX_RESULTS);
        };
        let Some(value) = value.as_u64() else {
            return Err(ToolError::ValidationError {
                message: "'max_results' must be a positive integer".into(),
            });
        };
        let max_results = usize::try_from(value).map_err(|_| ToolError::ValidationError {
            message: format!("'max_results' must be at most {MAX_RESULT_LIMIT}"),
        })?;
        if max_results == 0 || max_results > MAX_RESULT_LIMIT {
            return Err(ToolError::ValidationError {
                message: format!("'max_results' must be between 1 and {MAX_RESULT_LIMIT}"),
            });
        }
        Ok(max_results)
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
        let working_dir = input.working_dir.as_deref();
        let additional_read_dirs = &input.additional_read_dirs;
        let max_results = Self::parse_max_results(&input.arguments)?;

        // Resolve the target directory and effective glob pattern.
        let (resolved_path, glob_pattern) = if Path::new(pattern).is_absolute() {
            // Absolute pattern: split into base dir + relative glob.
            if let Some((base, rel)) = Self::parse_absolute_glob(pattern) {
                let base = base.to_string_lossy().to_string();
                (
                    resolve_read_path("Glob", Some(&base), working_dir, additional_read_dirs)?,
                    rel,
                )
            } else {
                // The pattern is a full absolute path with no globs.
                (
                    resolve_read_path("Glob", search_path, working_dir, additional_read_dirs)?,
                    pattern.to_string(),
                )
            }
        } else {
            (
                resolve_read_path("Glob", search_path, working_dir, additional_read_dirs)?,
                pattern.to_string(),
            )
        };
        let resolved_path = resolved_path.to_string_lossy().to_string();

        tracing::debug!("Glob tool: pattern={glob_pattern:?}, search_path={resolved_path:?}");

        // Execute the walk in a blocking task with timeout.
        // The AtomicBool is set when this future is dropped (e.g. by
        // tokio::select! on a CancellationToken) so the blocking thread
        // stops iterating promptly instead of running to completion.
        let rp = resolved_path.clone();
        let gp = glob_pattern.clone();
        let timeout = std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_clone = Arc::clone(&cancelled);
        let guard = DropGuard(Some(cancelled));
        let all_matches = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || Self::execute_walk(&gp, &rp, &cancelled_clone)),
        )
        .await
        .map_err(|_| ToolError::Timeout {
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        })?
        .map_err(|e| ToolError::RuntimeError {
            name: "Glob".into(),
            message: format!("search task failed: {e}"),
        })??;
        drop(guard);

        // Truncate.
        let total_count = all_matches.len();
        let (matches, truncated) = Self::truncate_matches(all_matches, max_results);
        let returned_count = matches.len();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "matches": matches,
                "count": total_count,
                "returned_count": returned_count,
                "result_limit": max_results,
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
    async fn test_glob_defaults_to_injected_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace
            .path()
            .join("website")
            .join("src")
            .join("__glob_working_dir_unique__.tsx");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "export const unique = true;").unwrap();

        let tool = GlobTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({"pattern": "website/src/__glob_working_dir_unique__.tsx"}),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(
            output.content["search_path"],
            workspace.path().display().to_string()
        );
        assert_eq!(output.content["count"], 1);
        assert_eq!(
            output.content["matches"][0],
            file_path.display().to_string()
        );
    }

    #[tokio::test]
    async fn test_glob_resolves_relative_search_path_against_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let file_path = workspace
            .path()
            .join("website")
            .join("src")
            .join("__glob_relative_path_unique__.tsx");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, "export const relativePath = true;").unwrap();

        let tool = GlobTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "pattern": "src/__glob_relative_path_unique__.tsx",
                "path": "website"
            }),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(
            output.content["search_path"],
            workspace.path().join("website").display().to_string()
        );
        assert_eq!(output.content["count"], 1);
        assert_eq!(
            output.content["matches"][0],
            file_path.display().to_string()
        );
    }

    #[tokio::test]
    async fn test_glob_rejects_search_outside_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("__glob_outside_unique__.tsx");
        std::fs::write(&outside_file, "export const outside = true;").unwrap();

        let tool = GlobTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "pattern": outside.path().join("*.tsx").display().to_string()
            }),
            workspace.path(),
        );
        let result = tool.execute(input).await;

        assert!(matches!(
            result,
            Err(ToolError::PermissionDenied { name, .. }) if name == "Glob"
        ));
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
        let (result, truncated) = GlobTool::truncate_matches(matches, 50);
        assert!(!truncated);
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_truncate_matches_limits_match_count() {
        let matches: Vec<String> = (0..60).map(|i| format!("/path/to/file_{i}.rs")).collect();
        let (result, truncated) = GlobTool::truncate_matches(matches, 50);
        assert!(truncated);
        assert_eq!(result.len(), 50);
    }

    #[tokio::test]
    async fn test_glob_default_result_limit() {
        let workspace = tempfile::tempdir().unwrap();
        for i in 0..60 {
            std::fs::write(workspace.path().join(format!("package-{i}.json")), "{}").unwrap();
        }

        let tool = GlobTool::new();
        let input =
            make_input_with_working_dir(serde_json::json!({"pattern": "*.json"}), workspace.path());
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["count"], 60);
        assert_eq!(output.content["returned_count"], 50);
        assert_eq!(output.content["result_limit"], 50);
        assert!(output.content["truncated"].as_bool().unwrap());
        assert_eq!(output.content["matches"].as_array().unwrap().len(), 50);
    }

    #[tokio::test]
    async fn test_glob_uses_explicit_result_limit() {
        let workspace = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(workspace.path().join(format!("match-{i}.rs")), "").unwrap();
        }

        let tool = GlobTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "pattern": "*.rs",
                "max_results": 3
            }),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["count"], 10);
        assert_eq!(output.content["returned_count"], 3);
        assert_eq!(output.content["result_limit"], 3);
        assert!(output.content["truncated"].as_bool().unwrap());
        assert_eq!(output.content["matches"].as_array().unwrap().len(), 3);
    }
}
