//! SQLite-backed `ChatMessageStore` implementation for session history tree.

use sqlx::SqlitePool;

use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionError};
use y_core::types::SessionId;

/// SQLite implementation of [`ChatMessageStore`].
pub struct SqliteChatMessageStore {
    pool: SqlitePool,
}

impl SqliteChatMessageStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ChatMessageStore for SqliteChatMessageStore {
    async fn insert(&self, record: &ChatMessageRecord) -> Result<(), SessionError> {
        let status_str = status_to_str(&record.status);
        sqlx::query(
            "INSERT INTO chat_messages (id, session_id, role, content, status, checkpoint_id, \
             model, input_tokens, output_tokens, cost_usd, context_window, \
             parent_message_id, pruning_group_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(record.session_id.as_str())
        .bind(&record.role)
        .bind(&record.content)
        .bind(status_str)
        .bind(&record.checkpoint_id)
        .bind(&record.model)
        .bind(record.input_tokens)
        .bind(record.output_tokens)
        .bind(record.cost_usd)
        .bind(record.context_window)
        .bind(&record.parent_message_id)
        .bind(&record.pruning_group_id)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to insert chat message: {e}"),
        })?;
        Ok(())
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatMessageRecord>, SessionError> {
        let rows: Vec<ChatMessageRow> = sqlx::query_as(
            "SELECT id, session_id, role, content, status, checkpoint_id, \
             model, input_tokens, output_tokens, cost_usd, context_window, \
             parent_message_id, pruning_group_id, created_at \
             FROM chat_messages WHERE session_id = ? ORDER BY created_at ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to list chat messages: {e}"),
        })?;
        Ok(rows.into_iter().map(ChatMessageRow::into_record).collect())
    }

    async fn list_active(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatMessageRecord>, SessionError> {
        let rows: Vec<ChatMessageRow> = sqlx::query_as(
            "SELECT id, session_id, role, content, status, checkpoint_id, \
             model, input_tokens, output_tokens, cost_usd, context_window, \
             parent_message_id, pruning_group_id, created_at \
             FROM chat_messages WHERE session_id = ? AND status = 'active' \
             ORDER BY created_at ASC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to list active chat messages: {e}"),
        })?;
        Ok(rows.into_iter().map(ChatMessageRow::into_record).collect())
    }

    async fn tombstone_after(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<u32, SessionError> {
        // Find the checkpoint's created_at to tombstone messages after it.
        let result = sqlx::query_scalar::<_, i32>(
            "UPDATE chat_messages SET status = 'tombstone' \
             WHERE session_id = ? AND status = 'active' AND checkpoint_id = ? \
             RETURNING 1",
        )
        .bind(session_id.as_str())
        .bind(checkpoint_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to tombstone messages: {e}"),
        })?;
        Ok(u32::try_from(result.len()).unwrap_or(0))
    }

    async fn restore_tombstoned(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<u32, SessionError> {
        let result = sqlx::query_scalar::<_, i32>(
            "UPDATE chat_messages SET status = 'active' \
             WHERE session_id = ? AND status = 'tombstone' AND checkpoint_id = ? \
             RETURNING 1",
        )
        .bind(session_id.as_str())
        .bind(checkpoint_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to restore tombstoned messages: {e}"),
        })?;
        Ok(u32::try_from(result.len()).unwrap_or(0))
    }

    async fn swap_branches(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<(u32, u32), SessionError> {
        // Count active and tombstoned messages for this checkpoint before swapping.
        // Pruned messages are excluded from swap.
        let active_count: i32 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_messages \
             WHERE session_id = ? AND checkpoint_id = ? AND status = 'active'",
        )
        .bind(session_id.as_str())
        .bind(checkpoint_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to count active messages: {e}"),
        })?;

        let tombstone_count: i32 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_messages \
             WHERE session_id = ? AND checkpoint_id = ? AND status = 'tombstone'",
        )
        .bind(session_id.as_str())
        .bind(checkpoint_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to count tombstoned messages: {e}"),
        })?;

        // Atomic swap: flip active <-> tombstone in a single UPDATE.
        // Pruned messages are NOT affected by branch swap.
        sqlx::query(
            "UPDATE chat_messages SET status = CASE \
                 WHEN status = 'active' THEN 'tombstone' \
                 WHEN status = 'tombstone' THEN 'active' \
             END \
             WHERE session_id = ? AND checkpoint_id = ? \
             AND status IN ('active', 'tombstone')",
        )
        .bind(session_id.as_str())
        .bind(checkpoint_id)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed atomic branch swap: {e}"),
        })?;

        // active_count messages were tombstoned, tombstone_count messages were restored.
        Ok((
            u32::try_from(active_count).unwrap_or(0),
            u32::try_from(tombstone_count).unwrap_or(0),
        ))
    }

    async fn set_status(
        &self,
        session_id: &SessionId,
        message_id: &str,
        status: ChatMessageStatus,
    ) -> Result<(), SessionError> {
        let status_str = status_to_str(&status);
        sqlx::query(
            "UPDATE chat_messages SET status = ? \
             WHERE session_id = ? AND id = ?",
        )
        .bind(status_str)
        .bind(session_id.as_str())
        .bind(message_id)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to set message status: {e}"),
        })?;
        Ok(())
    }

    async fn set_status_batch(
        &self,
        session_id: &SessionId,
        message_ids: &[String],
        status: ChatMessageStatus,
    ) -> Result<u32, SessionError> {
        if message_ids.is_empty() {
            return Ok(0);
        }
        let status_str = status_to_str(&status);
        // Build placeholder list for IN clause.
        let placeholders: Vec<&str> = message_ids.iter().map(|_| "?").collect();
        let sql = format!(
            "UPDATE chat_messages SET status = ? \
             WHERE session_id = ? AND id IN ({}) RETURNING 1",
            placeholders.join(", ")
        );
        let mut query = sqlx::query_scalar::<_, i32>(&sql)
            .bind(status_str)
            .bind(session_id.as_str());
        for id in message_ids {
            query = query.bind(id);
        }
        let result = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SessionError::StorageError {
                message: format!("failed to batch set message status: {e}"),
            })?;
        Ok(u32::try_from(result.len()).unwrap_or(0))
    }

    async fn restore_pruned(&self, session_id: &SessionId) -> Result<u32, SessionError> {
        let result = sqlx::query_scalar::<_, i32>(
            "UPDATE chat_messages SET status = 'active' \
             WHERE session_id = ? AND status = 'pruned' \
             RETURNING 1",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: format!("failed to restore pruned messages: {e}"),
        })?;
        Ok(u32::try_from(result.len()).unwrap_or(0))
    }
}

