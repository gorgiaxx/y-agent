//! SQLite-backed runtime instance registration and fenced leases.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

use crate::StorageError;

const DEFAULT_STALE_INSTANCE_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Registration data for one running y-agent process.
#[derive(Debug, Clone)]
pub struct RuntimeInstanceRegistration {
    /// Stable identifier generated once for the process lifetime.
    pub instance_id: String,
    /// Operating-system process identifier for diagnostics only.
    pub process_id: u32,
    /// Presentation/runtime kind such as `cli`, `tui`, `gui`, or `web`.
    pub runtime_kind: String,
    /// Bounded diagnostic metadata. It must not contain secrets or user content.
    pub metadata: Value,
}

/// A fenced lease over one logical runtime resource.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RuntimeLease {
    /// Resource namespace, for example `singleton` or `session`.
    pub resource_kind: String,
    /// Identifier within the resource namespace.
    pub resource_id: String,
    /// Process instance that currently owns the lease. The lease may outlive its diagnostic row.
    pub owner_instance_id: String,
    /// Monotonic token that changes whenever an expired lease is acquired again.
    pub fencing_token: i64,
    /// Database-clock expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Cross-process coordination primitives backed by one `SQLite` database.
#[derive(Debug, Clone)]
pub struct SqliteCoordinationStore {
    pool: SqlitePool,
}

impl SqliteCoordinationStore {
    /// Create a coordination store over an existing application pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Register or refresh one process instance.
    pub async fn register_instance(
        &self,
        registration: &RuntimeInstanceRegistration,
    ) -> Result<(), StorageError> {
        validate_identifier("instance_id", &registration.instance_id)?;
        validate_identifier("runtime_kind", &registration.runtime_kind)?;
        let metadata = serde_json::to_string(&registration.metadata)?;

        sqlx::query(
            r"INSERT INTO runtime_instances
                 (instance_id, process_id, runtime_kind, metadata)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(instance_id) DO UPDATE SET
                 process_id = excluded.process_id,
                 runtime_kind = excluded.runtime_kind,
                 metadata = excluded.metadata,
                 heartbeat_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&registration.instance_id)
        .bind(i64::from(registration.process_id))
        .bind(&registration.runtime_kind)
        .bind(metadata)
        .execute(&self.pool)
        .await?;
        self.prune_stale_instances(DEFAULT_STALE_INSTANCE_RETENTION)
            .await?;
        Ok(())
    }

