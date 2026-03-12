//! SQLite-backed schedule storage.
//!
//! Persists schedule definitions and execution history in `SQLite` tables
//! (`schedule_definitions`, `schedule_executions`).

use sqlx::SqlitePool;
use tracing::instrument;

use crate::error::StorageError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A persisted schedule definition row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleRow {
    /// Unique schedule ID (UUID).
    pub id: String,
    /// Human-readable name (unique).
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Schedule type: `"cron"`, `"interval"`, `"event"`, or `"onetime"`.
    pub schedule_type: String,
    /// Schedule expression (cron expr, interval seconds, event filter, or ISO timestamp).
    pub schedule_expr: String,
    /// Workflow to execute.
    pub workflow_id: String,
    /// JSON: parameter name → value/expression.
    pub parameter_bindings: Option<String>,
    /// JSON Schema (from workflow).
    pub parameter_schema: Option<String>,
    /// Whether this schedule is active.
    pub enabled: bool,
    /// Creator: `"user"` or `"agent"`.
    pub creator: String,
    /// Missed fire policy.
    pub missed_policy: String,
    /// Concurrency policy.
    pub concurrency_policy: String,
    /// Max executions per hour (0 = unlimited).
    pub max_executions_per_hour: i64,
    /// Tags as JSON array string.
    pub tags: String,
    /// Last fire time (ISO-8601 or None).
    pub last_fire: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
}

/// A persisted schedule execution row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleExecutionRow {
    pub id: String,
    pub schedule_id: String,
    pub session_id: Option<String>,
    pub status: String,
    pub resolved_params: Option<String>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Store implementation
// ---------------------------------------------------------------------------

/// SQLite-backed schedule store.
#[derive(Debug, Clone)]
pub struct SqliteScheduleStore {
    pool: SqlitePool,
}

impl SqliteScheduleStore {
    /// Create a new schedule store backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ── Schedule CRUD ──────────────────────────────────────────────────

