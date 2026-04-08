//! Conflict detection for journal rollback.

use std::path::Path;

use crate::storage::JournalEntry;

/// Result of a conflict check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictStatus {
    /// File is unmodified since journal capture — safe to rollback.
    Safe,
    /// File was modified by a third party — conflict.
    Conflict { current_hash: String },
    /// File no longer exists on disk.
    FileMissing,
    /// No hash available for comparison (metadata-only capture).
    NoHashAvailable,
}

/// Check if a file has been modified since the journal entry was created.
pub fn detect_conflict(entry: &JournalEntry) -> ConflictStatus {
    let path = Path::new(&entry.path);

    if !path.exists() {
        return ConflictStatus::FileMissing;
    }

    let Some(expected_hash) = &entry.original_hash else {
        return ConflictStatus::NoHashAvailable;
    };

    // Compute current hash.
    match std::fs::read(path) {
        Ok(content) => {
            let current_hash = compute_hash(&content);
            if current_hash == *expected_hash {
                ConflictStatus::Safe
            } else {
                ConflictStatus::Conflict { current_hash }
            }
        }
        Err(_) => ConflictStatus::FileMissing,
    }
}

/// Compute a hash for conflict detection (same algorithm as middleware).
fn compute_hash(content: &[u8]) -> String {
    use std::fmt::Write;
    let mut hash = 0u64;
    for (i, byte) in content.iter().enumerate() {
        hash = hash.wrapping_add(u64::from(*byte).wrapping_mul((i as u64).wrapping_add(1)));
    }
    let mut s = String::with_capacity(16);
    let _ = write!(s, "{hash:016x}");
    s
}

#[cfg(test)]
mod tests {
    use crate::storage::{FileOperation, ScopeType, StorageStrategy};

    use super::*;

    fn make_entry(path: &str, hash: Option<String>) -> JournalEntry {
        JournalEntry {
            entry_id: 1,
            scope_id: "scope1".into(),
            scope_type: ScopeType::Task,
            operation: FileOperation::Modify,
            path: path.into(),
            original_hash: hash,
            storage_strategy: StorageStrategy::Inline,
            original_content: Some(b"original".to_vec()),
            original_mode: None,
            created_at: 0,
            rolled_back: false,
        }
    }

    #[test]
    fn test_conflict_detection_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.txt");
        let entry = make_entry(path.to_str().unwrap(), Some("abc".into()));
        assert_eq!(detect_conflict(&entry), ConflictStatus::FileMissing);
    }

    #[test]
    fn test_conflict_detection_no_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nohash.txt");
        std::fs::write(&path, "some content").unwrap();

        let entry = make_entry(path.to_str().unwrap(), None);
        assert_eq!(detect_conflict(&entry), ConflictStatus::NoHashAvailable);
    }

    #[test]
    fn test_conflict_detection_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("safe.txt");
        let content = b"safe content";
        std::fs::write(&path, content).unwrap();

        let hash = compute_hash(content);
        let entry = make_entry(path.to_str().unwrap(), Some(hash));
        assert_eq!(detect_conflict(&entry), ConflictStatus::Safe);
    }

    #[test]
    fn test_conflict_detection_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conflict.txt");

        // Write original.
        let original = b"original content";
        std::fs::write(&path, original).unwrap();
        let original_hash = compute_hash(original);

        // Modify the file (third-party).
        std::fs::write(&path, b"modified by someone else").unwrap();

        let entry = make_entry(path.to_str().unwrap(), Some(original_hash));
        let status = detect_conflict(&entry);
        assert!(matches!(status, ConflictStatus::Conflict { .. }));
    }
}
