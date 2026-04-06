//! Persistent file history for rewind support.
//!
//! Manages per-session file backups and snapshots at user message
//! boundaries. When a rewind is requested, files are restored from
//! their backups to the state at the target snapshot.
//!
//! Design reference: chat-checkpoint-design.md, file-journal-design.md
//!
//! Inspired by Claude Code's `fileHistory.ts` but adapted for the
//! y-agent architecture (`SQLite` metadata + flat file backups).

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::JournalError;

/// Maximum number of snapshots retained per session.
const MAX_SNAPSHOTS_PER_SESSION: usize = 100;

/// Maximum file size eligible for backup (10 MB).
const MAX_BACKUP_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// A backup record for a single file at a specific version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBackup {
    /// Name of the backup file (without directory). `None` if the file
    /// did not exist before the mutation (i.e., it was created).
    pub backup_file_name: Option<String>,
    /// Version counter for this file within the session.
    pub version: u32,
    /// Unix timestamp of when the backup was taken.
    pub backup_time: i64,
}

/// Snapshot of all tracked files at a user message boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileHistorySnapshot {
    /// The user message ID that this snapshot is anchored to.
    pub message_id: String,
    /// Map of absolute file path -> backup record at snapshot time.
    pub file_backups: HashMap<String, FileBackup>,
    /// Unix timestamp of snapshot creation.
    pub timestamp: i64,
}

/// Result of a rewind operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindReport {
    /// Files successfully restored to their pre-snapshot state.
    pub restored: Vec<String>,
    /// Files that were created after the snapshot and deleted during rewind.
    pub deleted: Vec<String>,
    /// Files that could not be restored (external modifications, etc.).
    pub conflicts: Vec<RewindConflict>,
}

/// A conflict encountered during rewind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindConflict {
    /// Absolute path of the conflicted file.
    pub path: String,
    /// Human-readable reason for the conflict.
    pub reason: String,
}

/// Diff statistics for display in the rewind UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    /// Number of files changed since this snapshot.
    pub files_changed: usize,
    /// Number of files created after this snapshot.
    pub files_created: usize,
    /// Number of files deleted after this snapshot (currently unused).
    pub files_deleted: usize,
}

/// A rewind point displayed in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindPoint {
    /// User message ID for this rewind point.
    pub message_id: String,
    /// Preview of the user message content.
    pub message_preview: String,
    /// Timestamp of the snapshot.
    pub timestamp: i64,
    /// Diff stats relative to current filesystem state.
    pub diff_stats: DiffStats,
}

/// Persistent file history manager for a single session.
///
/// Stores file backups as flat files in a session-scoped directory
/// (`{data_dir}/file-history/{session_id}/`). Snapshot metadata is
/// held in memory and serialized to a JSON sidecar file.
pub struct FileHistoryManager {
    /// Session ID this manager belongs to.
    session_id: String,
    /// Directory where backup files are stored.
    backup_dir: PathBuf,
    /// Ordered list of snapshots (oldest first).
    snapshots: VecDeque<FileHistorySnapshot>,
    /// Set of file paths currently being tracked.
    tracked_files: HashSet<String>,
    /// Per-file version counter (path -> next version).
    file_versions: HashMap<String, u32>,
}

impl FileHistoryManager {
    /// Create a new file history manager for a session.
    ///
    /// Creates the backup directory if it does not exist.
    pub fn new(session_id: &str, data_dir: &Path) -> Result<Self, JournalError> {
        let backup_dir = data_dir.join("file-history").join(session_id);
        std::fs::create_dir_all(&backup_dir).map_err(|e| JournalError::CaptureFailed {
            path: backup_dir.display().to_string(),
            message: format!("failed to create backup directory: {e}"),
        })?;

        let mut manager = Self {
            session_id: session_id.to_string(),
            backup_dir,
            snapshots: VecDeque::new(),
            tracked_files: HashSet::new(),
            file_versions: HashMap::new(),
        };

        // Load existing state from the sidecar if it exists.
        manager.load_state();

        Ok(manager)
    }

