//! Journal storage: in-memory store with three-tier strategy.

use serde::{Deserialize, Serialize};

/// Type of scope for grouping journal entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeType {
    Task,
    Pipeline,
    Checkpoint,
}

/// File operation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Create,
    Modify,
    Delete,
    Rename { from_path: String },
}

/// Storage strategy for the original content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageStrategy {
    /// Content stored inline (< 256KB).
    Inline,
    /// Content stored in external blob file.
    BlobFile { blob_path: String },
    /// Content recoverable from git ref.
    GitRef { commit_hash: String },
    /// Only metadata stored (file too large).
    MetadataOnly,
}

/// Scope status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeStatus {
    Open,
    Closed,
    RolledBack,
    Abandoned,
}

/// A scope record.
#[derive(Debug, Clone)]
pub struct JournalScope {
    pub scope_id: String,
    pub scope_type: ScopeType,
    pub status: ScopeStatus,
    pub created_at: i64,
    pub closed_at: Option<i64>,
}

/// A journal entry recording a file operation.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub entry_id: u64,
    pub scope_id: String,
    pub scope_type: ScopeType,
    pub operation: FileOperation,
    pub path: String,
    pub original_hash: Option<String>,
    pub storage_strategy: StorageStrategy,
    pub original_content: Option<Vec<u8>>,
    pub original_mode: Option<u32>,
    pub created_at: i64,
    pub rolled_back: bool,
}

/// In-memory journal store.
///
/// In a production implementation, this would be backed by `SQLite` co-located
/// with the Orchestrator checkpoint database.
pub struct JournalStore {
    entries: Vec<JournalEntry>,
    scopes: Vec<JournalScope>,
    next_entry_id: u64,
}

impl JournalStore {
    /// Create a new empty journal store.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scopes: Vec::new(),
            next_entry_id: 1,
        }
    }

    /// Open a new scope.
    pub fn open_scope(&mut self, scope_id: &str, scope_type: ScopeType) {
        self.scopes.push(JournalScope {
            scope_id: scope_id.to_string(),
            scope_type,
            status: ScopeStatus::Open,
            created_at: chrono::Utc::now().timestamp(),
            closed_at: None,
        });
    }

    /// Close a scope.
    pub fn close_scope(&mut self, scope_id: &str) -> bool {
        self.set_scope_status(scope_id, ScopeStatus::Closed)
    }

    /// Set scope status.
    pub fn set_scope_status(&mut self, scope_id: &str, status: ScopeStatus) -> bool {
        if let Some(scope) = self.scopes.iter_mut().find(|s| s.scope_id == scope_id) {
            if status == ScopeStatus::Closed {
                scope.closed_at = Some(chrono::Utc::now().timestamp());
            }
            scope.status = status;
            true
        } else {
            false
        }
    }

    /// Get a scope by ID.
    pub fn get_scope(&self, scope_id: &str) -> Option<&JournalScope> {
        self.scopes.iter().find(|s| s.scope_id == scope_id)
    }

    /// Add a journal entry, assigning an ID. Returns the assigned ID.
    pub fn add_entry(&mut self, mut entry: JournalEntry) -> u64 {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        entry.entry_id = id;
        self.entries.push(entry);
        id
    }

    /// Get all entries for a scope, in chronological order.
    pub fn get_entries_by_scope(&self, scope_id: &str) -> Vec<&JournalEntry> {
        self.entries
            .iter()
            .filter(|e| e.scope_id == scope_id)
            .collect()
    }

    /// Get all entries for a scope, in reverse chronological order (for rollback).
    pub fn get_entries_by_scope_reverse(&self, scope_id: &str) -> Vec<&JournalEntry> {
        let mut entries: Vec<&JournalEntry> = self
            .entries
            .iter()
            .filter(|e| e.scope_id == scope_id)
            .collect();
        entries.reverse();
        entries
    }

    /// Mark an entry as rolled back.
    pub fn mark_rolled_back(&mut self, entry_id: u64) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) {
            entry.rolled_back = true;
            true
        } else {
            false
        }
    }

    /// Count total entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

impl Default for JournalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_add_and_get_entries() {
        let mut store = JournalStore::new();
        store.open_scope("scope1", ScopeType::Task);

        let entry = JournalEntry {
            entry_id: 0,
            scope_id: "scope1".into(),
            scope_type: ScopeType::Task,
            operation: FileOperation::Modify,
            path: "/tmp/test.txt".into(),
            original_hash: Some("abc123".into()),
            storage_strategy: StorageStrategy::Inline,
            original_content: Some(b"original".to_vec()),
            original_mode: None,
            created_at: 1000,
            rolled_back: false,
        };
        let id = store.add_entry(entry);
        assert_eq!(id, 1);

        let entries = store.get_entries_by_scope("scope1");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/tmp/test.txt");
    }

    #[test]
    fn test_store_scope_lifecycle() {
        let mut store = JournalStore::new();
        store.open_scope("scope1", ScopeType::Pipeline);

        let scope = store.get_scope("scope1").unwrap();
        assert_eq!(scope.status, ScopeStatus::Open);

        store.close_scope("scope1");
        let scope = store.get_scope("scope1").unwrap();
        assert_eq!(scope.status, ScopeStatus::Closed);
    }

    #[test]
    fn test_store_reverse_order() {
        let mut store = JournalStore::new();
        for i in 0..3 {
            store.add_entry(JournalEntry {
                entry_id: 0,
                scope_id: "scope1".into(),
                scope_type: ScopeType::Task,
                operation: FileOperation::Modify,
                path: format!("/tmp/file{i}.txt"),
                original_hash: None,
                storage_strategy: StorageStrategy::Inline,
                original_content: None,
                original_mode: None,
                created_at: i64::from(i),
                rolled_back: false,
            });
        }

        let entries = store.get_entries_by_scope_reverse("scope1");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "/tmp/file2.txt");
        assert_eq!(entries[2].path, "/tmp/file0.txt");
    }

    #[test]
    fn test_store_mark_rolled_back() {
        let mut store = JournalStore::new();
        let id = store.add_entry(JournalEntry {
            entry_id: 0,
            scope_id: "scope1".into(),
            scope_type: ScopeType::Task,
            operation: FileOperation::Create,
            path: "/tmp/new.txt".into(),
            original_hash: None,
            storage_strategy: StorageStrategy::Inline,
            original_content: None,
            original_mode: None,
            created_at: 0,
            rolled_back: false,
        });
        assert!(store.mark_rolled_back(id));
        let entries = store.get_entries_by_scope("scope1");
        assert!(entries[0].rolled_back);
    }
}