// ---------------------------------------------------------------------------
// Internal row type
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct ChatMessageRow {
    id: String,
    session_id: String,
    role: String,
    content: String,
    status: String,
    checkpoint_id: Option<String>,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cost_usd: Option<f64>,
    context_window: Option<i64>,
    parent_message_id: Option<String>,
    pruning_group_id: Option<String>,
    created_at: String,
}

/// Convert a `ChatMessageStatus` to its SQL string representation.
fn status_to_str(status: &ChatMessageStatus) -> &'static str {
    match status {
        ChatMessageStatus::Active => "active",
        ChatMessageStatus::Tombstone => "tombstone",
        ChatMessageStatus::Pruned => "pruned",
    }
}

impl ChatMessageRow {
    fn into_record(self) -> ChatMessageRecord {
        let status = match self.status.as_str() {
            "tombstone" => ChatMessageStatus::Tombstone,
            "pruned" => ChatMessageStatus::Pruned,
            _ => ChatMessageStatus::Active,
        };
        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));
        ChatMessageRecord {
            id: self.id,
            session_id: SessionId(self.session_id),
            role: self.role,
            content: self.content,
            status,
            checkpoint_id: self.checkpoint_id,
            model: self.model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cost_usd: self.cost_usd,
            context_window: self.context_window,
            parent_message_id: self.parent_message_id,
            pruning_group_id: self.pruning_group_id,
            created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> SqliteChatMessageStore {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_checkpoints (
                checkpoint_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_number INTEGER NOT NULL,
                message_count_before INTEGER NOT NULL,
                journal_scope_id TEXT NOT NULL,
                invalidated INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chat_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                checkpoint_id TEXT REFERENCES chat_checkpoints(checkpoint_id),
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cost_usd REAL,
                context_window INTEGER,
                parent_message_id TEXT,
                pruning_group_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        SqliteChatMessageStore::new(pool)
    }

    fn make_record(
        id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        cp: Option<&str>,
    ) -> ChatMessageRecord {
        ChatMessageRecord {
            id: id.to_string(),
            session_id: SessionId(session_id.to_string()),
            role: role.to_string(),
            content: content.to_string(),
            status: ChatMessageStatus::Active,
            checkpoint_id: cp.map(|s| s.to_string()),
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            context_window: None,
            parent_message_id: None,
            pruning_group_id: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_insert_and_list() {
        let store = setup().await;
        let sid = "s1";
        store
            .insert(&make_record("m1", sid, "user", "hello", None))
            .await
            .unwrap();
        store
            .insert(&make_record("m2", sid, "assistant", "hi", None))
            .await
            .unwrap();

        let all = store
            .list_by_session(&SessionId(sid.to_string()))
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].content, "hello");
        assert_eq!(all[1].content, "hi");
    }

    #[tokio::test]
    async fn test_list_active_filters_tombstoned() {
        let store = setup().await;
        let sid = "s1";

        // Insert a checkpoint first.
        sqlx::query(
            "INSERT INTO chat_checkpoints (checkpoint_id, session_id, turn_number, message_count_before, journal_scope_id) \
             VALUES ('cp1', 's1', 1, 0, 'scope1')",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        store
            .insert(&make_record("m1", sid, "user", "hello", Some("cp1")))
            .await
            .unwrap();
        store
            .insert(&make_record("m2", sid, "assistant", "hi", Some("cp1")))
            .await
            .unwrap();

        // Tombstone messages for cp1.
        let count = store
            .tombstone_after(&SessionId(sid.to_string()), "cp1")
            .await
            .unwrap();
        assert_eq!(count, 2);

        // list_active should return empty.
        let active = store
            .list_active(&SessionId(sid.to_string()))
            .await
            .unwrap();
        assert_eq!(active.len(), 0);

        // list_by_session should return all (tombstoned).
        let all = store
            .list_by_session(&SessionId(sid.to_string()))
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, ChatMessageStatus::Tombstone);
    }

    #[tokio::test]
    async fn test_restore_tombstoned() {
        let store = setup().await;
        let sid = "s1";

        sqlx::query(
            "INSERT INTO chat_checkpoints (checkpoint_id, session_id, turn_number, message_count_before, journal_scope_id) \
             VALUES ('cp1', 's1', 1, 0, 'scope1')",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        store
            .insert(&make_record("m1", sid, "user", "hello", Some("cp1")))
            .await
            .unwrap();
        store
            .tombstone_after(&SessionId(sid.to_string()), "cp1")
            .await
            .unwrap();

        // Restore.
        let restored = store
            .restore_tombstoned(&SessionId(sid.to_string()), "cp1")
            .await
            .unwrap();
        assert_eq!(restored, 1);

        let active = store
            .list_active(&SessionId(sid.to_string()))
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, ChatMessageStatus::Active);
    }

    #[tokio::test]
    async fn test_swap_branches() {
        let store = setup().await;
        let sid = "s1";

        sqlx::query(
            "INSERT INTO chat_checkpoints (checkpoint_id, session_id, turn_number, message_count_before, journal_scope_id) \
             VALUES ('cp1', 's1', 1, 0, 'scope1')",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        // Branch A: active messages.
        store
            .insert(&make_record(
                "m1",
                sid,
                "user",
                "branch-a-user",
                Some("cp1"),
            ))
            .await
            .unwrap();
        store
            .insert(&make_record(
                "m2",
                sid,
                "assistant",
                "branch-a-asst",
                Some("cp1"),
            ))
            .await
            .unwrap();

        // Tombstone branch A (simulating undo).
        store
            .tombstone_after(&SessionId(sid.to_string()), "cp1")
            .await
            .unwrap();

        // Insert branch B messages (new active branch).
        store
            .insert(&make_record(
                "m3",
                sid,
                "user",
                "branch-b-user",
                Some("cp1"),
            ))
            .await
            .unwrap();
        store
            .insert(&make_record(
                "m4",
                sid,
                "assistant",
                "branch-b-asst",
                Some("cp1"),
            ))
            .await
            .unwrap();

        // Now swap: branch B -> tombstone, branch A -> active.
        let (t, r) = store
            .swap_branches(&SessionId(sid.to_string()), "cp1")
            .await
            .unwrap();
        assert_eq!(t, 2); // branch B tombstoned
        assert_eq!(r, 2); // branch A restored

        let active = store
            .list_active(&SessionId(sid.to_string()))
            .await
            .unwrap();
        assert_eq!(active.len(), 2);
        assert!(active.iter().any(|m| m.content == "branch-a-user"));
        assert!(active.iter().any(|m| m.content == "branch-a-asst"));
    }
}
