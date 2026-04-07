//! Rewind service --- atomic rollback of conversation and filesystem state.
//!
//! Orchestrates three-phase rollback:
//! 1. Transcript truncation (via `TranscriptStore::truncate`)
//! 2. File restoration   (via `FileHistoryManager::rewind_to`)
//! 3. Checkpoint invalidation + session state reset
//!
//! Design reference: Implementation plan for rewind feature.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use y_core::types::Role;

use y_core::types::SessionId;
use y_journal::{DiffStats, FileHistoryManager, RewindReport};

use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Session-scoped file history managers
// ---------------------------------------------------------------------------

/// Thread-safe map of session ID -> `FileHistoryManager`.
pub type FileHistoryManagers = Arc<RwLock<HashMap<SessionId, FileHistoryManager>>>;

/// Create an empty file history managers map.
pub fn create_file_history_managers() -> FileHistoryManagers {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// RewindService
// ---------------------------------------------------------------------------

/// Errors from rewind operations.
#[derive(Debug, thiserror::Error)]
pub enum RewindError {
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("no file history for session: {0}")]
    NoHistory(String),

    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("transcript truncation failed: {0}")]
    TruncateFailed(String),

    #[error("checkpoint error: {0}")]
    CheckpointError(String),

    #[error("file restoration error: {0}")]
    FileError(String),
}

/// Info about a rewind point for display in the GUI/TUI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RewindPointInfo {
    /// Message ID of the user message at this boundary.
    pub message_id: String,
    /// Turn number (1-indexed).
    pub turn_number: u32,
    /// Preview of the user message.
    pub message_preview: String,
    /// Timestamp of the snapshot.
    pub timestamp: i64,
    /// Diff stats relative to the current state.
    pub diff_stats: DiffStats,
}

/// Result of a completed rewind operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RewindResult {
    /// Target message ID that was rewound to.
    pub target_message_id: String,
    /// Number of messages removed from transcript.
    pub messages_removed: usize,
    /// Number of checkpoints invalidated.
    pub checkpoints_invalidated: u32,
    /// File restoration report.
    pub file_report: RewindReport,
}

/// Service for listing rewind points and executing rewind operations.
pub struct RewindService;

