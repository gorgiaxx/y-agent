//! SQLite-backed `CheckpointStorage` implementation.

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::instrument;

use y_core::checkpoint::{
    CheckpointError, CheckpointStatus, CheckpointStorage, WorkflowCheckpoint,
};
use y_core::types::{SessionId, WorkflowId};

/// SQLite-backed checkpoint storage.
///
/// Implements committed/pending write separation for cancellation safety.
/// Pending writes are visible only to the current workflow step; committed
/// writes survive crashes.
#[derive(Debug, Clone)]
pub struct SqliteCheckpointStorage {
    pool: SqlitePool,
}

impl SqliteCheckpointStorage {
    /// Create a new checkpoint storage backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CheckpointStorage for SqliteCheckpointStorage {
    #[instrument(skip(self, state), fields(workflow_id = %workflow_id, step = step_number))]
    async fn write_pending(
        &self,
        workflow_id: &WorkflowId,
        session_id: &SessionId,
        step_number: u64,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointError> {
        let wf_id = workflow_id.as_str();
        let sess_id = session_id.as_str();
        let step = i64::try_from(step_number).unwrap_or(i64::MAX);
        let state_json = serde_json::to_string(state).map_err(|e| CheckpointError::Other {
            message: format!("serialize pending state: {e}"),
        })?;

        // Check if a checkpoint already exists for this workflow.
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM orchestrator_checkpoints WHERE workflow_id = ?1 ORDER BY step_number DESC LIMIT 1",
        )
        .bind(wf_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        if let Some((id,)) = existing {
            // Update existing checkpoint: set pending_state and step_number.
            sqlx::query(
                "UPDATE orchestrator_checkpoints SET pending_state = ?1, step_number = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?3",
            )
            .bind(&state_json)
            .bind(step)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(|e| CheckpointError::StorageError {
                message: e.to_string(),
            })?;
        } else {
            // Insert a new checkpoint with pending_state.
            let id = uuid::Uuid::new_v4().to_string();
            let committed = serde_json::to_string(&serde_json::json!({})).unwrap();
            sqlx::query(
                r"INSERT INTO orchestrator_checkpoints (id, workflow_id, session_id, step_number, status, committed_state, pending_state, versions_seen)
                  VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, '{}')",
            )
            .bind(&id)
            .bind(wf_id)
            .bind(sess_id)
            .bind(step)
            .bind(&committed)
            .bind(&state_json)
            .execute(&self.pool)
            .await
            .map_err(|e| CheckpointError::StorageError {
                message: e.to_string(),
            })?;
        }

        Ok(())
    }

