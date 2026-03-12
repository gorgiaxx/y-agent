//! `SQLite` migration runner using sqlx.

use std::path::Path;

use sqlx::SqlitePool;
use tracing::info;

use crate::error::StorageError;

/// Run all pending `SQLite` migrations from the given directory.
///
/// Uses sqlx's migration system with versioned `.up.sql` and `.down.sql` files.
pub async fn run_migrations(pool: &SqlitePool, migrations_dir: &Path) -> Result<(), StorageError> {
    let migrator = sqlx::migrate::Migrator::new(migrations_dir)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("failed to load migrations from {}: {e}", migrations_dir.display()),
        })?;

    migrator.run(pool).await.map_err(|e| StorageError::Migration {
        message: format!("migration execution failed: {e}"),
    })?;

    info!(
        migrations_dir = %migrations_dir.display(),
        "SQLite migrations completed"
    );

    Ok(())
}

/// Run all pending `SQLite` migrations using embedded migrations.
///
/// This is the preferred method as it embeds migrations into the binary,
/// removing the need for a migrations directory at runtime.
pub async fn run_embedded_migrations(pool: &SqlitePool) -> Result<(), StorageError> {
    sqlx::migrate!("../../migrations/sqlite")
        .run(pool)
        .await
        .map_err(|e| StorageError::Migration {
            message: format!("embedded migration execution failed: {e}"),
        })?;

    info!("Embedded SQLite migrations completed");

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

        // Check that the session_metadata table exists.
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' ORDER BY name",
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

        // Run migrations twice — second run should be a no-op.
        run_embedded_migrations(&pool).await.expect("first run");
        run_embedded_migrations(&pool).await.expect("second run should not fail");
    }

    #[tokio::test]
    async fn test_migration_version_tracking() {
        let pool = setup_pool_with_migrations().await;

        // sqlx stores migration metadata in _sqlx_migrations.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("should query migration table");
        assert!(
            count.0 >= 6,
            "should have at least 6 migrations recorded, got: {}",
            count.0
        );
    }
}
