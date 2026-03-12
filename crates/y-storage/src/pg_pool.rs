//! Feature-gated `PostgreSQL` connection pool.
//!
//! This module is only compiled when the `diagnostics_pg` feature is enabled.

#[cfg(feature = "diagnostics_pg")]
mod inner {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::PgPool;

    use crate::config::StorageConfig;
    use crate::error::StorageError;

    /// Create a `PostgreSQL` connection pool from the storage configuration.
    ///
    /// # Errors
    ///
    /// Returns `StorageError::Config` if `postgres_url` is not set,
    /// or `StorageError::Connection` if the pool cannot be created.
    pub async fn create_pg_pool(config: &StorageConfig) -> Result<PgPool, StorageError> {
        let url = config.postgres_url.as_deref().ok_or(StorageError::Config {
            message: "postgres_url must be set when diagnostics_pg feature is enabled".into(),
        })?;

        let connect_options: PgConnectOptions =
            url.parse().map_err(|e: sqlx::Error| StorageError::Config {
                message: format!("invalid postgres_url: {e}"),
            })?;

        let pool = PgPoolOptions::new()
            .max_connections(config.pg_pool_size)
            .connect_with(connect_options)
            .await
            .map_err(|e| StorageError::Connection {
                message: format!("PostgreSQL connection failed: {e}"),
            })?;

        Ok(pool)
    }
}

#[cfg(feature = "diagnostics_pg")]
pub use inner::create_pg_pool;