    /// Track a file edit by creating a backup before the mutation occurs.
    ///
    /// Called by the file journal middleware before each file-mutating
    /// tool call. Returns `Ok(true)` if a backup was created, `Ok(false)`
    /// if skipped (non-existent file = creation, or file too large).
    pub fn track_edit(&mut self, file_path: &str) -> Result<bool, JournalError> {
        let path = Path::new(file_path);

        if !path.exists() {
            // File does not exist yet -- tool will create it.
            // Track it so we know to delete it on rewind.
            self.tracked_files.insert(file_path.to_string());
            debug!(path = %file_path, "tracking new file creation (no backup needed)");
            return Ok(false);
        }

        // Check file size.
        let metadata = std::fs::metadata(path).map_err(|e| JournalError::CaptureFailed {
            path: file_path.to_string(),
            message: e.to_string(),
        })?;

        if metadata.len() > MAX_BACKUP_FILE_SIZE {
            warn!(
                path = %file_path,
                size = metadata.len(),
                "file too large for backup; skipping"
            );
            return Ok(false);
        }

        // Compute backup file name: {hash_prefix}@v{version}
        let version = self.next_version(file_path);
        let hash_prefix = path_hash(file_path);
        let backup_name = format!("{hash_prefix}@v{version}");
        let backup_path = self.backup_dir.join(&backup_name);

        // Copy file to backup location.
        std::fs::copy(path, &backup_path).map_err(|e| JournalError::CaptureFailed {
            path: file_path.to_string(),
            message: format!("failed to create backup: {e}"),
        })?;

        self.tracked_files.insert(file_path.to_string());

        debug!(
            path = %file_path,
            backup = %backup_name,
            version,
            "file backup created"
        );

        Ok(true)
    }

    /// Create a snapshot at the current user message boundary.
    ///
    /// Records the latest backup version for each tracked file.
    /// Called before each user message is processed.
    pub fn make_snapshot(&mut self, message_id: &str) {
        // Build the snapshot from current tracked state.
        let mut file_backups = HashMap::new();

        for file_path in &self.tracked_files {
            let version = self
                .file_versions
                .get(file_path)
                .copied()
                .unwrap_or(0)
                .saturating_sub(1);

            if version == 0 && !Path::new(file_path).exists() {
                // File was tracked as "will be created" but we have no
                // backup for it. Record with backup_file_name = None.
                file_backups.insert(
                    file_path.clone(),
                    FileBackup {
                        backup_file_name: None,
                        version: 0,
                        backup_time: chrono::Utc::now().timestamp(),
                    },
                );
                continue;
            }

            let hash_prefix = path_hash(file_path);
            let backup_name = if version > 0 {
                Some(format!("{hash_prefix}@v{version}"))
            } else {
                // Check if the file existed at start (version 0 means
                // the first edit bumped it to 1, so the backup is @v1).
                let v1_name = format!("{hash_prefix}@v1");
                if self.backup_dir.join(&v1_name).exists() {
                    Some(v1_name)
                } else {
                    None
                }
            };

            file_backups.insert(
                file_path.clone(),
                FileBackup {
                    backup_file_name: backup_name,
                    version,
                    backup_time: chrono::Utc::now().timestamp(),
                },
            );
        }

        let snapshot = FileHistorySnapshot {
            message_id: message_id.to_string(),
            file_backups,
            timestamp: chrono::Utc::now().timestamp(),
        };

        self.snapshots.push_back(snapshot);

        // Enforce retention cap (O(1) eviction with VecDeque).
        while self.snapshots.len() > MAX_SNAPSHOTS_PER_SESSION {
            self.snapshots.pop_front();
        }

        self.save_state();

        debug!(
            session = %self.session_id,
            message_id,
            tracked_files = self.tracked_files.len(),
            snapshots = self.snapshots.len(),
            "file history snapshot created"
        );
    }

