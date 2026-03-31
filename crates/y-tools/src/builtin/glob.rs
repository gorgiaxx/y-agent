//! `Glob` built-in tool: fast file pattern matching.
//!
//! Uses ripgrep (`rg --files --Glob`) under the hood for performant
//! file discovery across any codebase size. Supports standard Glob
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

/// Built-in Glob tool for file pattern matching.
///
/// Accepts a Glob `pattern` and an optional `path` (defaults to cwd).
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

    /// Resolve an absolute Glob pattern into a base directory and relative pattern.
    ///
    /// When the user provides an absolute path like `/home/user/src/**/*.rs`,
    /// we split it into base dir `/home/user/src` and pattern `**/*.rs`.
    fn parse_absolute_glob(pattern: &str) -> Option<(PathBuf, String)> {
        let path = Path::new(pattern);
        if !path.is_absolute() {
            return None;
        }

        // Walk components until we hit a Glob metacharacter.
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
            "--Glob".to_string(),
            pattern.to_string(),
            "--sort=modified".to_string(),
        ];

        if no_ignore {
            args.push("--no-ignore".to_string());
        }
        if hidden {
            args.push("--hidden".to_string());
        }

        args
    }
}

/// Check if a path component contains Glob metacharacters.
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

        // Resolve the target directory and effective Glob pattern.
        let (resolved_path, glob_pattern) = if Path::new(pattern).is_absolute() {
            // Absolute pattern: split into base dir + relative Glob.
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

        // -- Behavior placeholder --
        // Actual ripgrep execution will be implemented later.
        // For now, return the planned execution descriptor so the orchestrator
        // and tests can validate the tool's argument preparation logic.
        let _ = MAX_RESULT_SIZE_CHARS;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "glob_search",
                "pattern": glob_pattern,
                "search_path": resolved_path,
                "rg_args": rg_args,
                "matches": [],
                "count": 0,
                "truncated": false,
                "status": "pending"
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
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({"pattern": "**/*.rs"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "glob_search");
        assert_eq!(output.content["pattern"], "**/*.rs");
        assert_eq!(output.content["search_path"], ".");
    }

    #[tokio::test]
    async fn test_glob_with_search_path() {
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "*.toml",
            "path": "/tmp/project"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["pattern"], "*.toml");
        assert_eq!(output.content["search_path"], "/tmp/project");
    }

    #[tokio::test]
    async fn test_glob_absolute_pattern_split() {
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({
            "pattern": "/home/user/src/**/*.ts"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["pattern"], "**/*.ts");
        assert_eq!(output.content["search_path"], "/home/user/src");
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
        let tool = GlobTool::new();
        let input = make_input(serde_json::json!({"pattern": "**/*.rs"}));
        let output = tool.execute(input).await.unwrap();
        let rg_args: Vec<String> =
            serde_json::from_value(output.content["rg_args"].clone()).unwrap();
        assert!(rg_args.contains(&"--files".to_string()));
        assert!(rg_args.contains(&"--Glob".to_string()));
        assert!(rg_args.contains(&"--no-ignore".to_string()));
        assert!(rg_args.contains(&"--hidden".to_string()));
        assert!(rg_args.contains(&"--sort=modified".to_string()));
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
}