impl RewindService {
    /// List available rewind points for a session.
    ///
    /// Returns the points in reverse chronological order (most recent first).
    pub async fn list_rewind_points(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Result<Vec<RewindPointInfo>, RewindError> {
        // 1. Get snapshot data and batch diff_stats from the file history
        //    manager, then release the read lock immediately.
        let (snapshot_data, diff_map) = {
            let managers = container.file_history_managers.read().await;
            let mgr = managers
                .get(session_id)
                .ok_or_else(|| RewindError::NoHistory(session_id.0.clone()))?;

            let snapshots: Vec<_> = mgr
                .snapshots()
                .iter()
                .map(|s| (s.message_id.clone(), s.timestamp))
                .collect();

            let diff_batch: HashMap<String, DiffStats> =
                mgr.diff_stats_batch().into_iter().collect();

            (snapshots, diff_batch)
        };

        // 2. Get checkpoints and build an index keyed by scope content.
        let checkpoints = container
            .chat_checkpoint_manager
            .checkpoint_store()
            .list_by_session(session_id)
            .await
            .map_err(|e| RewindError::CheckpointError(e.to_string()))?;

        // Checkpoint scope_id is a string that contains the message_id.
        // Build a vec of (scope_id, turn_number) for substring matching.
        let cp_entries: Vec<(&str, u32)> = checkpoints
            .iter()
            .map(|cp| (cp.journal_scope_id.as_str(), cp.turn_number))
            .collect();

        // 3. Read display transcript and build a message preview index.
        let transcript = container
            .session_manager
            .read_display_transcript(session_id)
            .await
            .unwrap_or_default();

        let preview_index: HashMap<&str, &str> = transcript
            .iter()
            .filter(|m| m.role == Role::User)
            .map(|m| (m.message_id.as_str(), m.content.as_str()))
            .collect();

        // 4. Build rewind points with O(1) lookups per snapshot.
        let empty_stats = DiffStats {
            files_changed: 0,
            files_created: 0,
            files_deleted: 0,
        };

        let mut points: Vec<RewindPointInfo> = snapshot_data
            .iter()
            .filter_map(|(message_id, timestamp)| {
                let diff_stats = diff_map.get(message_id).unwrap_or(&empty_stats);

                if diff_stats.files_changed == 0 && diff_stats.files_created == 0 {
                    return None;
                }

                let turn_number = cp_entries
                    .iter()
                    .find(|(scope, _)| scope.contains(message_id.as_str()))
                    .map_or(0, |(_, tn)| *tn);

                // Skip orphaned snapshots whose message was removed by
                // undo/truncation. These have no entry in the display
                // transcript and would show as "[unknown message]".
                let content = preview_index.get(message_id.as_str())?;
                let preview = {
                    let truncated: String = content.chars().take(100).collect();
                    if content.chars().count() > 100 {
                        format!("{truncated}...")
                    } else {
                        truncated
                    }
                };

                Some(RewindPointInfo {
                    message_id: message_id.clone(),
                    turn_number,
                    message_preview: preview,
                    timestamp: *timestamp,
                    diff_stats: diff_stats.clone(),
                })
            })
            .collect();

        // Reverse to show most recent first.
        points.reverse();

        Ok(points)
    }

    /// Execute a rewind to a specific message boundary.
    ///
    /// Three-phase operation:
    /// 1. Truncate conversation transcripts (context + display)
    /// 2. Restore files from backups
    /// 3. Invalidate checkpoints after the target
    pub async fn execute_rewind(
        container: &ServiceContainer,
        session_id: &SessionId,
        target_message_id: &str,
    ) -> Result<RewindResult, RewindError> {
        info!(
            session = %session_id.0,
            target = target_message_id,
            "executing rewind"
        );

        // 1. Find the target checkpoint to determine truncation point.
        let checkpoints = container
            .chat_checkpoint_manager
            .checkpoint_store()
            .list_by_session(session_id)
            .await
            .map_err(|e| RewindError::CheckpointError(e.to_string()))?;

        // Find the checkpoint whose scope contains the target message ID.
        // If not found, fall back to finding the snapshot in file history and
        // using the transcript index directly.
        let target_checkpoint = checkpoints
            .iter()
            .find(|cp| cp.journal_scope_id.contains(target_message_id));

        // 2. Determine how many messages to keep.
        let keep_count = if let Some(cp) = target_checkpoint {
            // Keep messages up to and including the user message at this checkpoint.
            cp.message_count_before as usize + 1
        } else {
            // Fallback: find the message in the display transcript.
            let transcript = container
                .session_manager
                .read_display_transcript(session_id)
                .await
                .map_err(|e| RewindError::TruncateFailed(e.to_string()))?;

            let idx = transcript
                .iter()
                .position(|m| m.message_id == target_message_id)
                .ok_or_else(|| RewindError::SnapshotNotFound(target_message_id.to_string()))?;

            // Keep everything up to and including this message.
            idx + 1
        };

        // 3. Phase 1: Truncate both transcripts concurrently.
        let (display_result, context_result) = tokio::join!(
            container
                .session_manager
                .display_transcript_store()
                .truncate(session_id, keep_count),
            container
                .session_manager
                .transcript_store()
                .truncate(session_id, keep_count),
        );

        let display_removed =
            display_result.map_err(|e| RewindError::TruncateFailed(e.to_string()))?;
        let context_removed =
            context_result.map_err(|e| RewindError::TruncateFailed(e.to_string()))?;

        let messages_removed = display_removed.max(context_removed);

        info!(
            session = %session_id.0,
            keep_count,
            display_removed,
            context_removed,
            "transcript truncation complete"
        );

        // 4. Phase 2: Restore files from backups.
        //    Take the manager out of the map so we can move it into
        //    spawn_blocking (filesystem I/O should not block tokio).
        let file_report = {
            let mut mgr = container
                .file_history_managers
                .write()
                .await
                .remove(session_id)
                .ok_or_else(|| RewindError::NoHistory(session_id.0.clone()))?;

            let target = target_message_id.to_string();
            let (mgr, result) = tokio::task::spawn_blocking(move || {
                let report = mgr.rewind_to(&target);
                (mgr, report)
            })
            .await
            .map_err(|e| RewindError::FileError(format!("blocking task failed: {e}")))?;

            // Put the manager back.
            container
                .file_history_managers
                .write()
                .await
                .insert(session_id.clone(), mgr);

            result.map_err(|e| RewindError::FileError(e.to_string()))?
        };

        info!(
            session = %session_id.0,
            restored = file_report.restored.len(),
            deleted = file_report.deleted.len(),
            conflicts = file_report.conflicts.len(),
            "file restoration complete"
        );

        // 5. Phase 3: Invalidate checkpoints after the target.
        let target_turn = target_checkpoint.map_or(0, |cp| cp.turn_number);
        let checkpoints_invalidated = container
            .chat_checkpoint_manager
            .checkpoint_store()
            .invalidate_after(session_id, target_turn)
            .await
            .map_err(|e| RewindError::CheckpointError(e.to_string()))?;

        info!(
            session = %session_id.0,
            checkpoints_invalidated,
            "checkpoint invalidation complete"
        );

        // 6. Log rewind completion. Session metadata (message_count,
        //    token_count) will be recalculated naturally on the next turn
        //    by the chat service's prepare_turn logic.

        Ok(RewindResult {
            target_message_id: target_message_id.to_string(),
            messages_removed,
            checkpoints_invalidated,
            file_report,
        })
    }

    /// Restore files to a specific message boundary without touching transcripts
    /// or checkpoints.
    ///
    /// This is used by the GUI undo flow, where transcript truncation and
    /// checkpoint invalidation are already handled by `chat_undo`. We only
    /// need to perform the file restoration phase.
    ///
    /// If the target snapshot does not exist (e.g. the undone turn had no
    /// file edits), orphaned snapshots are cleaned up and an empty report
    /// is returned.
    pub async fn restore_files_only(
        container: &ServiceContainer,
        session_id: &SessionId,
        target_message_id: &str,
    ) -> Result<RewindReport, RewindError> {
        info!(
            session = %session_id.0,
            target = target_message_id,
            "restoring files only (no transcript/checkpoint changes)"
        );

        let file_report = {
            let mut mgr = container
                .file_history_managers
                .write()
                .await
                .remove(session_id)
                .ok_or_else(|| RewindError::NoHistory(session_id.0.clone()))?;

            let target = target_message_id.to_string();
            let (mgr, result) = tokio::task::spawn_blocking(move || {
                let report = mgr.rewind_to(&target);
                (mgr, report)
            })
            .await
            .map_err(|e| RewindError::FileError(format!("blocking task failed: {e}")))?;

            // Put the manager back.
            container
                .file_history_managers
                .write()
                .await
                .insert(session_id.clone(), mgr);

            match result {
                Ok(report) => report,
                Err(y_journal::JournalError::ScopeNotFound { .. }) => {
                    // Target message had no file edits -- no snapshot exists.
                    // Clean up any orphaned snapshots left behind by previous
                    // undo operations.
                    info!(
                        session = %session_id.0,
                        target = target_message_id,
                        "no snapshot for target; cleaning up orphaned snapshots"
                    );
                    Self::cleanup_orphaned_snapshots(container, session_id).await;
                    RewindReport {
                        restored: Vec::new(),
                        deleted: Vec::new(),
                        conflicts: Vec::new(),
                    }
                }
                Err(e) => return Err(RewindError::FileError(e.to_string())),
            }
        };

        info!(
            session = %session_id.0,
            restored = file_report.restored.len(),
            deleted = file_report.deleted.len(),
            conflicts = file_report.conflicts.len(),
            "file-only restoration complete"
        );

        Ok(file_report)
    }

    /// Remove file history snapshots whose `message_id` no longer appears
    /// in the display transcript.
    ///
    /// Called after undo operations where the transcript has been truncated
    /// but snapshot cleanup was skipped (e.g. the undone turn had no
    /// file-level snapshot).
    pub async fn cleanup_orphaned_snapshots(container: &ServiceContainer, session_id: &SessionId) {
        let transcript = container
            .session_manager
            .read_display_transcript(session_id)
            .await
            .unwrap_or_default();

        let valid_ids: HashSet<String> = transcript
            .iter()
            .filter(|m| m.role == Role::User)
            .map(|m| m.message_id.clone())
            .collect();

        let mut managers = container.file_history_managers.write().await;
        if let Some(mgr) = managers.get_mut(session_id) {
            mgr.retain_snapshots(&valid_ids);
        }
    }

    /// Get or create a [`FileHistoryManager`] for a session.
    ///
    /// Called by the chat service during turn preparation to ensure
    /// a manager exists before file-mutating tools run.
    pub async fn ensure_manager(
        managers: &FileHistoryManagers,
        session_id: &SessionId,
        data_dir: &std::path::Path,
    ) -> Result<(), String> {
        let mut map = managers.write().await;
        if map.contains_key(session_id) {
            return Ok(());
        }

        let mgr = FileHistoryManager::new(&session_id.0, data_dir)
            .map_err(|e| format!("failed to create FileHistoryManager: {e}"))?;
        map.insert(session_id.clone(), mgr);
        Ok(())
    }

    /// Create a snapshot for the given session at a user message boundary.
    ///
    /// Called after persisting the user message in `prepare_turn()`.
    pub async fn make_snapshot(
        managers: &FileHistoryManagers,
        session_id: &SessionId,
        message_id: &str,
    ) {
        let mut map = managers.write().await;
        if let Some(mgr) = map.get_mut(session_id) {
            mgr.make_snapshot(message_id);
        }
    }

    /// Track a file edit before a file-mutating tool call.
    ///
    /// Called from `tool_dispatch` before `FileWrite`, `FileDelete`, etc.
    pub async fn track_edit(
        managers: &FileHistoryManagers,
        session_id: &SessionId,
        file_path: &str,
    ) {
        let mut map = managers.write().await;
        if let Some(mgr) = map.get_mut(session_id) {
            if let Err(e) = mgr.track_edit(file_path) {
                warn!(
                    session = %session_id.0,
                    path = file_path,
                    error = %e,
                    "failed to track file edit"
                );
            }
        }
    }

    /// Clean up file history when a session is deleted/archived.
    pub async fn cleanup_session(managers: &FileHistoryManagers, session_id: &SessionId) {
        let mut map = managers.write().await;
        if let Some(mgr) = map.remove(session_id) {
            mgr.cleanup();
            info!(session = %session_id.0, "file history cleaned up");
        }
    }
}