    /// Rewind all tracked files to the state at the given snapshot.
    ///
    /// For files that existed at the snapshot: restore from backup.
    /// For files created after the snapshot: delete them.
    /// For files modified externally since the backup: report as conflict.
    pub fn rewind_to(&mut self, message_id: &str) -> Result<RewindReport, JournalError> {
        let snapshot_idx = self
            .snapshots
            .iter()
            .position(|s| s.message_id == message_id)
            .ok_or_else(|| JournalError::ScopeNotFound {
                scope_id: format!("snapshot:{message_id}"),
            })?;

        // Extract needed data from the target snapshot without cloning the
        // entire structure. We collect (path, Option<backup_name>) pairs.
        let target_entries: Vec<(String, Option<String>)> = self.snapshots[snapshot_idx]
            .file_backups
            .iter()
            .map(|(path, backup)| (path.clone(), backup.backup_file_name.clone()))
            .collect();

        let target_keys: HashSet<&str> = target_entries.iter().map(|(p, _)| p.as_str()).collect();

        // Collect files tracked in snapshots AFTER the target that are not
        // in the target snapshot -- candidates for deletion.
        let files_after: HashSet<String> = self
            .snapshots
            .range(snapshot_idx + 1..)
            .flat_map(|s| s.file_backups.keys())
            .filter(|p| !target_keys.contains(p.as_str()))
            .cloned()
            .collect();

        // Truncate snapshots: remove everything after the target.
        while self.snapshots.len() > snapshot_idx + 1 {
            self.snapshots.pop_back();
        }
        self.save_state();

        // Now perform filesystem operations (no borrows on self.snapshots).
        let mut report = RewindReport {
            restored: Vec::new(),
            deleted: Vec::new(),
            conflicts: Vec::new(),
        };

        for (file_path, backup_name) in &target_entries {
            match backup_name {
                Some(name) => {
                    let backup_path = self.backup_dir.join(name);
                    if !backup_path.exists() {
                        report.conflicts.push(RewindConflict {
                            path: file_path.clone(),
                            reason: format!("backup file '{name}' not found"),
                        });
                        continue;
                    }

                    if let Some(parent) = Path::new(file_path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::copy(&backup_path, file_path) {
                        Ok(_) => report.restored.push(file_path.clone()),
                        Err(e) => report.conflicts.push(RewindConflict {
                            path: file_path.clone(),
                            reason: format!("failed to restore: {e}"),
                        }),
                    }
                }
                None => {
                    if Path::new(file_path).exists() {
                        match std::fs::remove_file(file_path) {
                            Ok(()) => report.deleted.push(file_path.clone()),
                            Err(e) => report.conflicts.push(RewindConflict {
                                path: file_path.clone(),
                                reason: format!("failed to delete: {e}"),
                            }),
                        }
                    }
                }
            }
        }

        // Delete files created in subsequent snapshots.
        for file_path in &files_after {
            if Path::new(file_path).exists() {
                match std::fs::remove_file(file_path) {
                    Ok(()) => report.deleted.push(file_path.clone()),
                    Err(e) => report.conflicts.push(RewindConflict {
                        path: file_path.clone(),
                        reason: format!("failed to delete created file: {e}"),
                    }),
                }
            }
        }

        debug!(
            session = %self.session_id,
            message_id,
            restored = report.restored.len(),
            deleted = report.deleted.len(),
            conflicts = report.conflicts.len(),
            "file history rewind completed"
        );

        Ok(report)
    }

    /// Compute diff stats between the target snapshot and the current
    /// filesystem state (used for UI display).
    pub fn diff_stats_for(&self, message_id: &str) -> Option<DiffStats> {
        let snapshot_idx = self
            .snapshots
            .iter()
            .position(|s| s.message_id == message_id)?;

        let mut files_changed = 0;
        let mut files_created = 0;

        let files_at_snapshot: HashSet<&String> =
            self.snapshots[snapshot_idx].file_backups.keys().collect();

        let files_after: HashSet<&String> = self
            .snapshots
            .range(snapshot_idx + 1..)
            .flat_map(|s| s.file_backups.keys())
            .collect();

        for file_path in &files_after {
            if files_at_snapshot.contains(file_path) {
                files_changed += 1;
            } else {
                files_created += 1;
            }
        }

        Some(DiffStats {
            files_changed,
            files_created,
            files_deleted: 0,
        })
    }

    /// Compute diff stats for all snapshots in a single O(n) reverse pass.
    ///
    /// Returns `(message_id, DiffStats)` pairs for every snapshot. Much more
    /// efficient than calling `diff_stats_for` per snapshot (avoids O(n^2)).
    pub fn diff_stats_batch(&self) -> Vec<(String, DiffStats)> {
        if self.snapshots.is_empty() {
            return Vec::new();
        }

        let len = self.snapshots.len();
        let mut results = vec![
            DiffStats {
                files_changed: 0,
                files_created: 0,
                files_deleted: 0,
            };
            len
        ];

        // Walk backwards from the last snapshot, accumulating the set of
        // files that appear in snapshots after each index.
        let mut cumulative_after: HashSet<&String> = HashSet::new();

        for i in (0..len).rev() {
            // For this snapshot, diff_stats = how cumulative_after relates
            // to the files at this snapshot.
            let files_at: HashSet<&String> = self.snapshots[i].file_backups.keys().collect();

            let mut changed = 0usize;
            let mut created = 0usize;
            for f in &cumulative_after {
                if files_at.contains(*f) {
                    changed += 1;
                } else {
                    created += 1;
                }
            }
            results[i] = DiffStats {
                files_changed: changed,
                files_created: created,
                files_deleted: 0,
            };

            // Add this snapshot's files to cumulative set for earlier indices.
            cumulative_after.extend(self.snapshots[i].file_backups.keys());
        }

        self.snapshots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.message_id.clone(), results[i].clone()))
            .collect()
    }

    /// Check if there are any file changes since the given snapshot.
    pub fn has_changes_since(&self, message_id: &str) -> bool {
        self.diff_stats_for(message_id)
            .is_some_and(|s| s.files_changed > 0 || s.files_created > 0)
    }

    /// Return all snapshots (for UI listing).
    pub fn snapshots(&self) -> &VecDeque<FileHistorySnapshot> {
        &self.snapshots
    }

    /// Return the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Clean up all backup files for this session.
    pub fn cleanup(&self) {
        if self.backup_dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&self.backup_dir) {
                warn!(
                    session = %self.session_id,
                    error = %e,
                    "failed to clean up file history backup directory"
                );
            }
        }
    }

    // -- Private helpers --------------------------------------------------

    /// Get the next version number for a file and increment the counter.
    fn next_version(&mut self, file_path: &str) -> u32 {
        let entry = self.file_versions.entry(file_path.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Path to the JSON sidecar file storing snapshot metadata.
    fn state_path(&self) -> PathBuf {
        self.backup_dir.join("_snapshots.json")
    }

    /// Persist snapshot state to disk using compact, streaming JSON.
    fn save_state(&self) {
        let state = FileHistoryState {
            snapshots: &self.snapshots,
            file_versions: &self.file_versions,
            tracked_files: &self.tracked_files,
        };
        let path = self.state_path();
        match std::fs::File::create(&path) {
            Ok(file) => {
                let writer = BufWriter::new(file);
                if let Err(e) = serde_json::to_writer(writer, &state) {
                    warn!(error = %e, "failed to serialize file history state");
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to create file history state file");
            }
        }
    }

    /// Load snapshot state from disk.
    fn load_state(&mut self) {
        let path = self.state_path();
        if !path.exists() {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<FileHistoryStateOwned>(&json) {
                Ok(state) => {
                    self.snapshots = state.snapshots.into();
                    self.file_versions = state.file_versions;
                    self.tracked_files = state.tracked_files.into_iter().collect();
                    debug!(
                        session = %self.session_id,
                        snapshots = self.snapshots.len(),
                        tracked_files = self.tracked_files.len(),
                        "file history state loaded from disk"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "failed to deserialize file history state; starting fresh");
                }
            },
            Err(e) => {
                warn!(error = %e, "failed to read file history state file");
            }
        }
    }
}

/// Borrowed state for zero-copy serialization to disk.
#[derive(Serialize)]
struct FileHistoryState<'a> {
    snapshots: &'a VecDeque<FileHistorySnapshot>,
    file_versions: &'a HashMap<String, u32>,
    tracked_files: &'a HashSet<String>,
}

/// Owned state for deserialization from disk.
#[derive(Deserialize)]
struct FileHistoryStateOwned {
    snapshots: Vec<FileHistorySnapshot>,
    file_versions: HashMap<String, u32>,
    tracked_files: Vec<String>,
}

/// Compute a stable 16-character hex hash prefix from a file path.
///
/// Uses a simple FNV-1a-style hash for speed and determinism.
fn path_hash(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_manager() -> (FileHistoryManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager =
            FileHistoryManager::new("test-session", dir.path()).expect("manager creation");
        (manager, dir)
    }

    #[test]
    fn test_path_hash_deterministic() {
        let h1 = path_hash("/tmp/test.txt");
        let h2 = path_hash("/tmp/test.txt");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn test_path_hash_different_paths() {
        assert_ne!(path_hash("/tmp/a.txt"), path_hash("/tmp/b.txt"));
    }

    #[test]
    fn test_track_new_file_creation() {
        let (mut mgr, _dir) = setup_manager();
        let result = mgr.track_edit("/tmp/nonexistent_y_fh_test.txt").unwrap();
        assert!(!result); // No backup for non-existent file.
        assert!(mgr.tracked_files.contains("/tmp/nonexistent_y_fh_test.txt"));
    }

    #[test]
    fn test_track_existing_file() {
        let (mut mgr, _dir) = setup_manager();

        // Create a temp file.
        let test_dir = std::env::temp_dir().join("y_fh_test");
        std::fs::create_dir_all(&test_dir).ok();
        let file_path = test_dir.join("existing.txt");
        std::fs::write(&file_path, "original content").unwrap();

        let result = mgr.track_edit(file_path.to_str().unwrap()).unwrap();
        assert!(result);
        assert!(mgr.tracked_files.contains(file_path.to_str().unwrap()));

        // Verify backup file exists.
        let hash_prefix = path_hash(file_path.to_str().unwrap());
        let backup_path = mgr.backup_dir.join(format!("{hash_prefix}@v1"));
        assert!(backup_path.exists());

        // Cleanup.
        std::fs::remove_dir_all(&test_dir).ok();
    }

    #[test]
    fn test_snapshot_and_rewind() {
        let (mut mgr, _dir) = setup_manager();

        // Create a test file.
        let test_dir = std::env::temp_dir().join("y_fh_test_rewind");
        std::fs::create_dir_all(&test_dir).ok();
        let file_path = test_dir.join("rewindable.txt");
        std::fs::write(&file_path, "version 1").unwrap();

        // Track edit and make snapshot.
        mgr.track_edit(file_path.to_str().unwrap()).unwrap();
        mgr.make_snapshot("msg-001");

        // Modify the file.
        std::fs::write(&file_path, "version 2").unwrap();
        mgr.track_edit(file_path.to_str().unwrap()).unwrap();
        mgr.make_snapshot("msg-002");

        // Verify file is at version 2.
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "version 2");

        // Rewind to msg-001.
        let report = mgr.rewind_to("msg-001").unwrap();
        assert!(!report.restored.is_empty());
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "version 1");

        // Cleanup.
        std::fs::remove_dir_all(&test_dir).ok();
    }

    #[test]
    fn test_diff_stats() {
        let (mut mgr, _dir) = setup_manager();
        mgr.make_snapshot("msg-001");

        // Track a new file in a subsequent snapshot.
        let test_dir = std::env::temp_dir().join("y_fh_test_diff");
        std::fs::create_dir_all(&test_dir).ok();
        let file_path = test_dir.join("new_file.txt");

        mgr.tracked_files
            .insert(file_path.to_str().unwrap().to_string());
        mgr.make_snapshot("msg-002");

        let stats = mgr.diff_stats_for("msg-001").unwrap();
        assert_eq!(stats.files_created, 1);

        // Cleanup.
        std::fs::remove_dir_all(&test_dir).ok();
    }

    #[test]
    fn test_state_persistence() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create manager, track something, make a snapshot.
        {
            let mut mgr =
                FileHistoryManager::new("persist-test", dir.path()).expect("manager creation");
            mgr.tracked_files.insert("/tmp/test.txt".to_string());
            mgr.make_snapshot("msg-001");
        }

        // Create a new manager for the same session -- should load state.
        let mgr = FileHistoryManager::new("persist-test", dir.path()).expect("manager creation");
        assert_eq!(mgr.snapshots.len(), 1);
        assert_eq!(mgr.snapshots[0].message_id, "msg-001");
        assert!(mgr.tracked_files.contains("/tmp/test.txt"));
    }

    #[test]
    fn test_snapshot_retention_cap() {
        let (mut mgr, _dir) = setup_manager();

        for i in 0..MAX_SNAPSHOTS_PER_SESSION + 10 {
            mgr.make_snapshot(&format!("msg-{i:04}"));
        }

        assert_eq!(mgr.snapshots.len(), MAX_SNAPSHOTS_PER_SESSION);
        // Oldest snapshots should have been evicted.
        assert_eq!(mgr.snapshots[0].message_id, "msg-0010");
    }
}
