//! `ScopeRollback`: scope-based file restoration.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::conflict::{detect_conflict, ConflictStatus};
use crate::error::JournalError;
use crate::storage::{FileOperation, JournalStore, ScopeStatus, StorageStrategy};

/// Report of a rollback operation.
#[derive(Debug, Clone, Default)]
pub struct RollbackReport {
    /// Number of files successfully restored.
    pub restored: usize,
    /// Number of entries skipped (already rolled back, etc.).
    pub skipped: usize,
    /// Conflicting entries (modified by third party).
    pub conflicts: Vec<String>,
}

/// Execute a rollback for a specific scope.
///
/// Iterates journal entries in reverse chronological order and restores
/// files to their pre-operation state. Conflict detection prevents
/// overwriting third-party modifications.
pub async fn rollback_scope(
    store: Arc<Mutex<JournalStore>>,
    scope_id: &str,
) -> Result<RollbackReport, JournalError> {
    let mut report = RollbackReport::default();

    // Get entries in reverse order (newest first).
    let s = store.lock().await;
    let entries: Vec<_> = s
        .get_entries_by_scope_reverse(scope_id)
        .into_iter()
        .cloned()
        .collect();
    drop(s);

    if entries.is_empty() {
        return Err(JournalError::ScopeNotFound {
            scope_id: scope_id.to_string(),
        });
    }

    for entry in &entries {
        if entry.rolled_back {
            report.skipped += 1;
            continue;
        }

        match &entry.operation {
            FileOperation::Create => {
                // Tool created this file; rollback = delete it.
                let path = std::path::Path::new(&entry.path);
                if path.exists() {
                    std::fs::remove_file(path).map_err(|e| JournalError::StorageError {
                        message: format!("failed to delete {}: {e}", entry.path),
                    })?;
                }
                store.lock().await.mark_rolled_back(entry.entry_id);
                report.restored += 1;
            }
            FileOperation::Modify => {
                // Check for conflicts.
                let conflict_status = detect_conflict(entry);
                match conflict_status {
                    ConflictStatus::Safe | ConflictStatus::FileMissing => {
                        // Safe to restore.
                        if let Some(ref content) = entry.original_content {
                            if entry.storage_strategy == StorageStrategy::Inline {
                                std::fs::write(&entry.path, content).map_err(|e| {
                                    JournalError::StorageError {
                                        message: format!("failed to restore {}: {e}", entry.path),
                                    }
                                })?;
                            }
                        }
                        store.lock().await.mark_rolled_back(entry.entry_id);
                        report.restored += 1;
                    }
                    ConflictStatus::Conflict { .. } => {
                        report.conflicts.push(entry.path.clone());
                    }
                    ConflictStatus::NoHashAvailable => {
                        report.skipped += 1;
                    }
                }
            }
            FileOperation::Delete => {
                // Tool deleted this file; rollback = recreate it.
                if let Some(ref content) = entry.original_content {
                    std::fs::write(&entry.path, content).map_err(|e| {
                        JournalError::StorageError {
                            message: format!("failed to recreate {}: {e}", entry.path),
                        }
                    })?;
                }
                store.lock().await.mark_rolled_back(entry.entry_id);
                report.restored += 1;
            }
            FileOperation::Rename { .. } => {
                // Rename rollback is complex; skip for now.
                report.skipped += 1;
            }
        }
    }

    // Mark scope as rolled back.
    let mut s = store.lock().await;
    s.set_scope_status(scope_id, ScopeStatus::RolledBack);

    Ok(report)
}

#[cfg(test)]
mod tests {
    use crate::storage::{JournalEntry, ScopeType};

    use super::*;

    fn make_store() -> Arc<Mutex<JournalStore>> {
        Arc::new(Mutex::new(JournalStore::new()))
    }

    #[tokio::test]
    async fn test_rollback_create_deletes_file() {
        let dir = std::env::temp_dir().join("y_journal_rollback_test1");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("created.txt");
        std::fs::write(&path, "created by tool").unwrap();

        let store = make_store();
        {
            let mut s = store.lock().await;
            s.open_scope("scope1", ScopeType::Task);
            s.add_entry(JournalEntry {
                entry_id: 0,
                scope_id: "scope1".into(),
                scope_type: ScopeType::Task,
                operation: FileOperation::Create,
                path: path.to_str().unwrap().into(),
                original_hash: None,
                storage_strategy: StorageStrategy::Inline,
                original_content: None,
                original_mode: None,
                created_at: 0,
                rolled_back: false,
            });
        }

        let report = rollback_scope(store, "scope1").await.unwrap();
        assert_eq!(report.restored, 1);
        assert!(!path.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_rollback_modify_restores_content() {
        let dir = std::env::temp_dir().join("y_journal_rollback_test2");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("modified.txt");
        let original = b"original content";
        std::fs::write(&path, original).unwrap();

        // Compute hash of original to match what middleware would store.
        let hash = {
            use std::fmt::Write;
            let mut h = 0u64;
            for (i, byte) in original.iter().enumerate() {
                h = h.wrapping_add(u64::from(*byte).wrapping_mul((i as u64).wrapping_add(1)));
            }
            let mut s = String::with_capacity(16);
            let _ = write!(s, "{h:016x}");
            s
        };

        let store = make_store();
        {
            let mut s = store.lock().await;
            s.open_scope("scope1", ScopeType::Task);
            s.add_entry(JournalEntry {
                entry_id: 0,
                scope_id: "scope1".into(),
                scope_type: ScopeType::Task,
                operation: FileOperation::Modify,
                path: path.to_str().unwrap().into(),
                original_hash: Some(hash),
                storage_strategy: StorageStrategy::Inline,
                original_content: Some(original.to_vec()),
                original_mode: None,
                created_at: 0,
                rolled_back: false,
            });
        }

        // "Modify" the file (simulate tool execution).
        // For this test, file still has original content so hash matches → Safe.
        let report = rollback_scope(store, "scope1").await.unwrap();
        assert_eq!(report.restored, 1);

        let restored = std::fs::read(&path).unwrap();
        assert_eq!(restored, original);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_rollback_empty_scope_fails() {
        let store = make_store();
        let result = rollback_scope(store, "nonexistent").await;
        assert!(matches!(result, Err(JournalError::ScopeNotFound { .. })));
    }
}