    /// Refresh the liveness timestamp for a registered process instance.
    pub async fn heartbeat_instance(&self, instance_id: &str) -> Result<bool, StorageError> {
        validate_identifier("instance_id", instance_id)?;
        let result = sqlx::query(
            r"UPDATE runtime_instances SET
                 heartbeat_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               WHERE instance_id = ?1",
        )
        .bind(instance_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Delete diagnostic registrations older than `retention` unless they own a live lease.
    ///
    /// Expired lease rows remain intact so their fencing tokens stay monotonic.
    pub async fn prune_stale_instances(&self, retention: Duration) -> Result<u64, StorageError> {
        if retention.is_zero() {
            return Err(StorageError::Config {
                message: "stale instance retention must be greater than zero".to_string(),
            });
        }
        let retention_modifier = negative_ttl_modifier(retention);
        let result = sqlx::query(
            r"DELETE FROM runtime_instances
               WHERE heartbeat_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1)
                 AND NOT EXISTS (
                   SELECT 1 FROM runtime_leases
                   WHERE runtime_leases.owner_instance_id = runtime_instances.instance_id
                     AND runtime_leases.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 )",
        )
        .bind(retention_modifier)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Atomically acquire a lease, returning `None` while another live owner holds it.
    ///
    /// Re-acquisition by the current live owner renews the expiry without changing
    /// the fencing token. Acquisition after expiry increments the token, including
    /// when the same process instance re-acquires its old lease.
    pub async fn try_acquire_lease(
        &self,
        resource_kind: &str,
        resource_id: &str,
        owner_instance_id: &str,
        ttl: Duration,
    ) -> Result<Option<RuntimeLease>, StorageError> {
        validate_lease_input(resource_kind, resource_id, owner_instance_id, ttl)?;
        let ttl_modifier = ttl_modifier(ttl);

        let lease = sqlx::query_as(
            r"INSERT INTO runtime_leases
                 (resource_kind, resource_id, owner_instance_id, fencing_token, expires_at)
               VALUES (
                 ?1, ?2, ?3, 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?4)
               )
               ON CONFLICT(resource_kind, resource_id) DO UPDATE SET
                 owner_instance_id = excluded.owner_instance_id,
                 fencing_token = CASE
                   WHEN runtime_leases.owner_instance_id = excluded.owner_instance_id
                     AND runtime_leases.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                   THEN runtime_leases.fencing_token
                   ELSE runtime_leases.fencing_token + 1
                 END,
                 expires_at = excluded.expires_at,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               WHERE runtime_leases.owner_instance_id = excluded.owner_instance_id
                  OR runtime_leases.expires_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               RETURNING resource_kind, resource_id, owner_instance_id,
                         fencing_token, expires_at",
        )
        .bind(resource_kind)
        .bind(resource_id)
        .bind(owner_instance_id)
        .bind(ttl_modifier)
        .fetch_optional(&self.pool)
        .await?;
        Ok(lease)
    }

    /// Renew a currently live lease when its owner and fencing token still match.
    pub async fn renew_lease(
        &self,
        lease: &RuntimeLease,
        ttl: Duration,
    ) -> Result<Option<RuntimeLease>, StorageError> {
        validate_lease_input(
            &lease.resource_kind,
            &lease.resource_id,
            &lease.owner_instance_id,
            ttl,
        )?;
        let ttl_modifier = ttl_modifier(ttl);

        let renewed = sqlx::query_as(
            r"UPDATE runtime_leases SET
                 expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               WHERE resource_kind = ?2
                 AND resource_id = ?3
                 AND owner_instance_id = ?4
                 AND fencing_token = ?5
                 AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               RETURNING resource_kind, resource_id, owner_instance_id,
                         fencing_token, expires_at",
        )
        .bind(ttl_modifier)
        .bind(&lease.resource_kind)
        .bind(&lease.resource_id)
        .bind(&lease.owner_instance_id)
        .bind(lease.fencing_token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(renewed)
    }

    /// Expire a live lease when its owner and fencing token still match.
    ///
    /// The row is retained so future owners always receive a larger fencing token.
    pub async fn release_lease(&self, lease: &RuntimeLease) -> Result<bool, StorageError> {
        let result = sqlx::query(
            r"UPDATE runtime_leases SET
                 expires_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               WHERE resource_kind = ?1
                 AND resource_id = ?2
                 AND owner_instance_id = ?3
                 AND fencing_token = ?4
                 AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(&lease.resource_kind)
        .bind(&lease.resource_id)
        .bind(&lease.owner_instance_id)
        .bind(lease.fencing_token)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}

fn validate_lease_input(
    resource_kind: &str,
    resource_id: &str,
    owner_instance_id: &str,
    ttl: Duration,
) -> Result<(), StorageError> {
    validate_identifier("resource_kind", resource_kind)?;
    validate_identifier("resource_id", resource_id)?;
    validate_identifier("owner_instance_id", owner_instance_id)?;
    if ttl.is_zero() {
        return Err(StorageError::Config {
            message: "lease ttl must be greater than zero".to_string(),
        });
    }
    Ok(())
}

fn validate_identifier(field: &str, value: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::Config {
            message: format!("{field} must not be empty"),
        });
    }
    Ok(())
}

fn ttl_modifier(ttl: Duration) -> String {
    duration_modifier('+', ttl)
}

fn negative_ttl_modifier(ttl: Duration) -> String {
    duration_modifier('-', ttl)
}

fn duration_modifier(sign: char, duration: Duration) -> String {
    let milliseconds = duration.as_millis().max(1);
    format!(
        "{sign}{}.{:03} seconds",
        milliseconds / 1_000,
        milliseconds % 1_000
    )
}
