//! `FileJournalMiddleware`: intercepts file-mutating tool calls and captures state.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::JournalError;
use crate::hash::compute_sha256_hex;
use crate::storage::{FileOperation, JournalEntry, JournalStore, ScopeType, StorageStrategy};

/// List of core tools known to mutate the filesystem.
const FILE_MUTATING_TOOLS: &[&str] = &["FileWrite", "FilePatch", "FileDelete"];

/// Maximum file size for inline storage (256KB).
const _INLINE_THRESHOLD: u64 = 256 * 1024;

/// Maximum file size for blob storage (10MB).
const BLOB_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Middleware that captures file state before tool execution.
///
/// Operates as a pre-execution hook: reads the target file's content and
/// metadata, stores a journal entry, then allows the tool to proceed.
/// On capture failure, the tool still executes (fail-open).
pub struct FileJournalMiddleware {
    store: Arc<Mutex<JournalStore>>,
    /// Override set of tool names considered file-mutating.
    mutating_tools: HashSet<String>,
}

impl FileJournalMiddleware {
    /// Create a new middleware with the given journal store.
    pub fn new(store: Arc<Mutex<JournalStore>>) -> Self {
        let mutating_tools = FILE_MUTATING_TOOLS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        Self {
            store,
            mutating_tools,
        }
    }

    /// Check if a tool is file-mutating.
    pub fn is_file_mutating(&self, tool_name: &str) -> bool {
        self.mutating_tools.contains(tool_name)
    }

    /// Capture file state before a tool execution.
    ///
    /// Returns `Ok(Some(entry_id))` if captured, `Ok(None)` if not applicable,
    /// or `Err` if capture fails (but tool should still proceed).
    pub async fn capture_before(
        &self,
        scope_id: &str,
        scope_type: ScopeType,
        tool_name: &str,
        file_path: &str,
    ) -> Result<Option<u64>, JournalError> {
        if !self.is_file_mutating(tool_name) {
            return Ok(None);
        }

        let path = std::path::Path::new(file_path);

        let (operation, content, hash, file_mode) = if path.exists() {
            let metadata = std::fs::metadata(path).map_err(|e| JournalError::CaptureFailed {
                path: file_path.to_string(),
                message: e.to_string(),
            })?;

            let size = metadata.len();

            if size > BLOB_THRESHOLD {
                // Too large — metadata only.
                tracing::warn!(path = %file_path, size, "file too large for journal; metadata only");
                let hash = compute_hash_from_path(path)?;
                (FileOperation::Modify, None, Some(hash), None)
            } else {
                let content = std::fs::read(path).map_err(|e| JournalError::CaptureFailed {
                    path: file_path.to_string(),
                    message: e.to_string(),
                })?;
                let hash = compute_sha256_hex(&content);

                let strategy_content = Some(content);

                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    Some(metadata.permissions().mode())
                };
                #[cfg(not(unix))]
                let mode: Option<u32> = None;

                (FileOperation::Modify, strategy_content, Some(hash), mode)
            }
        } else {
            // File doesn't exist yet — tool will create it.
            (FileOperation::Create, None, None, None)
        };

        let strategy = if content.is_some() {
            StorageStrategy::Inline
        } else {
            StorageStrategy::MetadataOnly
        };

        let entry = JournalEntry {
            entry_id: 0, // Assigned by store.
            scope_id: scope_id.to_string(),
            scope_type,
            operation,
            path: file_path.to_string(),
            original_hash: hash,
            storage_strategy: strategy,
            original_content: content,
            original_mode: file_mode,
            created_at: chrono::Utc::now().timestamp(),
            rolled_back: false,
        };

        let id = self.store.lock().await.add_entry(entry);
        Ok(Some(id))
    }
}

/// Compute hash from a file path.
fn compute_hash_from_path(path: &std::path::Path) -> Result<String, JournalError> {
    let content = std::fs::read(path).map_err(|e| JournalError::CaptureFailed {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(compute_sha256_hex(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> Arc<Mutex<JournalStore>> {
        Arc::new(Mutex::new(JournalStore::new()))
    }

    #[test]
    fn test_middleware_identifies_file_mutating_tools() {
        let mw = FileJournalMiddleware::new(make_store());
        assert!(mw.is_file_mutating("FileWrite"));
        assert!(mw.is_file_mutating("FilePatch"));
        assert!(mw.is_file_mutating("FileDelete"));
        assert!(!mw.is_file_mutating("FileRead"));
        assert!(!mw.is_file_mutating("web_search"));
    }

    #[tokio::test]
    async fn test_middleware_skips_non_mutating_tools() {
        let mw = FileJournalMiddleware::new(make_store());
        let result = mw
            .capture_before("scope1", ScopeType::Task, "FileRead", "/tmp/test.txt")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_middleware_captures_new_file_creation() {
        let store = make_store();
        let mw = FileJournalMiddleware::new(store.clone());
        let result = mw
            .capture_before(
                "scope1",
                ScopeType::Task,
                "FileWrite",
                "/tmp/nonexistent_y_journal_test_file.txt",
            )
            .await
            .unwrap();
        assert!(result.is_some());

        let s = store.lock().await;
        let entries = s.get_entries_by_scope("scope1");
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].operation, FileOperation::Create));
    }

    #[tokio::test]
    async fn test_middleware_captures_existing_file() {
        // Create a temp file.
        let dir = std::env::temp_dir().join("y_journal_test");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("existing.txt");
        std::fs::write(&path, "original content").unwrap();

        let store = make_store();
        let mw = FileJournalMiddleware::new(store.clone());
        let result = mw
            .capture_before(
                "scope1",
                ScopeType::Pipeline,
                "FileWrite",
                path.to_str().unwrap(),
            )
            .await
            .unwrap();
        assert!(result.is_some());

        let s = store.lock().await;
        let entries = s.get_entries_by_scope("scope1");
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].operation, FileOperation::Modify));
        assert!(entries[0].original_content.is_some());
        assert!(entries[0].original_hash.is_some());

        // Cleanup.
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let data = b"hello world";
        let hash1 = compute_sha256_hex(data);
        let hash2 = compute_sha256_hex(data);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_compute_hash_matches_sha256() {
        let hash = compute_sha256_hex(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
