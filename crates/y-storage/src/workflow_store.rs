//! SQLite-backed workflow template storage.
//!
//! Persists workflow definitions (Expression DSL or TOML) in the
//! `orchestrator_workflows` table for durable, cross-restart access.

use sqlx::SqlitePool;
use tracing::instrument;

use crate::error::StorageError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A persisted workflow template row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowRow {
    /// Unique template ID (UUID).
    pub id: String,
    /// Human-readable unique name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// Raw definition source (Expression DSL string or TOML text).
    pub definition: String,
    /// Format of the definition: `"expression_dsl"` or `"toml"`.
    pub format: String,
    /// JSON-serialized compiled DAG.
    pub compiled_dag: String,
    /// JSON Schema for parameters (optional).
    pub parameter_schema: Option<String>,
    /// Tags as JSON array string, e.g. `["research","llm"]`.
    pub tags: String,
    /// Who created this workflow: `"user"` or `"agent"`.
    pub creator: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 update timestamp.
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Store implementation
// ---------------------------------------------------------------------------

/// SQLite-backed workflow store.
#[derive(Debug, Clone)]
pub struct SqliteWorkflowStore {
    pool: SqlitePool,
}

impl SqliteWorkflowStore {
    /// Create a new workflow store backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Save (insert) a new workflow template.
    ///
    /// Returns an error if a workflow with the same `id` or `name` already exists.
    #[instrument(skip(self, row), fields(workflow_id = %row.id, name = %row.name))]
    pub async fn save(&self, row: &WorkflowRow) -> Result<(), StorageError> {
        sqlx::query(
            r"INSERT INTO orchestrator_workflows
              (id, name, description, definition, compiled_dag, parameter_schema, tags, creator)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&row.id)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.definition)
        .bind(&row.compiled_dag)
        .bind(&row.parameter_schema)
        .bind(&row.tags)
        .bind(&row.creator)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("save workflow '{}': {e}", row.id),
        })?;

        Ok(())
    }

    /// Get a workflow by ID.
    #[instrument(skip(self))]
    pub async fn get(&self, id: &str) -> Result<Option<WorkflowRow>, StorageError> {
        let row: Option<DbWorkflowRow> = sqlx::query_as(
            r"SELECT id, name, description, definition, compiled_dag,
                     parameter_schema, tags, creator, created_at, updated_at
              FROM orchestrator_workflows
              WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("get workflow '{id}': {e}"),
        })?;

        Ok(row.map(Into::into))
    }

    /// Get a workflow by name.
    #[instrument(skip(self))]
    pub async fn get_by_name(&self, name: &str) -> Result<Option<WorkflowRow>, StorageError> {
        let row: Option<DbWorkflowRow> = sqlx::query_as(
            r"SELECT id, name, description, definition, compiled_dag,
                     parameter_schema, tags, creator, created_at, updated_at
              FROM orchestrator_workflows
              WHERE name = ?1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("get workflow by name '{name}': {e}"),
        })?;

        Ok(row.map(Into::into))
    }

    /// List all workflows, ordered by name.
    #[instrument(skip(self))]
    pub async fn list(&self) -> Result<Vec<WorkflowRow>, StorageError> {
        let rows: Vec<DbWorkflowRow> = sqlx::query_as(
            r"SELECT id, name, description, definition, compiled_dag,
                     parameter_schema, tags, creator, created_at, updated_at
              FROM orchestrator_workflows
              ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list workflows: {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// List workflows that contain the given tag.
    #[instrument(skip(self))]
    pub async fn list_by_tag(&self, tag: &str) -> Result<Vec<WorkflowRow>, StorageError> {
        // SQLite JSON: use json_each to match tag values.
        let rows: Vec<DbWorkflowRow> = sqlx::query_as(
            r"SELECT w.id, w.name, w.description, w.definition, w.compiled_dag,
                     w.parameter_schema, w.tags, w.creator, w.created_at, w.updated_at
              FROM orchestrator_workflows w, json_each(w.tags) t
              WHERE t.value = ?1
              ORDER BY w.name",
        )
        .bind(tag)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("list workflows by tag '{tag}': {e}"),
        })?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Delete a workflow by ID. Returns true if a row was deleted.
    #[instrument(skip(self))]
    pub async fn delete(&self, id: &str) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM orchestrator_workflows WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database {
                message: format!("delete workflow '{id}': {e}"),
            })?;

        Ok(result.rows_affected() > 0)
    }
}