    #[instrument(skip(self), fields(workflow_id = %workflow_id, step = step_number))]
    async fn commit(
        &self,
        workflow_id: &WorkflowId,
        step_number: u64,
    ) -> Result<(), CheckpointError> {
        let wf_id = workflow_id.as_str();
        let step = i64::try_from(step_number).unwrap_or(i64::MAX);

        // Move pending_state to committed_state and clear pending.
        let result = sqlx::query(
            r"UPDATE orchestrator_checkpoints
              SET committed_state = pending_state,
                  pending_state = NULL,
                  step_number = ?1,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE workflow_id = ?2 AND pending_state IS NOT NULL",
        )
        .bind(step)
        .bind(wf_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(CheckpointError::NotFound {
                workflow_id: wf_id.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(workflow_id = %workflow_id))]
    async fn read_committed(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowCheckpoint>, CheckpointError> {
        let wf_id = workflow_id.as_str();

        let row: Option<CheckpointRow> = sqlx::query_as(
            r"SELECT id, workflow_id, session_id, step_number, status,
                     committed_state, pending_state, interrupt_data,
                     versions_seen, created_at, updated_at
              FROM orchestrator_checkpoints
              WHERE workflow_id = ?1
              ORDER BY step_number DESC
              LIMIT 1",
        )
        .bind(wf_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        match row {
            Some(r) => Ok(Some(r.into_checkpoint()?)),
            None => Ok(None),
        }
    }

    #[instrument(skip(self, interrupt_data), fields(workflow_id = %workflow_id))]
    async fn set_interrupted(
        &self,
        workflow_id: &WorkflowId,
        interrupt_data: serde_json::Value,
    ) -> Result<(), CheckpointError> {
        let wf_id = workflow_id.as_str();
        let data = serde_json::to_string(&interrupt_data).map_err(|e| CheckpointError::Other {
            message: format!("serialize interrupt_data: {e}"),
        })?;

        let result = sqlx::query(
            r"UPDATE orchestrator_checkpoints
              SET status = 'interrupted', interrupt_data = ?1,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE workflow_id = ?2",
        )
        .bind(&data)
        .bind(wf_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(CheckpointError::NotFound {
                workflow_id: wf_id.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(workflow_id = %workflow_id))]
    async fn set_completed(&self, workflow_id: &WorkflowId) -> Result<(), CheckpointError> {
        let wf_id = workflow_id.as_str();

        let result = sqlx::query(
            r"UPDATE orchestrator_checkpoints
              SET status = 'completed',
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE workflow_id = ?1",
        )
        .bind(wf_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(CheckpointError::NotFound {
                workflow_id: wf_id.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(workflow_id = %workflow_id))]
    async fn set_failed(
        &self,
        workflow_id: &WorkflowId,
        error: &str,
    ) -> Result<(), CheckpointError> {
        let wf_id = workflow_id.as_str();

        let error_data =
            serde_json::to_string(&serde_json::json!({"error": error})).map_err(|e| {
                CheckpointError::Other {
                    message: format!("serialize error: {e}"),
                }
            })?;

        let result = sqlx::query(
            r"UPDATE orchestrator_checkpoints
              SET status = 'failed', interrupt_data = ?1,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE workflow_id = ?2",
        )
        .bind(&error_data)
        .bind(wf_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(CheckpointError::NotFound {
                workflow_id: wf_id.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(workflow_id = %workflow_id))]
    async fn prune(
        &self,
        workflow_id: &WorkflowId,
        keep_after_step: u64,
    ) -> Result<u64, CheckpointError> {
        let wf_id = workflow_id.as_str();
        let step = i64::try_from(keep_after_step).unwrap_or(i64::MAX);

        let result = sqlx::query(
            "DELETE FROM orchestrator_checkpoints WHERE workflow_id = ?1 AND step_number < ?2",
        )
        .bind(wf_id)
        .bind(step)
        .execute(&self.pool)
        .await
        .map_err(|e| CheckpointError::StorageError {
            message: e.to_string(),
        })?;

        Ok(result.rows_affected())
    }
}

// ---------------------------------------------------------------------------
// Internal row mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct CheckpointRow {
    #[allow(dead_code)]
    id: String,
    workflow_id: String,
    session_id: String,
    step_number: i64,
    status: String,
    committed_state: String,
    pending_state: Option<String>,
    interrupt_data: Option<String>,
    versions_seen: String,
    created_at: String,
    updated_at: String,
}

impl CheckpointRow {
    fn into_checkpoint(self) -> Result<WorkflowCheckpoint, CheckpointError> {
        let status = match self.status.as_str() {
            "running" => CheckpointStatus::Running,
            "completed" => CheckpointStatus::Completed,
            "failed" => CheckpointStatus::Failed,
            "interrupted" => CheckpointStatus::Interrupted,
            "compensating" => CheckpointStatus::Compensating,
            other => {
                return Err(CheckpointError::Other {
                    message: format!("unknown checkpoint status: {other}"),
                })
            }
        };

        let committed_state: serde_json::Value = serde_json::from_str(&self.committed_state)
            .map_err(|e| CheckpointError::Other {
                message: format!("parse committed_state: {e}"),
            })?;

        let pending_state = self
            .pending_state
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| CheckpointError::Other {
                message: format!("parse pending_state: {e}"),
            })?;

        let interrupt_data = self
            .interrupt_data
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| CheckpointError::Other {
                message: format!("parse interrupt_data: {e}"),
            })?;

        let versions_seen: serde_json::Value =
            serde_json::from_str(&self.versions_seen).map_err(|e| CheckpointError::Other {
                message: format!("parse versions_seen: {e}"),
            })?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));

        let updated_at = chrono::DateTime::parse_from_rfc3339(&self.updated_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));

        Ok(WorkflowCheckpoint {
            workflow_id: WorkflowId::from_string(self.workflow_id),
            session_id: SessionId::from_string(self.session_id),
            step_number: u64::try_from(self.step_number).unwrap_or(0),
            status,
            committed_state,
            pending_state,
            interrupt_data,
            versions_seen,
            created_at,
            updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::migration::run_embedded_migrations;
    use crate::pool::create_pool;

    /// Helper: create an in-memory pool with migrations applied.
    async fn setup() -> (SqlitePool, SqliteCheckpointStorage) {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();

        // We also need a workflow in orchestrator_workflows to satisfy the FK.
        sqlx::query(
            r"INSERT INTO orchestrator_workflows (id, name, definition, compiled_dag, creator)
              VALUES ('wf-1', 'test-wf', 'test', '{}', 'user')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let storage = SqliteCheckpointStorage::new(pool.clone());
        (pool, storage)
    }

    #[tokio::test]
    async fn test_checkpoint_write_pending_then_read_committed_is_none() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        // Insert a session to satisfy FK.
        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"key": "value"});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();

        // Read committed — should return a checkpoint but committed_state should be empty.
        let checkpoint = storage.read_committed(&wf_id).await.unwrap();
        assert!(checkpoint.is_some());
        let cp = checkpoint.unwrap();
        // The committed state is the initial empty object
        assert_eq!(cp.committed_state, serde_json::json!({}));
    }

    #[tokio::test]
    async fn test_checkpoint_commit_makes_state_durable() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"step": 1, "data": "hello"});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();
        storage.commit(&wf_id, 1).await.unwrap();

        let checkpoint = storage.read_committed(&wf_id).await.unwrap().unwrap();
        assert_eq!(checkpoint.committed_state, state);
        assert!(checkpoint.pending_state.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_overwrite_pending() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state1 = serde_json::json!({"version": 1});
        let state2 = serde_json::json!({"version": 2});

        storage
            .write_pending(&wf_id, &sess_id, 1, &state1)
            .await
            .unwrap();
        storage
            .write_pending(&wf_id, &sess_id, 2, &state2)
            .await
            .unwrap();

        storage.commit(&wf_id, 2).await.unwrap();
        let cp = storage.read_committed(&wf_id).await.unwrap().unwrap();
        assert_eq!(cp.committed_state, state2);
    }

    #[tokio::test]
    async fn test_checkpoint_set_interrupted() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"data": "test"});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();
        storage.commit(&wf_id, 1).await.unwrap();

        let interrupt = serde_json::json!({"reason": "user_input_needed"});
        storage
            .set_interrupted(&wf_id, interrupt.clone())
            .await
            .unwrap();

        let cp = storage.read_committed(&wf_id).await.unwrap().unwrap();
        assert_eq!(cp.status, CheckpointStatus::Interrupted);
        assert_eq!(cp.interrupt_data, Some(interrupt));
    }