    /// Save a new schedule definition.
    #[instrument(skip(self, row), fields(schedule_id = %row.id, name = %row.name))]
    pub async fn save(&self, row: &ScheduleRow) -> Result<(), StorageError> {
        sqlx::query(
            r"INSERT INTO schedule_definitions
              (id, name, description, schedule_type, schedule_expr, workflow_id,
               parameter_bindings, parameter_schema, enabled, creator,
               missed_policy, concurrency_policy, max_executions_per_hour, tags, last_fire)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.schedule_type)
        .bind(&row.schedule_expr)
        .bind(&row.workflow_id)
        .bind(&row.parameter_bindings)
        .bind(&row.parameter_schema)
        .bind(row.enabled)
        .bind(&row.creator)
        .bind(&row.missed_policy)
        .bind(&row.concurrency_policy)
        .bind(row.max_executions_per_hour)
        .bind(&row.tags)
        .bind(&row.last_fire)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("save schedule '{}': {e}", row.id),
        })?;

        Ok(())
    }

    /// Get a schedule by ID.
    #[instrument(skip(self))]
    pub async fn get(&self, id: &str) -> Result<Option<ScheduleRow>, StorageError> {
        let row: Option<DbScheduleRow> = sqlx::query_as(
            r"SELECT id, name, description, schedule_type, schedule_expr, workflow_id,
                     parameter_bindings, parameter_schema, enabled, creator,
                     missed_policy, concurrency_policy, max_executions_per_hour,
                     tags, last_fire, created_at, updated_at
              FROM schedule_definitions
              WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("get schedule '{id}': {e}"),
        })?;

        Ok(row.map(Into::into))
    }

    /// List all schedules, ordered by name.
    #[instrument(skip(self))]
    pub async fn list(&self) -> Result<Vec<ScheduleRow>, StorageError> {
        let rows: Vec<DbScheduleRow> = sqlx::query_as(
            r"SELECT id, name, description, schedule_type, schedule_expr, workflow_id,
                     parameter_bindings, parameter_schema, enabled, creator,
                     missed_policy, concurrency_policy, max_executions_per_hour,
                     tags, last_fire, created_at, updated_at
              FROM schedule_definitions
              ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list schedules: {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// List enabled schedules only.
    #[instrument(skip(self))]
    pub async fn list_enabled(&self) -> Result<Vec<ScheduleRow>, StorageError> {
        let rows: Vec<DbScheduleRow> = sqlx::query_as(
            r"SELECT id, name, description, schedule_type, schedule_expr, workflow_id,
                     parameter_bindings, parameter_schema, enabled, creator,
                     missed_policy, concurrency_policy, max_executions_per_hour,
                     tags, last_fire, created_at, updated_at
              FROM schedule_definitions
              WHERE enabled = 1
              ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list enabled schedules: {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// List schedules that have a given tag.
    #[instrument(skip(self))]
    pub async fn list_by_tag(&self, tag: &str) -> Result<Vec<ScheduleRow>, StorageError> {
        let rows: Vec<DbScheduleRow> = sqlx::query_as(
            r"SELECT s.id, s.name, s.description, s.schedule_type, s.schedule_expr,
                     s.workflow_id, s.parameter_bindings, s.parameter_schema,
                     s.enabled, s.creator, s.missed_policy, s.concurrency_policy,
                     s.max_executions_per_hour, s.tags, s.last_fire,
                     s.created_at, s.updated_at
              FROM schedule_definitions s, json_each(s.tags) t
              WHERE t.value = ?1
              ORDER BY s.name",
        )
        .bind(tag)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list schedules by tag '{tag}': {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Update a schedule definition (by ID).
    #[instrument(skip(self, row), fields(schedule_id = %row.id))]
    pub async fn update(&self, row: &ScheduleRow) -> Result<bool, StorageError> {
        let result = sqlx::query(
            r"UPDATE schedule_definitions SET
                name = ?1, description = ?2, schedule_type = ?3, schedule_expr = ?4,
                workflow_id = ?5, parameter_bindings = ?6, parameter_schema = ?7,
                enabled = ?8, missed_policy = ?9, concurrency_policy = ?10,
                max_executions_per_hour = ?11, tags = ?12, last_fire = ?13,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?14",
        )
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.schedule_type)
        .bind(&row.schedule_expr)
        .bind(&row.workflow_id)
        .bind(&row.parameter_bindings)
        .bind(&row.parameter_schema)
        .bind(row.enabled)
        .bind(&row.missed_policy)
        .bind(&row.concurrency_policy)
        .bind(row.max_executions_per_hour)
        .bind(&row.tags)
        .bind(&row.last_fire)
        .bind(&row.id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("update schedule '{}': {e}", row.id),
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Update just the `last_fire` timestamp.
    #[instrument(skip(self))]
    pub async fn update_last_fire(
        &self,
        id: &str,
        last_fire: &str,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            r"UPDATE schedule_definitions SET
                last_fire = ?1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(last_fire)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("update last_fire for '{id}': {e}"),
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Set enabled/disabled.
    #[instrument(skip(self))]
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, StorageError> {
        let result = sqlx::query(
            r"UPDATE schedule_definitions SET
                enabled = ?1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(enabled)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("set_enabled for '{id}': {e}"),
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a schedule by ID. Returns true if a row was deleted.
    #[instrument(skip(self))]
    pub async fn delete(&self, id: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM schedule_definitions WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database {
                message: format!("delete schedule '{id}': {e}"),
            })?;

        Ok(result.rows_affected() > 0)
    }

    // ── Execution history ──────────────────────────────────────────────

    /// Record a schedule execution.
    #[instrument(skip(self, row), fields(execution_id = %row.id, schedule_id = %row.schedule_id))]
    pub async fn record_execution(
        &self,
        row: &ScheduleExecutionRow,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r"INSERT INTO schedule_executions
              (id, schedule_id, session_id, status, resolved_params, error_message, started_at, completed_at)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&row.id)
        .bind(&row.schedule_id)
        .bind(&row.session_id)
        .bind(&row.status)
        .bind(&row.resolved_params)
        .bind(&row.error_message)
        .bind(&row.started_at)
        .bind(&row.completed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("record execution '{}': {e}", row.id),
        })?;

        Ok(())
    }

    /// Get execution history for a schedule, newest first.
    #[instrument(skip(self))]
    pub async fn get_executions(
        &self,
        schedule_id: &str,
        limit: i64,
    ) -> Result<Vec<ScheduleExecutionRow>, StorageError> {
        let rows: Vec<DbExecutionRow> = sqlx::query_as(
            r"SELECT id, schedule_id, session_id, status, resolved_params,
                     error_message, started_at, completed_at
              FROM schedule_executions
              WHERE schedule_id = ?1
              ORDER BY started_at DESC
              LIMIT ?2",
        )
        .bind(schedule_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("get executions for '{schedule_id}': {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Update execution status.
    #[instrument(skip(self))]
    pub async fn update_execution_status(
        &self,
        execution_id: &str,
        status: &str,
        error_message: Option<&str>,
        completed_at: Option<&str>,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            r"UPDATE schedule_executions SET
                status = ?1, error_message = ?2, completed_at = ?3
              WHERE id = ?4",
        )
        .bind(status)
        .bind(error_message)
        .bind(completed_at)
        .bind(execution_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("update execution '{execution_id}': {e}"),
        })?;

        Ok(result.rows_affected() > 0)
    }
}

// ---------------------------------------------------------------------------
// Internal DB row mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct DbScheduleRow {
    id: String,
    name: String,
    description: Option<String>,
    schedule_type: String,
    schedule_expr: String,
    workflow_id: String,
    parameter_bindings: Option<String>,
    parameter_schema: Option<String>,
    enabled: bool,
    creator: String,
    missed_policy: String,
    concurrency_policy: String,
    max_executions_per_hour: i64,
    tags: String,
    last_fire: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<DbScheduleRow> for ScheduleRow {
    fn from(r: DbScheduleRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            description: r.description,
            schedule_type: r.schedule_type,
            schedule_expr: r.schedule_expr,
            workflow_id: r.workflow_id,
            parameter_bindings: r.parameter_bindings,
            parameter_schema: r.parameter_schema,
            enabled: r.enabled,
            creator: r.creator,
            missed_policy: r.missed_policy,
            concurrency_policy: r.concurrency_policy,
            max_executions_per_hour: r.max_executions_per_hour,
            tags: r.tags,
            last_fire: r.last_fire,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct DbExecutionRow {
    id: String,
    schedule_id: String,
    session_id: Option<String>,
    status: String,
    resolved_params: Option<String>,
    error_message: Option<String>,
    started_at: String,
    completed_at: Option<String>,
}

impl From<DbExecutionRow> for ScheduleExecutionRow {
    fn from(r: DbExecutionRow) -> Self {
        Self {
            id: r.id,
            schedule_id: r.schedule_id,
            session_id: r.session_id,
            status: r.status,
            resolved_params: r.resolved_params,
            error_message: r.error_message,
            started_at: r.started_at,
            completed_at: r.completed_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::migration::run_embedded_migrations;
    use crate::pool::create_pool;

    async fn setup() -> (SqlitePool, SqliteScheduleStore) {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        let store = SqliteScheduleStore::new(pool.clone());
        // Insert a dummy workflow for FK constraint.
        sqlx::query(
            r"INSERT INTO orchestrator_workflows
              (id, name, definition, compiled_dag, tags, creator)
              VALUES ('wf-1', 'test-wf', 'a >> b', '{}', '[]', 'user')")
            .execute(&pool)
            .await
            .unwrap();
        (pool, store)
    }

    fn sample_row(id: &str, name: &str) -> ScheduleRow {
        ScheduleRow {
            id: id.to_string(),
            name: name.to_string(),
            description: Some("Test schedule".into()),
            schedule_type: "interval".into(),
            schedule_expr: "3600".into(),
            workflow_id: "wf-1".into(),
            parameter_bindings: None,
            parameter_schema: None,
            enabled: true,
            creator: "user".into(),
            missed_policy: "skip".into(),
            concurrency_policy: "skip".into(),
            max_executions_per_hour: 0,
            tags: r#"["test"]"#.into(),
            last_fire: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn test_save_and_get() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "daily-cleanup")).await.unwrap();

        let loaded = store.get("s1").await.unwrap().expect("should exist");
        assert_eq!(loaded.name, "daily-cleanup");
        assert_eq!(loaded.schedule_type, "interval");
        assert!(loaded.enabled);
    }

    #[tokio::test]
    async fn test_list_all() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "alpha")).await.unwrap();
        store.save(&sample_row("s2", "beta")).await.unwrap();

        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alpha");
    }

    #[tokio::test]
    async fn test_list_enabled() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "enabled-one")).await.unwrap();
        let mut disabled = sample_row("s2", "disabled-one");
        disabled.enabled = false;
        store.save(&disabled).await.unwrap();

        let enabled = store.list_enabled().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "s1");
    }

    #[tokio::test]
    async fn test_list_by_tag() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "tagged")).await.unwrap();

        let mut s2 = sample_row("s2", "untagged");
        s2.tags = r#"["production"]"#.into();
        store.save(&s2).await.unwrap();

        let test_results = store.list_by_tag("test").await.unwrap();
        assert_eq!(test_results.len(), 1);
        assert_eq!(test_results[0].id, "s1");
    }

    #[tokio::test]
    async fn test_update() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "original")).await.unwrap();

        let mut updated = store.get("s1").await.unwrap().unwrap();
        updated.name = "updated-name".into();
        updated.missed_policy = "catch_up".into();
        assert!(store.update(&updated).await.unwrap());

        let loaded = store.get("s1").await.unwrap().unwrap();
        assert_eq!(loaded.name, "updated-name");
        assert_eq!(loaded.missed_policy, "catch_up");
    }

    #[tokio::test]
    async fn test_set_enabled() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "toggle")).await.unwrap();

        store.set_enabled("s1", false).await.unwrap();
        assert!(!store.get("s1").await.unwrap().unwrap().enabled);

        store.set_enabled("s1", true).await.unwrap();
        assert!(store.get("s1").await.unwrap().unwrap().enabled);
    }

    #[tokio::test]
    async fn test_update_last_fire() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "fire-test")).await.unwrap();

        store.update_last_fire("s1", "2026-03-11T09:00:00Z").await.unwrap();
        let loaded = store.get("s1").await.unwrap().unwrap();
        assert_eq!(loaded.last_fire.as_deref(), Some("2026-03-11T09:00:00Z"));
    }

    #[tokio::test]
    async fn test_delete() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "to-delete")).await.unwrap();

        assert!(store.delete("s1").await.unwrap());
        assert!(store.get("s1").await.unwrap().is_none());
        assert!(!store.delete("s1").await.unwrap());
    }

    #[tokio::test]
    async fn test_record_and_get_execution() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "exec-test")).await.unwrap();

        let exec = ScheduleExecutionRow {
            id: "exec-1".into(),
            schedule_id: "s1".into(),
            session_id: None,
            status: "completed".into(),
            resolved_params: Some(r#"{"key":"value"}"#.into()),
            error_message: None,
            started_at: "2026-03-11T09:00:00Z".into(),
            completed_at: Some("2026-03-11T09:01:00Z".into()),
        };
        store.record_execution(&exec).await.unwrap();

        let history = store.get_executions("s1", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "completed");
    }

    #[tokio::test]
    async fn test_update_execution_status() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("s1", "status-test")).await.unwrap();

        let exec = ScheduleExecutionRow {
            id: "exec-2".into(),
            schedule_id: "s1".into(),
            session_id: None,
            status: "triggered".into(),
            resolved_params: None,
            error_message: None,
            started_at: "2026-03-11T09:00:00Z".into(),
            completed_at: None,
        };
        store.record_execution(&exec).await.unwrap();

        store
            .update_execution_status("exec-2", "failed", Some("timeout"), Some("2026-03-11T09:05:00Z"))
            .await
            .unwrap();

        let history = store.get_executions("s1", 10).await.unwrap();
        assert_eq!(history[0].status, "failed");
        assert_eq!(history[0].error_message.as_deref(), Some("timeout"));
    }
}
