//! Chat checkpoint manager — coordinates checkpoint creation and rollback.
//!
//! Links session transcripts to File Journal scopes so that a single "undo"
//! operation can revert both conversation history and filesystem changes.

use std::sync::Arc;

use tracing::{info, instrument, warn};

use y_core::session::{
    ChatCheckpoint, ChatCheckpointStore, DisplayTranscriptStore, SessionError, SessionStore,
    TranscriptStore,
};
use y_core::types::SessionId;

/// Result of a rollback operation.
#[derive(Debug, Clone)]
pub struct RollbackResult {
    /// Number of messages removed from the transcript.
    pub messages_removed: usize,
    /// File Journal scopes that were rolled back.
    pub scopes_rolled_back: Vec<String>,
    /// Turn number rolled back to.
    pub rolled_back_to_turn: u32,
    /// Number of checkpoints invalidated.
    pub checkpoints_invalidated: u32,
}

/// Manages chat-level checkpoints for turn-level rollback.
///
/// Coordinates between `TranscriptStore` (conversation), `ChatCheckpointStore`
/// (checkpoint records), and File Journal scopes (filesystem rollback).
pub struct ChatCheckpointManager {
    transcript_store: Arc<dyn TranscriptStore>,
    display_transcript_store: Arc<dyn DisplayTranscriptStore>,
    checkpoint_store: Arc<dyn ChatCheckpointStore>,
    session_store: Arc<dyn SessionStore>,
}

impl ChatCheckpointManager {
    /// Create a new checkpoint manager.
    pub fn new(
        transcript_store: Arc<dyn TranscriptStore>,
        display_transcript_store: Arc<dyn DisplayTranscriptStore>,
        checkpoint_store: Arc<dyn ChatCheckpointStore>,
        session_store: Arc<dyn SessionStore>,
    ) -> Self {
        Self {
            transcript_store,
            display_transcript_store,
            checkpoint_store,
            session_store,
        }
    }

    /// Create a checkpoint after a completed agent turn.
    ///
    /// - `session_id`: The session this turn belongs to.
    /// - `turn_number`: 1-indexed turn counter.
    /// - `message_count_before`: Number of messages in transcript before the turn started.
    /// - `journal_scope_id`: File Journal scope ID for this turn's file operations.
    #[instrument(skip(self), fields(
        session_id = %session_id,
        turn = turn_number,
        msg_before = message_count_before,
    ))]
    pub async fn create_checkpoint(
        &self,
        session_id: &SessionId,
        turn_number: u32,
        message_count_before: u32,
        journal_scope_id: String,
    ) -> Result<ChatCheckpoint, SessionError> {
        let checkpoint = ChatCheckpoint {
            checkpoint_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.clone(),
            turn_number,
            message_count_before,
            journal_scope_id,
            invalidated: false,
            created_at: chrono::Utc::now(),
        };

        self.checkpoint_store.save(&checkpoint).await?;

        info!(
            checkpoint_id = %checkpoint.checkpoint_id,
            "chat checkpoint created"
        );

        Ok(checkpoint)
    }

    /// Rollback to the latest non-invalidated checkpoint (undo last turn).
    ///
    /// Truncates the transcript and invalidates the checkpoint.
    /// File Journal rollback is delegated back to the caller via the
    /// `scopes_rolled_back` field in the result (since y-journal is a
    /// separate crate and we use trait boundaries).
    #[instrument(skip(self), fields(session_id = %session_id))]
    pub async fn rollback_last(
        &self,
        session_id: &SessionId,
    ) -> Result<RollbackResult, SessionError> {
        let checkpoint = self
            .checkpoint_store
            .latest(session_id)
            .await?
            .ok_or_else(|| SessionError::Other {
                message: "no checkpoints available for rollback".to_string(),
            })?;

        self.rollback_to(session_id, &checkpoint.checkpoint_id)
            .await
    }

    /// Rollback to a specific checkpoint.
    ///
    /// Truncates the transcript to `message_count_before`, invalidates
    /// all checkpoints from the target turn onward, and returns the
    /// scope IDs that need file-level rollback.
    #[instrument(skip(self), fields(session_id = %session_id, checkpoint_id = %checkpoint_id))]
    pub async fn rollback_to(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<RollbackResult, SessionError> {
        // Load the target checkpoint.
        let target = self.checkpoint_store.load(checkpoint_id).await?;

        if target.session_id != *session_id {
            return Err(SessionError::Other {
                message: format!(
                    "checkpoint {} belongs to session {}, not {}",
                    target.checkpoint_id, target.session_id, session_id
                ),
            });
        }

        if target.invalidated {
            return Err(SessionError::Other {
                message: format!("checkpoint {} is already invalidated", target.checkpoint_id),
            });
        }

        // Collect all scopes from target turn onward for file rollback.
        let all_checkpoints = self.checkpoint_store.list_by_session(session_id).await?;
        let scopes_to_rollback: Vec<String> = all_checkpoints
            .iter()
            .filter(|cp| cp.turn_number >= target.turn_number && !cp.invalidated)
            .map(|cp| cp.journal_scope_id.clone())
            .collect();

        // Truncate display transcript to the pre-turn state.
        if let Err(e) = self
            .display_transcript_store
            .truncate(session_id, target.message_count_before as usize)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "failed to truncate display transcript during rollback"
            );
        }

        // Truncate context transcript to the pre-turn state.
        let messages_removed = self
            .transcript_store
            .truncate(session_id, target.message_count_before as usize)
            .await?;

        // Update session metadata with new message count.
        if let Err(e) = self
            .session_store
            .update_metadata(session_id, None, 0, target.message_count_before)
            .await
        {
            warn!(error = %e, "failed to update session metadata after rollback");
        }

        // Invalidate target checkpoint and all newer ones.
        let invalidated = self
            .checkpoint_store
            .invalidate_after(session_id, target.turn_number.saturating_sub(1))
            .await?;

        info!(
            messages_removed,
            invalidated,
            scopes = scopes_to_rollback.len(),
            "chat rollback completed"
        );

        Ok(RollbackResult {
            messages_removed,
            scopes_rolled_back: scopes_to_rollback,
            rolled_back_to_turn: target.turn_number.saturating_sub(1),
            checkpoints_invalidated: invalidated,
        })
    }

    /// Get a reference to the underlying checkpoint store.
    pub fn checkpoint_store(&self) -> &dyn ChatCheckpointStore {
        &*self.checkpoint_store
    }

    /// List available (non-invalidated) checkpoints for a session.
    pub async fn list_checkpoints(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatCheckpoint>, SessionError> {
        let all = self.checkpoint_store.list_by_session(session_id).await?;
        Ok(all.into_iter().filter(|cp| !cp.invalidated).collect())
    }

    /// Get the current turn number for a session (based on checkpoint count).
    pub async fn current_turn(&self, session_id: &SessionId) -> Result<u32, SessionError> {
        let latest = self.checkpoint_store.latest(session_id).await?;
        Ok(latest.map_or(0, |cp| cp.turn_number))
    }
}

