//! SQLite-backed `ChatCheckpointStore` implementation.

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::instrument;

use y_core::session::{ChatCheckpoint, ChatCheckpointStore, SessionError};
use y_core::types::SessionId;

/// SQLite-backed chat checkpoint storage.
///
/// Stores turn-level checkpoint records that link a session's message count
/// to a File Journal scope, enabling conversation + file rollback.
#[derive(Debug, Clone)]
pub struct SqliteChatCheckpointStore {
    pool: SqlitePool,
}

impl SqliteChatCheckpointStore {
    /// Create a new chat checkpoint store backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatCheckpointStore for SqliteChatCheckpointStore {
    #[instrument(skip(self, checkpoint), fields(
        checkpoint_id = %checkpoint.checkpoint_id,
        session_id = %checkpoint.session_id,
        turn = checkpoint.turn_number,
    ))]
    async fn save(&self, checkpoint: &ChatCheckpoint) -> Result<(), SessionError> {
        // The table has UNIQUE(session_id, turn_number).  After a resend the
        // old checkpoint for the same turn is invalidated but still occupies
        // the unique slot.  Delete it first so the new INSERT succeeds.
        sqlx::query(
            r"DELETE FROM chat_checkpoints
              WHERE session_id = ?1 AND turn_number = ?2 AND invalidated = 1",
        )
        .bind(checkpoint.session_id.as_str())
        .bind(i64::from(checkpoint.turn_number))
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("delete invalidated checkpoint: {e}"),
        })?;

        sqlx::query(
            r"INSERT INTO chat_checkpoints
              (checkpoint_id, session_id, turn_number, message_count_before,
               journal_scope_id, invalidated, created_at)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
              ON CONFLICT(checkpoint_id) DO UPDATE SET
                  turn_number = excluded.turn_number,
                  message_count_before = excluded.message_count_before,
                  journal_scope_id = excluded.journal_scope_id,
                  invalidated = excluded.invalidated",
        )
        .bind(&checkpoint.checkpoint_id)
        .bind(checkpoint.session_id.as_str())
        .bind(i64::from(checkpoint.turn_number))
        .bind(i64::from(checkpoint.message_count_before))
        .bind(&checkpoint.journal_scope_id)
        .bind(i32::from(checkpoint.invalidated))
        .bind(checkpoint.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("save chat checkpoint: {e}"),
        })?;

        Ok(())
    }

    #[instrument(skip(self), fields(checkpoint_id = %checkpoint_id))]
    async fn load(&self, checkpoint_id: &str) -> Result<ChatCheckpoint, SessionError> {
        let row: ChatCheckpointRow = sqlx::query_as(
            r"SELECT checkpoint_id, session_id, turn_number, message_count_before,
                     journal_scope_id, invalidated, created_at
              FROM chat_checkpoints
              WHERE checkpoint_id = ?1",
        )
        .bind(checkpoint_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("load chat checkpoint: {e}"),
        })?
        .ok_or_else(|| SessionError::NotFound {
            id: checkpoint_id.to_string(),
        })?;

        Ok(row.into_checkpoint())
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatCheckpoint>, SessionError> {
        let rows: Vec<ChatCheckpointRow> = sqlx::query_as(
            r"SELECT checkpoint_id, session_id, turn_number, message_count_before,
                     journal_scope_id, invalidated, created_at
              FROM chat_checkpoints
              WHERE session_id = ?1
              ORDER BY turn_number DESC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("list chat checkpoints: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .map(ChatCheckpointRow::into_checkpoint)
            .collect())
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn latest(&self, session_id: &SessionId) -> Result<Option<ChatCheckpoint>, SessionError> {
        let row: Option<ChatCheckpointRow> = sqlx::query_as(
            r"SELECT checkpoint_id, session_id, turn_number, message_count_before,
                     journal_scope_id, invalidated, created_at
              FROM chat_checkpoints
              WHERE session_id = ?1 AND invalidated = 0
              ORDER BY turn_number DESC
              LIMIT 1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("latest chat checkpoint: {e}"),
        })?;

        Ok(row.map(ChatCheckpointRow::into_checkpoint))
    }

    #[instrument(skip(self), fields(session_id = %session_id, after_turn = turn_number))]
    async fn invalidate_after(
        &self,
        session_id: &SessionId,
        turn_number: u32,
    ) -> Result<u32, SessionError> {
        let result = sqlx::query(
            r"UPDATE chat_checkpoints
              SET invalidated = 1
              WHERE session_id = ?1 AND turn_number > ?2 AND invalidated = 0",
        )
        .bind(session_id.as_str())
        .bind(i64::from(turn_number))
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("invalidate chat checkpoints: {e}"),
        })?;

        Ok(result.rows_affected() as u32)
    }
}

