//! `SQLite` persistence for replayable session events.

use std::collections::HashSet;

use sqlx::{FromRow, SqlitePool};
use y_core::session_event::{
    NewSessionEvent, PersistedSessionEvent, SessionEventKind, SessionEventRetention,
};
use y_core::types::SessionId;

use crate::StorageError;

#[derive(Clone)]
pub struct SqliteSessionEventStore {
    pool: SqlitePool,
}

impl SqliteSessionEventStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn append(
        &self,
        event: &NewSessionEvent,
    ) -> Result<PersistedSessionEvent, StorageError> {
        let payload = serde_json::to_string(&event.payload)?;
        let row: DbSessionEventRow = sqlx::query_as(
            r"INSERT INTO session_events
               (session_id, seq, kind, payload, retention_class, correlation_id)
               SELECT ?1, COALESCE(MAX(seq), 0) + 1, ?2, ?3, ?4, ?5
               FROM session_events
               WHERE session_id = ?1
               RETURNING event_id, session_id, seq, kind, payload,
                         retention_class, correlation_id, created_at",
        )
        .bind(event.session_id.as_str())
        .bind(event.kind.as_str())
        .bind(payload)
        .bind(event.retention.as_str())
        .bind(event.correlation_id.as_deref())
        .fetch_one(&self.pool)
        .await?;
        row.try_into()
    }

    pub async fn list_after_event_id(
        &self,
        event_id: u64,
        session_id: Option<&SessionId>,
        limit: usize,
    ) -> Result<Vec<PersistedSessionEvent>, StorageError> {
        let event_id = i64::try_from(event_id).map_err(|_| StorageError::Config {
            message: "session event cursor exceeds SQLite integer range".to_string(),
        })?;
        let limit = bounded_limit(limit);
        let rows: Vec<DbSessionEventRow> = if let Some(session_id) = session_id {
            sqlx::query_as(
                r"SELECT event_id, session_id, seq, kind, payload,
                          retention_class, correlation_id, created_at
                   FROM session_events
                   WHERE event_id > ?1 AND session_id = ?2
                   ORDER BY event_id ASC
                   LIMIT ?3",
            )
            .bind(event_id)
            .bind(session_id.as_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                r"SELECT event_id, session_id, seq, kind, payload,
                          retention_class, correlation_id, created_at
                   FROM session_events
                   WHERE event_id > ?1
                   ORDER BY event_id ASC
                   LIMIT ?2",
            )
            .bind(event_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn latest_event_id(&self) -> Result<u64, StorageError> {
        let event_id: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(event_id), 0) FROM session_events")
                .fetch_one(&self.pool)
                .await?;
        u64::try_from(event_id).map_err(invalid_row("event_id"))
    }

    pub async fn prune_short_lived_for_correlation(
        &self,
        session_id: &SessionId,
        correlation_id: &str,
        keep_latest: usize,
    ) -> Result<u64, StorageError> {
        let keep_latest = i64::try_from(keep_latest).map_err(|_| StorageError::Config {
            message: "short-lived event retention exceeds SQLite integer range".to_string(),
        })?;
        let result = sqlx::query(
            r"DELETE FROM session_events
               WHERE event_id IN (
                   SELECT event_id
                   FROM session_events
                   WHERE session_id = ?1
                     AND correlation_id = ?2
                     AND retention_class = 'short_lived'
                   ORDER BY event_id DESC
                   LIMIT -1 OFFSET ?3
               )",
        )
        .bind(session_id.as_str())
        .bind(correlation_id)
        .bind(keep_latest)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn latest_for_correlations(
        &self,
        session_id: &SessionId,
        correlation_ids: &[String],
    ) -> Result<Vec<PersistedSessionEvent>, StorageError> {
        if correlation_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: HashSet<&str> = correlation_ids.iter().map(String::as_str).collect();
        let rows: Vec<DbSessionEventRow> = sqlx::query_as(
            r"SELECT event_id, session_id, seq, kind, payload,
                      retention_class, correlation_id, created_at
               FROM session_events
               WHERE session_id = ?1 AND correlation_id IS NOT NULL
               ORDER BY event_id DESC",
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        let mut seen = HashSet::new();
        let mut matches = Vec::new();
        for row in rows {
            let Some(correlation_id) = row.correlation_id.as_deref() else {
                continue;
            };
            if wanted.contains(correlation_id) && seen.insert(correlation_id.to_string()) {
                matches.push(row.try_into()?);
            }
        }
        matches.sort_by_key(|event: &PersistedSessionEvent| event.event_id);
        Ok(matches)
    }
}

fn bounded_limit(limit: usize) -> i64 {
    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000)
}

#[derive(Debug, FromRow)]
struct DbSessionEventRow {
    event_id: i64,
    session_id: String,
    seq: i64,
    kind: String,
    payload: String,
    retention_class: String,
    correlation_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<DbSessionEventRow> for PersistedSessionEvent {
    type Error = StorageError;

    fn try_from(row: DbSessionEventRow) -> Result<Self, Self::Error> {
        Ok(Self {
            event_id: u64::try_from(row.event_id).map_err(invalid_row("event_id"))?,
            session_id: SessionId(row.session_id),
            seq: u64::try_from(row.seq).map_err(invalid_row("seq"))?,
            kind: SessionEventKind::parse(&row.kind).ok_or_else(|| StorageError::Database {
                message: format!("unknown session event kind '{}'", row.kind),
            })?,
            payload: serde_json::from_str(&row.payload)?,
            retention: SessionEventRetention::parse(&row.retention_class).ok_or_else(|| {
                StorageError::Database {
                    message: format!(
                        "unknown session event retention class '{}'",
                        row.retention_class
                    ),
                }
            })?,
            correlation_id: row.correlation_id,
            created_at: row.created_at,
        })
    }
}

fn invalid_row(field: &'static str) -> impl FnOnce(std::num::TryFromIntError) -> StorageError {
    move |error| StorageError::Database {
        message: format!("invalid session event {field}: {error}"),
    }
}
