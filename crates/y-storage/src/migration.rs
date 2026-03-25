//! `SQLite` schema initializer.
//!
//! Embeds the consolidated schema DDL at compile time via `include_str!` and
//! executes it as raw SQL. All `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF
//! NOT EXISTS` statements are naturally idempotent.

use sqlx::SqlitePool;
use tracing::info;

use crate::error::StorageError;

/// Full DDL for a fresh database, embedded at compile time.
const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Initialize the `SQLite` database with the embedded schema.
///
/// This is the preferred (and only) method. The entire DDL is compiled into
/// the binary so no external migration directory is needed at runtime. Every
/// statement uses `IF NOT EXISTS`, making the call safe to repeat.
pub async fn run_embedded_migrations(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::raw_sql(SCHEMA_SQL)
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("schema initialization failed: {e}"),
        })?;

    // Post-schema migrations for existing databases that were created before
    // new columns were added to schema.sql. SQLite lacks ADD COLUMN IF NOT
    // EXISTS, so we check PRAGMA table_info first.
    add_column_if_missing(pool, "session_metadata", "context_reset_index", "INTEGER").await?;

    info!("SQLite schema initialized");

    Ok(())
}

/// Add a column to an existing table only if it is not already present.
///
/// Uses `PRAGMA table_info` to inspect the schema, then runs `ALTER TABLE`
/// when the column is missing. This is safe to call repeatedly (idempotent).
async fn add_column_if_missing(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    col_type: &str,
) -> Result<(), StorageError> {
    let rows: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT name FROM pragma_table_info('{table}') WHERE name = '{column}'"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| StorageError::Migration {
        message: format!("failed to inspect {table}.{column}: {e}"),
    })?;

    if rows.is_empty() {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}");
        sqlx::query(&sql)
            .execute(pool)
            .await
            .map_err(|e| StorageError::Migration {
                message: format!("failed to add {table}.{column}: {e}"),
            })?;
        info!("Added column {table}.{column} ({col_type})");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::pool::create_pool;

    async fn setup_pool_with_migrations() -> SqlitePool {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.expect("pool creation");
        run_embedded_migrations(&pool).await.expect("migrations");
        pool
    }

    #[tokio::test]
    async fn test_migration_run_creates_tables() {
        let pool = setup_pool_with_migrations().await;

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("should list tables");

        let table_names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        assert!(
            table_names.contains(&"session_metadata"),
            "session_metadata table should exist, got: {table_names:?}"
        );
        assert!(
            table_names.contains(&"orchestrator_checkpoints"),
            "orchestrator_checkpoints table should exist, got: {table_names:?}"
        );
    }

    #[tokio::test]
    async fn test_migration_idempotent() {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.expect("pool creation");

        // Run twice -- second run should be a no-op thanks to IF NOT EXISTS.
        run_embedded_migrations(&pool).await.expect("first run");
        run_embedded_migrations(&pool)
            .await
            .expect("second run should not fail");
    }

    #[tokio::test]
    async fn test_schema_creates_all_expected_tables() {
        let pool = setup_pool_with_migrations().await;

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("should list tables");

        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();
        let expected = [
            "session_metadata",
            "orchestrator_workflows",
            "orchestrator_checkpoints",
            "file_journal_entries",
            "tool_dynamic_definitions",
            "tool_activation_log",
            "agent_definitions",
            "schedule_definitions",
            "schedule_executions",
            "stm_experience_store",
            "dynamic_agents",
            "chat_checkpoints",
            "chat_messages",
            "diag_traces",
            "diag_observations",
            "diag_scores",
            "provider_metrics_log",
        ];
        for table in &expected {
            assert!(
                names.contains(table),
                "expected table '{table}' missing, got: {names:?}"
            );
        }
    }
}