// ---------------------------------------------------------------------------
// Internal row mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ChatCheckpointRow {
    checkpoint_id: String,
    session_id: String,
    turn_number: i64,
    message_count_before: i64,
    journal_scope_id: String,
    invalidated: i32,
    created_at: String,
}

impl ChatCheckpointRow {
    fn into_checkpoint(self) -> ChatCheckpoint {
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));

        ChatCheckpoint {
            checkpoint_id: self.checkpoint_id,
            session_id: SessionId::from_string(self.session_id),
            turn_number: self.turn_number as u32,
            message_count_before: self.message_count_before as u32,
            journal_scope_id: self.journal_scope_id,
            invalidated: self.invalidated != 0,
            created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::migration::run_embedded_migrations;
    use crate::pool::create_pool;

    async fn setup() -> SqliteChatCheckpointStore {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        SqliteChatCheckpointStore::new(pool)
    }

    fn make_checkpoint(session_id: &str, turn: u32, msg_count: u32) -> ChatCheckpoint {
        ChatCheckpoint {
            checkpoint_id: uuid::Uuid::new_v4().to_string(),
            session_id: SessionId::from_string(session_id),
            turn_number: turn,
            message_count_before: msg_count,
            journal_scope_id: format!("scope-turn-{turn}"),
            invalidated: false,
            created_at: chrono::Utc::now(),
        }
    }

    // T-CP-06: Save + load round-trip.
    #[tokio::test]
    async fn test_save_and_load() {
        let store = setup().await;
        let cp = make_checkpoint("sess-1", 1, 0);
        let cp_id = cp.checkpoint_id.clone();

        store.save(&cp).await.unwrap();

        let loaded = store.load(&cp_id).await.unwrap();
        assert_eq!(loaded.checkpoint_id, cp_id);
        assert_eq!(loaded.session_id.as_str(), "sess-1");
        assert_eq!(loaded.turn_number, 1);
        assert_eq!(loaded.message_count_before, 0);
        assert!(!loaded.invalidated);
    }

    // T-CP-07: list_by_session returns checkpoints in descending turn order.
    #[tokio::test]
    async fn test_list_by_session_descending() {
        let store = setup().await;
        let sid = SessionId::from_string("sess-2");

        for turn in 1..=5 {
            let cp = make_checkpoint("sess-2", turn, (turn - 1) * 3);
            store.save(&cp).await.unwrap();
        }

        let list = store.list_by_session(&sid).await.unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].turn_number, 5); // most recent first
        assert_eq!(list[4].turn_number, 1); // oldest last
    }

    // T-CP-08: invalidate_after marks correct checkpoints.
    #[tokio::test]
    async fn test_invalidate_after() {
        let store = setup().await;
        let sid = SessionId::from_string("sess-3");

        for turn in 1..=5 {
            let cp = make_checkpoint("sess-3", turn, (turn - 1) * 2);
            store.save(&cp).await.unwrap();
        }

        // Invalidate everything after turn 2.
        let count = store.invalidate_after(&sid, 2).await.unwrap();
        assert_eq!(count, 3); // turns 3, 4, 5

        // Verify: turns 1-2 valid, 3-5 invalidated.
        let list = store.list_by_session(&sid).await.unwrap();
        for cp in &list {
            if cp.turn_number <= 2 {
                assert!(!cp.invalidated, "turn {} should be valid", cp.turn_number);
            } else {
                assert!(
                    cp.invalidated,
                    "turn {} should be invalidated",
                    cp.turn_number
                );
            }
        }
    }

    // T-CP-09: latest returns most recent non-invalidated checkpoint.
    #[tokio::test]
    async fn test_latest_non_invalidated() {
        let store = setup().await;
        let sid = SessionId::from_string("sess-4");

        for turn in 1..=3 {
            let cp = make_checkpoint("sess-4", turn, (turn - 1) * 2);
            store.save(&cp).await.unwrap();
        }

        // Invalidate turn 3.
        store.invalidate_after(&sid, 2).await.unwrap();

        let latest = store.latest(&sid).await.unwrap().unwrap();
        assert_eq!(latest.turn_number, 2);
    }

    // Latest returns None when no checkpoints exist.
    #[tokio::test]
    async fn test_latest_none_when_empty() {
        let store = setup().await;
        let sid = SessionId::from_string("no-checkpoints");
        let result = store.latest(&sid).await.unwrap();
        assert!(result.is_none());
    }

    // Load returns error for nonexistent checkpoint.
    #[tokio::test]
    async fn test_load_not_found() {
        let store = setup().await;
        let result = store.load("nonexistent").await;
        assert!(result.is_err());
    }
}
