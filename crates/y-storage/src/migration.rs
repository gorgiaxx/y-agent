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
    add_column_if_missing(pool, "session_metadata", "custom_system_prompt", "TEXT").await?;
    add_column_if_missing(pool, "session_metadata", "manual_title", "TEXT").await?;
    add_column_if_missing(pool, "schedule_executions", "triggered_at", "TEXT").await?;
    add_column_if_missing(pool, "schedule_executions", "workflow_execution_id", "TEXT").await?;
    add_column_if_missing(pool, "schedule_executions", "request_summary", "TEXT").await?;
    add_column_if_missing(pool, "schedule_executions", "response_summary", "TEXT").await?;

    // Add 'sub_agent' to session_metadata.session_type CHECK constraint.
    // SQLite cannot ALTER CHECK constraints, so we recreate the table if needed.
    migrate_session_type_sub_agent(pool).await?;

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

/// Migrate `session_metadata.session_type` CHECK constraint to include `'sub_agent'`.
///
/// `SQLite` does not support modifying CHECK constraints via ALTER TABLE, so
/// this performs the standard table-recreation pattern. Foreign keys must be
/// disabled during the operation because other tables reference
/// `session_metadata`.
///
/// This is safe to call repeatedly (idempotent).
async fn migrate_session_type_sub_agent(pool: &SqlitePool) -> Result<(), StorageError> {
    // Probe: try inserting a row with session_type = 'sub_agent'.
    // If the CHECK constraint rejects it, we need to migrate.
    let probe = sqlx::query(
        "INSERT INTO session_metadata \
         (id, root_id, depth, path, session_type, state, transcript_path, \
          created_at, updated_at) \
         VALUES ('__probe_sub_agent__', '__probe_sub_agent__', 0, '[]', \
         'sub_agent', 'active', '__probe__', datetime('now'), datetime('now'))",
    )
    .execute(pool)
    .await;

    match probe {
        Ok(_) => {
            // Probe succeeded -- constraint already allows 'sub_agent'.
            let _ = sqlx::query("DELETE FROM session_metadata WHERE id = '__probe_sub_agent__'")
                .execute(pool)
                .await;
            return Ok(());
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("CHECK constraint failed") {
                info!("session_metadata needs migration for 'sub_agent' session type");
            } else {
                return Err(StorageError::Migration {
                    message: format!("probe for sub_agent migration failed: {e}"),
                });
            }
        }
    }

    // Disable foreign keys so DROP TABLE succeeds despite FK references
    // from orchestrator_checkpoints, tool_activation_log, etc.
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to disable foreign keys: {e}"),
        })?;

    // Step 1: Clean up any leftover from a previous failed migration attempt.
    let _ = sqlx::query("DROP TABLE IF EXISTS session_metadata_new")
        .execute(pool)
        .await;

    // Step 2: Create replacement table with updated CHECK constraint.
    sqlx::query(
        "CREATE TABLE session_metadata_new (
            id              TEXT PRIMARY KEY,
            parent_id       TEXT,
            root_id         TEXT NOT NULL,
            depth           INTEGER NOT NULL DEFAULT 0,
            path            TEXT NOT NULL,
            session_type    TEXT NOT NULL CHECK (session_type IN (
                'main','child','branch','ephemeral','sub_agent','canonical'
            )),
            state           TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
                'active','paused','archived','merged','tombstone'
            )),
            agent_id        TEXT,
            title           TEXT,
            manual_title    TEXT,
            token_count     INTEGER NOT NULL DEFAULT 0,
            message_count   INTEGER NOT NULL DEFAULT 0,
            transcript_path TEXT NOT NULL,
            channel         TEXT,
            label           TEXT,
            last_compaction TEXT,
            compaction_count INTEGER NOT NULL DEFAULT 0,
            context_reset_index INTEGER,
            custom_system_prompt TEXT,
            created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
            updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| StorageError::Migration {
        message: format!("failed to create session_metadata_new: {e}"),
    })?;

    // Step 3: Copy data.
    sqlx::query("INSERT INTO session_metadata_new SELECT * FROM session_metadata")
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to copy session_metadata data: {e}"),
        })?;

    // Step 4: Drop old table.
    sqlx::query("DROP TABLE session_metadata")
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to drop old session_metadata: {e}"),
        })?;

    // Step 5: Rename new table.
    sqlx::query("ALTER TABLE session_metadata_new RENAME TO session_metadata")
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to rename session_metadata_new: {e}"),
        })?;

    // Step 6: Recreate indexes.
    for idx_sql in [
        "CREATE INDEX IF NOT EXISTS idx_session_parent ON session_metadata(parent_id)",
        "CREATE INDEX IF NOT EXISTS idx_session_root   ON session_metadata(root_id)",
        "CREATE INDEX IF NOT EXISTS idx_session_state  ON session_metadata(state)",
        "CREATE INDEX IF NOT EXISTS idx_session_agent  ON session_metadata(agent_id)",
    ] {
        sqlx::query(idx_sql)
            .execute(pool)
            .await
            .map_err(|e| StorageError::Migration {
                message: format!("failed to recreate session index: {e}"),
            })?;
    }

    // Re-enable foreign keys.
    let _ = sqlx::query("PRAGMA foreign_keys = ON").execute(pool).await;

    info!("Migrated session_metadata CHECK constraint to include 'sub_agent'");
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
