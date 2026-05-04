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

/// Maximum result size in characters returned to the LLM.
const MAX_RESULT_SIZE_CHARS: usize = 10_000;

/// Default timeout for the search (seconds).
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Sets the inner `AtomicBool` to `true` when dropped, signalling
/// the blocking walker thread to stop early.
struct DropGuard(Option<Arc<AtomicBool>>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        if let Some(flag) = self.0.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

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
            working_dir: None,
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
}
