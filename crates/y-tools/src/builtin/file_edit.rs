//! `file_edit` built-in tool: perform exact string replacements in files.
//!
//! This tool replaces an exact substring (`old_string`) with a new string
//! (`new_string`) inside a file. When `old_string` is empty the tool creates
//! the file with `new_string` as content (parent directories are created
//! automatically).
//!
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use tokio::sync::Mutex;
use y_core::file_mutation::{
    content_hash, FileMutationCapability, FileMutationOperation, ABSENT_CONTENT_HASH,
};
use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use super::path_utils::resolve_workspace_path;

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
        let mut capabilities = RuntimeCapability::default();
        capabilities.filesystem.mutation = Some(FileMutationCapability::new(
            FileMutationOperation::CreateOrModify,
            "file_path",
        ));
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
                    },
                    "expected_content_hash": {
                        "type": "string",
                        "description": concat!(
                            "Optional SHA-256 content hash returned by FileRead. ",
                            "The edit fails without writing if the file changed after it was read."
                        )
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities,
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
        let expected_content_hash = input
            .arguments
            .get("expected_content_hash")
            .and_then(serde_json::Value::as_str);

        // Reject no-ops where old and new are identical.
        if old_string == new_string {
            return Err(ToolError::ValidationError {
                message: "no changes to make: old_string and new_string are identical".into(),
            });
        }

        let path =
            resolve_workspace_path("FileEdit", Some(file_path), input.working_dir.as_deref())?;
        let path_lock = lock_for_path(&lock_identity(&path));
        let _path_guard = path_lock.lock().await;

        // --- File creation path (old_string is empty) ---
        if old_string.is_empty() {
            return self
                .create_file(&path, file_path, new_string, expected_content_hash)
                .await;
        }

        // --- Edit path ---
        self.edit_file(
            &path,
            file_path,
            old_string,
            new_string,
            replace_all,
            expected_content_hash,
        )
        .await
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }

    fn is_destructive(&self) -> bool {
        true
    }
}

impl FileEditTool {
    /// Create a new file, failing if the file already has non-empty content.
    async fn create_file(
        &self,
        path: &Path,
        file_path: &str,
        new_string: &str,
        expected_content_hash: Option<&str>,
    ) -> Result<ToolOutput, ToolError> {
        let existing = match tokio::fs::read(path).await {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(ToolError::Other {
                    message: format!("failed to read '{file_path}': {error}"),
                });
            }
        };
        let actual_hash = existing
            .as_deref()
            .map_or(ABSENT_CONTENT_HASH.to_string(), content_hash);
        verify_expected_hash(
            file_path,
            expected_content_hash,
            &actual_hash,
            existing.as_deref().unwrap_or_default(),
        )?;