    #[tokio::test]
    async fn test_checkpoint_set_completed() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"data": "test"});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();
        storage.commit(&wf_id, 1).await.unwrap();
        storage.set_completed(&wf_id).await.unwrap();

        let cp = storage.read_committed(&wf_id).await.unwrap().unwrap();
        assert_eq!(cp.status, CheckpointStatus::Completed);
    }

    #[tokio::test]
    async fn test_checkpoint_set_failed() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"data": "test"});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();
        storage.commit(&wf_id, 1).await.unwrap();
        storage.set_failed(&wf_id, "something broke").await.unwrap();

        let cp = storage.read_committed(&wf_id).await.unwrap().unwrap();
        assert_eq!(cp.status, CheckpointStatus::Failed);
    }

    #[tokio::test]
    async fn test_checkpoint_prune_old_steps() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("wf-1");
        let sess_id = SessionId::from_string("sess-1");

        sqlx::query(
            r"INSERT INTO session_metadata (id, root_id, path, session_type, transcript_path)
              VALUES ('sess-1', 'sess-1', '[]', 'main', '/tmp/t.jsonl')",
        )
        .execute(&_pool)
        .await
        .unwrap();

        let state = serde_json::json!({"step": 1});
        storage
            .write_pending(&wf_id, &sess_id, 1, &state)
            .await
            .unwrap();
        storage.commit(&wf_id, 1).await.unwrap();

        // Prune steps before 5 — should delete the step 1 checkpoint.
        let deleted = storage.prune(&wf_id, 5).await.unwrap();
        assert_eq!(deleted, 1);

        // Should be gone now.
        let cp = storage.read_committed(&wf_id).await.unwrap();
        assert!(cp.is_none());
    }

    #[tokio::test]
    async fn test_checkpoint_not_found() {
        let (_pool, storage) = setup().await;
        let wf_id = WorkflowId::from_string("nonexistent-wf");
        let result = storage.read_committed(&wf_id).await.unwrap();
        assert!(result.is_none());
    }
}
