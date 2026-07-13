//! SQLite-backed store for plan execution runs and per-step results.
//!
//! Enables step-level resume: after each task completes, its result is persisted
//! so that a failed or unsatisfactory plan can be resumed from any step.

use sqlx::SqlitePool;
use tracing::instrument;

use crate::StorageError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanRunRow {
    pub id: String,
    pub session_id: String,
    pub plan_json: String,
    pub plan_path: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanStepResultRow {
    pub plan_run_id: String,
    pub task_id: String,
    pub phase: i64,
    pub title: String,
    pub status: String,
    pub output_json: Option<String>,
    pub completed_at: String,
}

// ---------------------------------------------------------------------------
// DB row types (private)
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct DbPlanRunRow {
    id: String,
    session_id: String,
    plan_json: String,
    plan_path: String,
    status: String,
    created_at: String,
    updated_at: String,
}

impl From<DbPlanRunRow> for PlanRunRow {
    fn from(r: DbPlanRunRow) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id,
            plan_json: r.plan_json,
            plan_path: r.plan_path,
            status: r.status,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct DbPlanStepResultRow {
    plan_run_id: String,
    task_id: String,
    phase: i64,
    title: String,
    status: String,
    output_json: Option<String>,
    completed_at: String,
}

impl From<DbPlanStepResultRow> for PlanStepResultRow {
    fn from(r: DbPlanStepResultRow) -> Self {
        Self {
            plan_run_id: r.plan_run_id,
            task_id: r.task_id,
            phase: r.phase,
            title: r.title,
            status: r.status,
            output_json: r.output_json,
            completed_at: r.completed_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Store implementation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SqlitePlanRunStore {
    pool: SqlitePool,
}

impl SqlitePlanRunStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    #[instrument(skip(self, plan_json))]
    pub async fn create_run(
        &self,
        id: &str,
        session_id: &str,
        plan_json: &str,
        plan_path: &str,
    ) -> Result<(), StorageError> {
        self.create_run_with_status(id, session_id, plan_json, plan_path, "running")
            .await
    }

    #[instrument(skip(self, plan_json))]
    pub async fn create_run_with_status(
        &self,
        id: &str,
        session_id: &str,
        plan_json: &str,
        plan_path: &str,
        status: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r"INSERT INTO plan_runs (id, session_id, plan_json, plan_path, status)
              VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(id)
        .bind(session_id)
        .bind(plan_json)
        .bind(plan_path)
        .bind(status)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("create plan run '{id}': {e}"),
        })?;
        Ok(())
    }

    #[instrument(skip(self, output_json))]
    pub async fn record_step_result(
        &self,
        plan_run_id: &str,
        task_id: &str,
        phase: usize,
        title: &str,
        status: &str,
        output_json: Option<&str>,
    ) -> Result<(), StorageError> {
        let phase_i64 = i64::try_from(phase).unwrap_or(0);
        sqlx::query(
            r"INSERT OR REPLACE INTO plan_step_results
              (plan_run_id, task_id, phase, title, status, output_json)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(plan_run_id)
        .bind(task_id)
        .bind(phase_i64)
        .bind(title)
        .bind(status)
        .bind(output_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("record step result '{plan_run_id}/{task_id}': {e}"),
        })?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn update_run_status(
        &self,
        plan_run_id: &str,
        status: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r"UPDATE plan_runs
              SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(status)
        .bind(plan_run_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("update plan run status '{plan_run_id}': {e}"),
        })?;
        Ok(())
    }

    /// Replace an existing run with a newly drafted revision while preserving
    /// its stable identity. Old step rows refer to the superseded task graph,
    /// so the plan replacement and step cleanup are committed atomically.
    #[instrument(skip(self, plan_json))]
    pub async fn replace_run_for_revision(
        &self,
        plan_run_id: &str,
        plan_json: &str,
        plan_path: &str,
    ) -> Result<(), StorageError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Database {
                message: format!("begin plan revision '{plan_run_id}': {e}"),
            })?;

        sqlx::query(
            r"UPDATE plan_runs
              SET plan_json = ?1,
                  plan_path = ?2,
                  status = 'awaiting_approval',
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?3",
        )
        .bind(plan_json)
        .bind(plan_path)
        .bind(plan_run_id)
        .execute(&mut *transaction)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("replace plan run for revision '{plan_run_id}': {e}"),
        })?;

        sqlx::query("DELETE FROM plan_step_results WHERE plan_run_id = ?1")
            .bind(plan_run_id)
            .execute(&mut *transaction)
            .await
            .map_err(|e| StorageError::Database {
                message: format!("clear superseded plan steps '{plan_run_id}': {e}"),
            })?;

        transaction
            .commit()
            .await
            .map_err(|e| StorageError::Database {
                message: format!("commit plan revision '{plan_run_id}': {e}"),
            })?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn load_run(&self, plan_run_id: &str) -> Result<Option<PlanRunRow>, StorageError> {
        let row: Option<DbPlanRunRow> = sqlx::query_as(
            r"SELECT id, session_id, plan_json, plan_path, status, created_at, updated_at
              FROM plan_runs WHERE id = ?1",
        )
        .bind(plan_run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("load plan run '{plan_run_id}': {e}"),
        })?;
        Ok(row.map(Into::into))
    }

    #[instrument(skip(self))]
    pub async fn load_step_results(
        &self,
        plan_run_id: &str,
    ) -> Result<Vec<PlanStepResultRow>, StorageError> {
        let rows: Vec<DbPlanStepResultRow> = sqlx::query_as(
            r"SELECT plan_run_id, task_id, phase, title, status, output_json, completed_at
              FROM plan_step_results
              WHERE plan_run_id = ?1
              ORDER BY completed_at",
        )
        .bind(plan_run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("load step results for run '{plan_run_id}': {e}"),
        })?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    #[instrument(skip(self))]
    pub async fn find_latest_run(
        &self,
        session_id: &str,
    ) -> Result<Option<PlanRunRow>, StorageError> {
        let row: Option<DbPlanRunRow> = sqlx::query_as(
            r"SELECT id, session_id, plan_json, plan_path, status, created_at, updated_at
              FROM plan_runs
              WHERE session_id = ?1
              ORDER BY created_at DESC
              LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("find latest plan run for session '{session_id}': {e}"),
        })?;
        Ok(row.map(Into::into))
    }

    #[instrument(skip(self))]
    pub async fn delete_step_results(
        &self,
        plan_run_id: &str,
        task_ids: &[&str],
    ) -> Result<(), StorageError> {
        if task_ids.is_empty() {
            return Ok(());
        }
        let placeholders: String = task_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "DELETE FROM plan_step_results WHERE plan_run_id = ?1 AND task_id IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql).bind(plan_run_id);
        for id in task_ids {
            query = query.bind(*id);
        }
        query
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database {
                message: format!("delete step results for run '{plan_run_id}': {e}"),
            })?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn find_awaiting_runs_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<PlanRunRow>, StorageError> {
        let rows: Vec<DbPlanRunRow> = sqlx::query_as(
            r"SELECT id, session_id, plan_json, plan_path, status, created_at, updated_at
              FROM plan_runs
              WHERE session_id = ?1 AND status = 'awaiting_approval'
              ORDER BY created_at DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("find awaiting runs for session '{session_id}': {e}"),
        })?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// List every plan run for a session, oldest first, so the UI can present
    /// the full history (completed, failed, awaiting, running) and not just the
    /// most recent run.
    #[instrument(skip(self))]
    pub async fn list_runs_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<PlanRunRow>, StorageError> {
        let rows: Vec<DbPlanRunRow> = sqlx::query_as(
            r"SELECT id, session_id, plan_json, plan_path, status, created_at, updated_at
              FROM plan_runs
              WHERE session_id = ?1
              ORDER BY created_at ASC, rowid ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list runs for session '{session_id}': {e}"),
        })?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    #[instrument(skip(self))]
    pub async fn cancel_awaiting_runs_for_session(
        &self,
        session_id: &str,
    ) -> Result<u64, StorageError> {
        let result = sqlx::query(
            r"UPDATE plan_runs
              SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE session_id = ?1 AND status = 'awaiting_approval'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("cancel awaiting runs for session '{session_id}': {e}"),
        })?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::migration::run_embedded_migrations;
    use crate::pool::create_pool;

    async fn setup() -> SqlitePlanRunStore {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        SqlitePlanRunStore::new(pool)
    }

    #[tokio::test]
    async fn test_list_runs_for_session_returns_all_runs_oldest_first() {
        let store = setup().await;
        store
            .create_run("run-1", "sess-a", "{}", "/p1.md")
            .await
            .unwrap();
        store
            .create_run_with_status("run-2", "sess-a", "{}", "/p2.md", "awaiting_approval")
            .await
            .unwrap();
        store.update_run_status("run-1", "completed").await.unwrap();
        // A run for a different session must not leak in.
        store
            .create_run("run-other", "sess-b", "{}", "/p3.md")
            .await
            .unwrap();

        let runs = store.list_runs_for_session("sess-a").await.unwrap();

        let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["run-1", "run-2"]);
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[1].status, "awaiting_approval");
    }

    #[tokio::test]
    async fn test_list_runs_for_session_is_empty_when_none() {
        let store = setup().await;
        let runs = store.list_runs_for_session("nobody").await.unwrap();
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn test_replace_run_for_revision_updates_plan_and_clears_old_steps() {
        let store = setup().await;
        store
            .create_run("run-1", "sess-a", r#"{"plan_title":"Old"}"#, "/old.md")
            .await
            .unwrap();
        store
            .record_step_result(
                "run-1",
                "old-task",
                1,
                "Old task",
                "completed",
                Some("done"),
            )
            .await
            .unwrap();

        store
            .replace_run_for_revision("run-1", r#"{"plan_title":"Revised"}"#, "/revised.md")
            .await
            .unwrap();

        let run = store.load_run("run-1").await.unwrap().unwrap();
        assert_eq!(run.plan_json, r#"{"plan_title":"Revised"}"#);
        assert_eq!(run.plan_path, "/revised.md");
        assert_eq!(run.status, "awaiting_approval");
        assert!(store.load_step_results("run-1").await.unwrap().is_empty());
    }
}