        if existing
            .as_deref()
            .is_some_and(|content| !String::from_utf8_lossy(content).trim().is_empty())
        {
            return Err(ToolError::ValidationError {
                message: format!(
                    "cannot create new file -- '{file_path}' already exists with content",
                ),
            });
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
        let after_hash = content_hash(new_string.as_bytes());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "file_path": file_path,
                "action": "created",
                "bytes_written": new_string.len(),
                "before_hash": existing.as_deref().map(content_hash),
                "after_hash": after_hash,
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
        expected_content_hash: Option<&str>,
    ) -> Result<ToolOutput, ToolError> {
        // Read current content (normalise line endings to LF).
        let raw_content = tokio::fs::read(path)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => ToolError::FileNotFound {
                    path: file_path.to_string(),
                },
                _ => ToolError::Other {
                    message: format!("failed to read '{file_path}': {error}"),
                },
            })?;
        let before_hash = content_hash(&raw_content);
        verify_expected_hash(file_path, expected_content_hash, &before_hash, &raw_content)?;
        let content = String::from_utf8(raw_content).map_err(|error| ToolError::Other {
            message: format!("failed to decode '{file_path}' as UTF-8: {error}"),
        })?;
        let content = content.replace("\r\n", "\n");

        // Count occurrences of old_string.
        let match_count = content.matches(old_string).count();

        if match_count == 0 {
            return Err(ToolError::EditTargetNotFound {
                path: file_path.to_string(),
            });
        }

        if match_count > 1 && !replace_all {
            return Err(ToolError::AmbiguousEdit {
                path: file_path.to_string(),
                matches: match_count,
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
        let after_hash = content_hash(updated.as_bytes());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "file_path": file_path,
                "action": "edited",
                "replacements": if replace_all { match_count } else { 1 },
                "replace_all": replace_all,
                "before_hash": before_hash,
                "after_hash": after_hash,
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
- Pass FileRead's `content_hash` as `expected_content_hash` when available. \
  A stale hash fails without modifying the file and returns fresh context.
- Prefer editing existing files. Only create new files when explicitly required.
- The edit will FAIL if `old_string` is not unique in the file. Either provide \
  a larger string with more surrounding context to make it unique, or set \
  `replace_all` to true.
- Use `replace_all` for bulk renaming (e.g. renaming a variable across the \
  entire file).
- When `old_string` is empty, the tool creates the file with `new_string` as \
  content (fails if the file already has non-empty content).";

const MAX_STALE_CONTEXT_CHARS: usize = 1_200;

fn verify_expected_hash(
    file_path: &str,
    expected_hash: Option<&str>,
    actual_hash: &str,
    current_content: &[u8],
) -> Result<(), ToolError> {
    if let Some(expected_hash) = expected_hash {
        if expected_hash != actual_hash {
            return Err(ToolError::StaleFile {
                path: file_path.to_string(),
                expected_hash: expected_hash.to_string(),
                actual_hash: actual_hash.to_string(),
                fresh_context: String::from_utf8_lossy(current_content)
                    .chars()
                    .take(MAX_STALE_CONTEXT_CHARS)
                    .collect(),
            });
        }
    }
    Ok(())
}

fn lock_for_path(path: &Path) -> Arc<Mutex<()>> {
    static PATH_LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
        OnceLock::new();
    let registry = PATH_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut locks = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn lock_identity(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(name);
        }
    }
    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("FileEdit"),
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

    fn target_test_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/y-tools-tests/file-edit-outside");
        std::fs::create_dir_all(&dir).unwrap();
        dir
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
    async fn test_expected_content_hash_rejects_stale_edit_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("stale.txt");
        std::fs::write(&file, "version two").unwrap();

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "two",
            "new_string": "three",
            "expected_content_hash": y_core::file_mutation::content_hash(b"version one")
        }));
        let result = tool.execute(input).await;

        assert!(matches!(
            result,
            Err(ToolError::StaleFile {
                expected_hash,
                actual_hash,
                ..
            }) if expected_hash == y_core::file_mutation::content_hash(b"version one")
                && actual_hash == y_core::file_mutation::content_hash(b"version two")
        ));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "version two");
    }

    #[tokio::test]
    async fn test_successful_expected_hash_edit_returns_before_and_after_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("guarded.txt");
        std::fs::write(&file, "before").unwrap();
        let before_hash = y_core::file_mutation::content_hash(b"before");

        let tool = FileEditTool::new();
        let input = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "before",
            "new_string": "after",
            "expected_content_hash": before_hash
        }));
        let output = tool.execute(input).await.unwrap();

        assert_eq!(output.content["before_hash"], before_hash);
        assert_eq!(
            output.content["after_hash"],
            y_core::file_mutation::content_hash(b"after")
        );
    }

    #[tokio::test]
    async fn test_parallel_edits_with_same_expected_hash_allow_only_one_writer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("parallel.txt");
        std::fs::write(&file, "alpha beta").unwrap();
        let expected_hash = y_core::file_mutation::content_hash(b"alpha beta");
        let tool = FileEditTool::new();
        let first = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "alpha",
            "new_string": "ALPHA",
            "expected_content_hash": expected_hash.clone()
        }));
        let second = make_input(serde_json::json!({
            "file_path": file.to_str().unwrap(),
            "old_string": "beta",
            "new_string": "BETA",
            "expected_content_hash": expected_hash
        }));

        let (first_result, second_result) = tokio::join!(tool.execute(first), tool.execute(second));
        let success_count = usize::from(first_result.is_ok()) + usize::from(second_result.is_ok());
        let stale_count = usize::from(matches!(first_result, Err(ToolError::StaleFile { .. })))
            + usize::from(matches!(second_result, Err(ToolError::StaleFile { .. })));

        assert_eq!(success_count, 1);
        assert_eq!(stale_count, 1);
    }

    #[tokio::test]
    async fn test_file_edit_resolves_relative_path_against_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let file = workspace
            .path()
            .join("notes")
            .join("__file_edit_unique__.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "hello workspace").unwrap();

        let tool = FileEditTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "file_path": "notes/__file_edit_unique__.txt",
                "old_string": "hello",
                "new_string": "goodbye"
            }),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "goodbye workspace");
    }

    #[tokio::test]
    async fn test_file_edit_rejects_path_outside_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::Builder::new()
            .prefix("outside-")
            .tempdir_in(target_test_dir())
            .unwrap();
        let outside_file = outside.path().join("__file_edit_outside__.txt");
        std::fs::write(&outside_file, "outside").unwrap();

        let tool = FileEditTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "file_path": outside_file.display().to_string(),
                "old_string": "outside",
                "new_string": "changed"
            }),
            workspace.path(),
        );
        let result = tool.execute(input).await;

        assert!(matches!(
            result,
            Err(ToolError::PermissionDenied { name, .. }) if name == "FileEdit"
        ));
        assert_eq!(std::fs::read_to_string(&outside_file).unwrap(), "outside");
    }

    #[tokio::test]
    async fn test_file_edit_allows_system_temp_path_outside_working_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file = temp_dir.path().join("__file_edit_temp_allowed__.txt");
        std::fs::write(&temp_file, "temporary old").unwrap();

        let tool = FileEditTool::new();
        let input = make_input_with_working_dir(
            serde_json::json!({
                "file_path": temp_file.display().to_string(),
                "old_string": "old",
                "new_string": "new"
            }),
            workspace.path(),
        );
        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(
            std::fs::read_to_string(&temp_file).unwrap(),
            "temporary new"
        );
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
        assert!(matches!(result, Err(ToolError::EditTargetNotFound { .. })));
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
        assert!(matches!(
            result,
            Err(ToolError::AmbiguousEdit { matches: 2, .. })
        ));
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
        assert!(matches!(result, Err(ToolError::FileNotFound { .. })));
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
        let mutation = def.capabilities.filesystem.mutation.unwrap();
        assert_eq!(mutation.path_argument, "file_path");
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