// ---------------------------------------------------------------------------
// Internal DB row mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct DbWorkflowRow {
    id: String,
    name: String,
    description: Option<String>,
    definition: String,
    compiled_dag: String,
    parameter_schema: Option<String>,
    tags: String,
    creator: String,
    created_at: String,
    updated_at: String,
}

impl From<DbWorkflowRow> for WorkflowRow {
    fn from(r: DbWorkflowRow) -> Self {
        let format = if r.definition.contains(">>") || r.definition.contains('|') {
            "expression_dsl".to_string()
        } else {
            "toml".to_string()
        };
        Self {
            id: r.id,
            name: r.name,
            description: r.description,
            definition: r.definition,
            format,
            compiled_dag: r.compiled_dag,
            parameter_schema: r.parameter_schema,
            tags: r.tags,
            creator: r.creator,
            created_at: r.created_at,
            updated_at: r.updated_at,
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

    /// Helper: create an in-memory pool with migrations applied.
    async fn setup() -> (SqlitePool, SqliteWorkflowStore) {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        let store = SqliteWorkflowStore::new(pool.clone());
        (pool, store)
    }

    fn sample_row(id: &str, name: &str) -> WorkflowRow {
        WorkflowRow {
            id: id.to_string(),
            name: name.to_string(),
            description: Some("A test workflow".to_string()),
            definition: "search >> analyze >> summarize".to_string(),
            format: "expression_dsl".to_string(),
            compiled_dag: "{}".to_string(),
            parameter_schema: None,
            tags: r#"["test","research"]"#.to_string(),
            creator: "user".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn test_save_and_get() {
        let (_pool, store) = setup().await;
        let row = sample_row("wf-1", "research-pipeline");
        store.save(&row).await.unwrap();

        let loaded = store.get("wf-1").await.unwrap().expect("should exist");
        assert_eq!(loaded.name, "research-pipeline");
        assert_eq!(loaded.definition, "search >> analyze >> summarize");
        assert_eq!(loaded.format, "expression_dsl");
    }

    #[tokio::test]
    async fn test_get_by_name() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("wf-1", "my-flow")).await.unwrap();

        let loaded = store
            .get_by_name("my-flow")
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(loaded.id, "wf-1");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let (_pool, store) = setup().await;
        let result = store.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_list_all() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("wf-1", "alpha-flow")).await.unwrap();
        store.save(&sample_row("wf-2", "beta-flow")).await.unwrap();

        let rows = store.list().await.unwrap();
        assert_eq!(rows.len(), 2);
        // Ordered by name
        assert_eq!(rows[0].name, "alpha-flow");
        assert_eq!(rows[1].name, "beta-flow");
    }

    #[tokio::test]
    async fn test_list_by_tag() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("wf-1", "flow-a")).await.unwrap();

        let mut row2 = sample_row("wf-2", "flow-b");
        row2.tags = r#"["production"]"#.to_string();
        store.save(&row2).await.unwrap();

        let research = store.list_by_tag("research").await.unwrap();
        assert_eq!(research.len(), 1);
        assert_eq!(research[0].id, "wf-1");

        let prod = store.list_by_tag("production").await.unwrap();
        assert_eq!(prod.len(), 1);
        assert_eq!(prod[0].id, "wf-2");
    }

    #[tokio::test]
    async fn test_delete() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("wf-1", "to-delete")).await.unwrap();

        let deleted = store.delete("wf-1").await.unwrap();
        assert!(deleted);
        assert!(store.get("wf-1").await.unwrap().is_none());

        // Delete again returns false
        let deleted_again = store.delete("wf-1").await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_duplicate_name_rejected() {
        let (_pool, store) = setup().await;
        store.save(&sample_row("wf-1", "same-name")).await.unwrap();
        let result = store.save(&sample_row("wf-2", "same-name")).await;
        assert!(result.is_err(), "duplicate name should fail");
    }
}
