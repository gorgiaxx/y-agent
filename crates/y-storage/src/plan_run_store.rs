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