impl std::fmt::Debug for ChatCheckpointManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatCheckpointManager")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_test_utils::fixtures::{make_assistant_message, make_user_message};
    use y_test_utils::mock_storage::{
        MockDisplayTranscriptStore, MockSessionStore, MockTranscriptStore,
    };

    /// In-memory `ChatCheckpointStore` for tests.
    #[derive(Debug, Default)]
    struct MockChatCheckpointStore {
        data: std::sync::RwLock<Vec<ChatCheckpoint>>,
    }

    #[async_trait::async_trait]
    impl ChatCheckpointStore for MockChatCheckpointStore {
        async fn save(&self, checkpoint: &ChatCheckpoint) -> Result<(), SessionError> {
            let mut data = self.data.write().unwrap();
            // Upsert by checkpoint_id.
            data.retain(|cp| cp.checkpoint_id != checkpoint.checkpoint_id);
            data.push(checkpoint.clone());
            Ok(())
        }

        async fn load(&self, checkpoint_id: &str) -> Result<ChatCheckpoint, SessionError> {
            let data = self.data.read().unwrap();
            data.iter()
                .find(|cp| cp.checkpoint_id == checkpoint_id)
                .cloned()
                .ok_or(SessionError::NotFound {
                    id: checkpoint_id.to_string(),
                })
        }

        async fn list_by_session(
            &self,
            session_id: &SessionId,
        ) -> Result<Vec<ChatCheckpoint>, SessionError> {
            let data = self.data.read().unwrap();
            let mut result: Vec<_> = data
                .iter()
                .filter(|cp| cp.session_id.as_str() == session_id.as_str())
                .cloned()
                .collect();
            result.sort_by(|a, b| b.turn_number.cmp(&a.turn_number));
            Ok(result)
        }

        async fn latest(
            &self,
            session_id: &SessionId,
        ) -> Result<Option<ChatCheckpoint>, SessionError> {
            let data = self.data.read().unwrap();
            let latest = data
                .iter()
                .filter(|cp| cp.session_id.as_str() == session_id.as_str() && !cp.invalidated)
                .max_by_key(|cp| cp.turn_number)
                .cloned();
            Ok(latest)
        }

        async fn invalidate_after(
            &self,
            session_id: &SessionId,
            turn_number: u32,
        ) -> Result<u32, SessionError> {
            let mut data = self.data.write().unwrap();
            let mut count = 0;
            for cp in data.iter_mut() {
                if cp.session_id.as_str() == session_id.as_str()
                    && cp.turn_number > turn_number
                    && !cp.invalidated
                {
                    cp.invalidated = true;
                    count += 1;
                }
            }
            Ok(count)
        }
    }

    async fn setup() -> (ChatCheckpointManager, SessionId) {
        let session_store = Arc::new(MockSessionStore::new());
        let transcript_store = Arc::new(MockTranscriptStore::new());
        let display_transcript_store = Arc::new(MockDisplayTranscriptStore::new());
        let checkpoint_store = Arc::new(MockChatCheckpointStore::default());

        // Create a session.
        let session = session_store
            .create(y_core::session::CreateSessionOptions {
                parent_id: None,
                session_type: y_core::session::SessionType::Main,
                agent_id: None,
                title: Some("test".into()),
            })
            .await
            .unwrap();

        let mgr = ChatCheckpointManager::new(
            transcript_store,
            display_transcript_store,
            checkpoint_store,
            session_store,
        );
        (mgr, session.id)
    }

    // T-CP-10: end_turn creates checkpoint with correct message_count_before.
    #[tokio::test]
    async fn test_create_checkpoint() {
        let (mgr, sid) = setup().await;

        let cp = mgr
            .create_checkpoint(&sid, 1, 0, "scope-turn-1".into())
            .await
            .unwrap();

        assert_eq!(cp.turn_number, 1);
        assert_eq!(cp.message_count_before, 0);
        assert_eq!(cp.journal_scope_id, "scope-turn-1");
        assert!(!cp.invalidated);
    }

    // T-CP-11: rollback_last truncates transcript.
    #[tokio::test]
    async fn test_rollback_last() {
        let (mgr, sid) = setup().await;

        // Simulate turn 1: user + assistant messages.
        mgr.transcript_store
            .append(&sid, &make_user_message("hello"))
            .await
            .unwrap();
        mgr.transcript_store
            .append(&sid, &make_assistant_message("hi"))
            .await
            .unwrap();

        mgr.create_checkpoint(&sid, 1, 0, "scope-1".into())
            .await
            .unwrap();

        // Simulate turn 2: user + assistant messages.
        mgr.transcript_store
            .append(&sid, &make_user_message("more"))
            .await
            .unwrap();
        mgr.transcript_store
            .append(&sid, &make_assistant_message("sure"))
            .await
            .unwrap();

        mgr.create_checkpoint(&sid, 2, 2, "scope-2".into())
            .await
            .unwrap();

        // Rollback last turn.
        let result = mgr.rollback_last(&sid).await.unwrap();

        assert_eq!(result.messages_removed, 2);
        assert_eq!(result.rolled_back_to_turn, 1);
        assert!(result.scopes_rolled_back.contains(&"scope-2".to_string()));

        // Transcript should have only turn 1 messages.
        let msgs = mgr.transcript_store.read_all(&sid).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "hello");
    }

    // T-CP-12: rollback to specific checkpoint with multi-turn gap.
    #[tokio::test]
    async fn test_rollback_multi_turn() {
        let (mgr, sid) = setup().await;

        // 3 turns, 2 messages each.
        for turn in 1..=3 {
            let before = ((turn - 1) * 2) as u32;
            mgr.transcript_store
                .append(&sid, &make_user_message(&format!("user-{turn}")))
                .await
                .unwrap();
            mgr.transcript_store
                .append(&sid, &make_assistant_message(&format!("asst-{turn}")))
                .await
                .unwrap();
            mgr.create_checkpoint(&sid, turn as u32, before, format!("scope-{turn}"))
                .await
                .unwrap();
        }

        // Rollback to turn 1 (undoing turns 2 and 3).
        let checkpoints = mgr.list_checkpoints(&sid).await.unwrap();
        let turn1_cp = checkpoints.iter().find(|cp| cp.turn_number == 1).unwrap();

        let result = mgr
            .rollback_to(&sid, &turn1_cp.checkpoint_id)
            .await
            .unwrap();

        // Rolling back to turn 1 truncates to message_count_before=0, removing all 6 messages.
        assert_eq!(result.messages_removed, 6);
        assert_eq!(result.rolled_back_to_turn, 0); // before turn 1
        assert_eq!(result.scopes_rolled_back.len(), 3); // scopes 1, 2, 3

        let msgs = mgr.transcript_store.read_all(&sid).await.unwrap();
        assert_eq!(msgs.len(), 0); // all removed since turn 1 starts at msg 0
    }

    // T-CP-13: Rollback with no checkpoints returns error.
    #[tokio::test]
    async fn test_rollback_no_checkpoints() {
        let (mgr, sid) = setup().await;
        let result = mgr.rollback_last(&sid).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rollback_to_rejects_checkpoint_from_other_session() {
        let (mgr, sid) = setup().await;
        let other_session = mgr
            .session_store
            .create(y_core::session::CreateSessionOptions {
                parent_id: None,
                session_type: y_core::session::SessionType::Main,
                agent_id: None,
                title: Some("other".into()),
            })
            .await
            .unwrap();

        let checkpoint = mgr
            .create_checkpoint(&sid, 1, 0, "scope-1".into())
            .await
            .unwrap();

        let result = mgr
            .rollback_to(&other_session.id, &checkpoint.checkpoint_id)
            .await;

        assert!(result.is_err());
    }
}
