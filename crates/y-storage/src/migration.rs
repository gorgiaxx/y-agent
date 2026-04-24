//! `SQLite` schema initializer.
//!
//! Embeds the consolidated schema DDL at compile time via `include_str!` and
//! executes it as raw SQL. All `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF
//! NOT EXISTS` statements are naturally idempotent.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection, SqlitePool};
use tracing::{info, warn};

use crate::config::StorageConfig;
use crate::error::StorageError;

/// Full DDL for a fresh database, embedded at compile time.
const SCHEMA_SQL: &str = include_str!("schema.sql");
/// Monotonic storage schema version mirrored in `PRAGMA user_version`.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

const REQUIRED_TABLES: &[&str] = &[
    "session_metadata",
    "orchestrator_workflows",
    "orchestrator_checkpoints",
    "schedule_definitions",
    "schedule_executions",
    "chat_checkpoints",
    "chat_messages",
    "diag_traces",
    "diag_observations",
    "diag_scores",
    "provider_metrics_log",
];

const REQUIRED_SESSION_COLUMNS: &[&str] = &[
    "manual_title",
    "context_reset_index",
    "custom_system_prompt",
];
const REQUIRED_SCHEDULE_COLUMNS: &[&str] = &[
    "missed_policy",
    "concurrency_policy",
    "max_executions_per_hour",
    "last_fire",
];
const REQUIRED_CHAT_MESSAGE_COLUMNS: &[&str] = &["parent_message_id", "pruning_group_id"];
const REQUIRED_TRACE_COLUMNS: &[&str] = &["tags", "replay_context"];

/// Prepare an on-disk database before normal pool creation.
///
/// Existing databases created by the legacy sqlx migration flow or by an older
/// incompatible schema revision are archived and replaced on the next startup.
pub async fn prepare_database(config: &StorageConfig) -> Result<(), StorageError> {
    if config.is_in_memory() {
        return Ok(());
    }

    let db_path = Path::new(&config.db_path);
    if !db_path.exists() {
        return Ok(());
    }

    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&config.db_path)
            .create_if_missing(false)
            .foreign_keys(true),
    )
    .await
    .map_err(|e| StorageError::Connection {
        message: format!("failed to open SQLite database for compatibility check: {e}"),
    })?;

    let incompatibility = incompatibility_reason(&mut connection).await?;
    connection
        .close()
        .await
        .map_err(|e| StorageError::Connection {
            message: format!("failed to close SQLite compatibility-check connection: {e}"),
        })?;

    if let Some(reason) = incompatibility {
        archive_incompatible_database(db_path, &reason).await?;
    }

    Ok(())
}

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

    sqlx::query(&format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"))
        .execute(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to persist schema version: {e}"),
        })?;

    info!("SQLite schema initialized");

    Ok(())
}

async fn incompatibility_reason(
    connection: &mut SqliteConnection,
) -> Result<Option<String>, StorageError> {
    let table_names = user_table_names(connection).await?;
    if table_names.is_empty() {
        return Ok(None);
    }

    if table_names.contains("_sqlx_migrations") {
        return Ok(Some(
            "legacy sqlx migration metadata detected; reset required".to_string(),
        ));
    }

    let user_version = current_user_version(connection).await?;
    let schema_matches = schema_shape_matches(connection, &table_names).await?;

    if user_version == CURRENT_SCHEMA_VERSION && schema_matches {
        return Ok(None);
    }

    if user_version == 0 && schema_matches {
        set_user_version(connection, CURRENT_SCHEMA_VERSION).await?;
        info!(
            schema_version = CURRENT_SCHEMA_VERSION,
            "adopted existing SQLite database into versioned schema tracking"
        );
        return Ok(None);
    }

    if user_version != CURRENT_SCHEMA_VERSION {
        return Ok(Some(format!(
            "schema version mismatch: database={user_version}, expected={CURRENT_SCHEMA_VERSION}",
        )));
    }

    if !schema_matches {
        return Ok(Some(
            "database schema does not match the required runtime shape".to_string(),
        ));
    }

    Ok(None)
}

async fn schema_shape_matches(
    connection: &mut SqliteConnection,
    table_names: &BTreeSet<String>,
) -> Result<bool, StorageError> {
    if REQUIRED_TABLES
        .iter()
        .any(|table| !table_names.contains(*table))
    {
        return Ok(false);
    }

    if !table_has_columns(connection, "session_metadata", REQUIRED_SESSION_COLUMNS).await? {
        return Ok(false);
    }

    if !table_has_columns(
        connection,
        "schedule_definitions",
        REQUIRED_SCHEDULE_COLUMNS,
    )
    .await?
    {
        return Ok(false);
    }

    if !table_has_columns(connection, "chat_messages", REQUIRED_CHAT_MESSAGE_COLUMNS).await? {
        return Ok(false);
    }

    if !table_has_columns(connection, "diag_traces", REQUIRED_TRACE_COLUMNS).await? {
        return Ok(false);
    }

    Ok(true)
}

async fn user_table_names(
    connection: &mut SqliteConnection,
) -> Result<BTreeSet<String>, StorageError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         ORDER BY name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| StorageError::Database {
        message: format!("failed to read SQLite table list: {e}"),
    })?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}

