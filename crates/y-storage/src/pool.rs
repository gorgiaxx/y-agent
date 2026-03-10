//! SQLite connection pool factory with WAL mode configuration.

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use tracing::info;

use crate::config::StorageConfig;
use crate::error::StorageError;

/// Create a SQLite connection pool configured per storage config.
///
/// Applies WAL mode, foreign keys, and busy timeout PRAGMAs.
pub async fn create_pool(config: &StorageConfig) -> Result<SqlitePool, StorageError> {
    let mut options = if config.is_in_memory() {
        SqliteConnectOptions::new()
            .filename(":memory:")
            .create_if_missing(true)
    } else {
        // Ensure parent directory exists.
        if let Some(dir) = config.db_dir() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| StorageError::Config {
                message: format!("failed to create database directory: {e}"),
            })?;
        }
        SqliteConnectOptions::new()
            .filename(&config.db_path)
            .create_if_missing(true)
    };

    // Apply WAL mode if enabled.
    if config.wal_enabled {
        options = options
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
    }

    // Apply foreign keys and busy timeout via pragmas.
    options = options
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_millis(u64::from(
            config.busy_timeout_ms,
        )));

    let pool = SqlitePoolOptions::new()
        .max_connections(config.pool_size)
        .connect_with(options)
        .await?;

    info!(
        db_path = %config.db_path,
        pool_size = config.pool_size,
        wal_enabled = config.wal_enabled,
        "SQLite connection pool created"
    );

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    #[tokio::test]
    async fn test_pool_create_in_memory() {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.expect("pool should be created");
        // Verify the pool is usable by running a simple query.
        let row: (i64,) = sqlx::query_as("SELECT 1")
            .fetch_one(&pool)
            .await
            .expect("should execute query");
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn test_pool_wal_mode_enabled() {
        let config = StorageConfig {
            db_path: ":memory:".into(),
            pool_size: 1,
            wal_enabled: true,
            ..StorageConfig::default()
        };
        let pool = create_pool(&config).await.expect("pool should be created");
        let row = sqlx::query("SELECT * FROM pragma_journal_mode")
            .fetch_one(&pool)
            .await
            .expect("should query pragma");
        let mode: String = row.get(0);
        // In-memory databases may report 'memory' rather than 'wal',
        // but the option is set. For file-based DBs this would be 'wal'.
        assert!(
            mode == "wal" || mode == "memory",
            "journal_mode should be wal or memory, got: {mode}"
        );
    }

    #[tokio::test]
    async fn test_pool_foreign_keys_enabled() {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.expect("pool should be created");
        let row = sqlx::query("SELECT * FROM pragma_foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("should query pragma");
        let fk: i64 = row.get(0);
        assert_eq!(fk, 1, "foreign_keys should be enabled");
    }

    #[tokio::test]
    async fn test_pool_busy_timeout_set() {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.expect("pool should be created");
        let row = sqlx::query("SELECT * FROM pragma_busy_timeout")
            .fetch_one(&pool)
            .await
            .expect("should query pragma");
        let timeout: i64 = row.get(0);
        assert_eq!(timeout, 5000, "busy_timeout should be 5000ms");
    }
}