async fn current_user_version(connection: &mut SqliteConnection) -> Result<i64, StorageError> {
    let row = sqlx::query("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("failed to read SQLite user_version: {e}"),
        })?;
    Ok(row.get(0))
}

async fn set_user_version(
    connection: &mut SqliteConnection,
    version: i64,
) -> Result<(), StorageError> {
    sqlx::query(&format!("PRAGMA user_version = {version}"))
        .execute(&mut *connection)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("failed to set SQLite user_version to {version}: {e}"),
        })?;
    Ok(())
}

async fn table_has_columns(
    connection: &mut SqliteConnection,
    table: &str,
    required_columns: &[&str],
) -> Result<bool, StorageError> {
    let columns = table_columns(connection, table).await?;
    Ok(required_columns
        .iter()
        .all(|column| columns.contains(*column)))
}

async fn table_columns(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<BTreeSet<String>, StorageError> {
    let sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
    let rows: Vec<(String,)> = sqlx::query_as(&sql)
        .fetch_all(&mut *connection)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("failed to inspect SQLite columns for '{table}': {e}"),
        })?;

    Ok(rows.into_iter().map(|(name,)| name).collect())
}

async fn archive_incompatible_database(
    db_path: &Path,
    reason: &str,
) -> Result<PathBuf, StorageError> {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let backup_path = PathBuf::from(format!(
        "{}.incompatible-{timestamp}.bak",
        db_path.to_string_lossy()
    ));

    tokio::fs::rename(db_path, &backup_path)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!(
                "failed to archive incompatible database '{}' -> '{}': {e}",
                db_path.display(),
                backup_path.display()
            ),
        })?;

    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", db_path.to_string_lossy(), suffix));
        if sidecar.exists() {
            tokio::fs::remove_file(&sidecar)
                .await
                .map_err(|e| StorageError::Migration {
                    message: format!(
                        "failed to remove stale SQLite sidecar '{}': {e}",
                        sidecar.display()
                    ),
                })?;
        }
    }

    warn!(
        db_path = %db_path.display(),
        backup_path = %backup_path.display(),
        reason,
        "SQLite database is incompatible with the current schema; archived and recreating"
    );

    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::create_pool;

    const ACTIVE_TABLES: &[&str] = &[
        "session_metadata",
        "orchestrator_workflows",
        "orchestrator_checkpoints",
        "schedule_definitions",
        "schedule_executions",
        "chat_checkpoints",
        "chat_messages",
        "diag_traces",
        "diag_observations",
        "diag_scores",
        "provider_metrics_log",
    ];

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
        for table in ACTIVE_TABLES {
            assert!(
                names.contains(table),
                "expected table '{table}' missing, got: {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_prepare_database_resets_legacy_sqlx_schema() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("legacy.db");
        let config = StorageConfig {
            db_path: db_path.to_string_lossy().to_string(),
            pool_size: 1,
            wal_enabled: true,
            busy_timeout_ms: 5000,
            transcript_dir: temp_dir.path().join("transcripts"),
        };
        std::fs::create_dir_all(&config.transcript_dir).unwrap();

        let legacy_pool = create_pool(&config).await.unwrap();
        sqlx::raw_sql(
            r"
            CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL
            );
            INSERT INTO _sqlx_migrations (version, description)
            VALUES (1, 'initial sessions');

            CREATE TABLE session_metadata (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                root_id TEXT NOT NULL,
                depth INTEGER NOT NULL DEFAULT 0,
                path TEXT NOT NULL,
                session_type TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'active',
                agent_id TEXT,
                title TEXT,
                token_count INTEGER NOT NULL DEFAULT 0,
                message_count INTEGER NOT NULL DEFAULT 0,
                transcript_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
        ",
        )
        .execute(&legacy_pool)
        .await
        .unwrap();
        legacy_pool.close().await;

        prepare_database(&config).await.unwrap();

        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();

        let user_version: i64 = sqlx::query("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(user_version, CURRENT_SCHEMA_VERSION);

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%' \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();

        for table in ACTIVE_TABLES {
            assert!(
                names.contains(table),
                "expected table '{table}' after reset, got: {names:?}"
            );
        }

        assert!(
            !names.contains(&"_sqlx_migrations"),
            "legacy sqlx migration metadata should be removed"
        );
        assert!(
            !names.contains(&"file_journal_entries"),
            "unused file_journal_entries table should not be recreated"
        );
        assert!(
            !names.contains(&"tool_dynamic_definitions"),
            "unused tool_dynamic_definitions table should not be recreated"
        );

        let columns: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('session_metadata') ORDER BY cid")
                .fetch_all(&pool)
                .await
                .unwrap();
        let column_names: Vec<&str> = columns.iter().map(|c| c.0.as_str()).collect();
        for required in [
            "manual_title",
            "context_reset_index",
            "custom_system_prompt",
        ] {
            assert!(
                column_names.contains(&required),
                "session_metadata should include '{required}', got: {column_names:?}"
            );
        }

        pool.close().await;

        let backup_files: Vec<String> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("legacy.db.incompatible-") && name.ends_with(".bak"))
            .collect();
        assert_eq!(
            backup_files.len(),
            1,
            "expected one backup file after reset"
        );
    }
}
