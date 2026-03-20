//! Provider metrics event log store -- persists per-request metrics for
//! historical observability queries.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::error::StorageError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single provider request event to be persisted.
#[derive(Debug, Clone)]
pub struct ProviderMetricsEvent {
    pub provider_id: String,
    pub model: String,
    pub is_error: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micros: u64,
}

/// Aggregated metrics for a provider over a time range.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AggregatedProviderMetrics {
    pub provider_id: String,
    pub model: String,
    pub total_requests: u64,
    pub total_errors: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_micros: u64,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// SQLite-backed provider metrics event log.
///
/// Records individual request events and provides time-range aggregation
/// queries for the observability panel.
#[derive(Debug, Clone)]
pub struct SqliteProviderMetricsStore {
    pool: SqlitePool,
}

impl SqliteProviderMetricsStore {
    /// Create a new store backed by the given connection pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Record a single provider request event.
    pub async fn record_event(&self, event: &ProviderMetricsEvent) -> Result<(), StorageError> {
        let event_type = if event.is_error { "error" } else { "success" };
        let input_tokens = i64::try_from(event.input_tokens).unwrap_or(i64::MAX);
        let output_tokens = i64::try_from(event.output_tokens).unwrap_or(i64::MAX);
        let cost_micros = i64::try_from(event.cost_micros).unwrap_or(i64::MAX);

        sqlx::query(
            "INSERT INTO provider_metrics_log \
             (provider_id, model, event_type, input_tokens, output_tokens, cost_micros) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.provider_id)
        .bind(&event.model)
        .bind(event_type)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cost_micros)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("failed to record metrics event: {e}"),
        })?;

        Ok(())
    }

    /// Query aggregated metrics grouped by provider, optionally filtered by time range.
    ///
    /// When both `since` and `until` are `None`, returns all-time aggregates.
    pub async fn query_aggregated(
        &self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<AggregatedProviderMetrics>, StorageError> {
        let since_str = since.map(|dt| dt.to_rfc3339());
        let until_str = until.map(|dt| dt.to_rfc3339());

        let rows: Vec<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT \
                 provider_id, \
                 model, \
                 COUNT(*) as total_requests, \
                 SUM(CASE WHEN event_type = 'error' THEN 1 ELSE 0 END) as total_errors, \
                 SUM(input_tokens) as total_input_tokens, \
                 SUM(output_tokens) as total_output_tokens, \
                 SUM(cost_micros) as total_cost_micros \
             FROM provider_metrics_log \
             WHERE (? IS NULL OR recorded_at >= ?) \
               AND (? IS NULL OR recorded_at <= ?) \
             GROUP BY provider_id \
             ORDER BY provider_id",
        )
        .bind(&since_str)
        .bind(&since_str)
        .bind(&until_str)
        .bind(&until_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::Database {
            message: format!("failed to query aggregated metrics: {e}"),
        })?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    provider_id,
                    model,
                    total_requests,
                    total_errors,
                    input_tok,
                    output_tok,
                    cost,
                )| {
                    AggregatedProviderMetrics {
                        provider_id,
                        model,
                        total_requests: u64::try_from(total_requests).unwrap_or(0),
                        total_errors: u64::try_from(total_errors).unwrap_or(0),
                        total_input_tokens: u64::try_from(input_tok).unwrap_or(0),
                        total_output_tokens: u64::try_from(output_tok).unwrap_or(0),
                        total_cost_micros: u64::try_from(cost).unwrap_or(0),
                    }
                },
            )
            .collect())
    }

    /// Delete all events recorded before the given timestamp.
    ///
    /// Returns the number of rows deleted.
    pub async fn cleanup_before(&self, before: DateTime<Utc>) -> Result<u64, StorageError> {
        let before_str = before.to_rfc3339();
        let result = sqlx::query("DELETE FROM provider_metrics_log WHERE recorded_at < ?")
            .bind(&before_str)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Database {
                message: format!("failed to cleanup old metrics: {e}"),
            })?;

        Ok(result.rows_affected())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_embedded_migrations;

    async fn setup() -> SqliteProviderMetricsStore {
        let config = crate::config::StorageConfig::in_memory();
        let pool = crate::pool::create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        SqliteProviderMetricsStore::new(pool)
    }

    #[tokio::test]
    async fn test_record_and_query_aggregated() {
        let store = setup().await;

        // Record 2 successes + 1 error for provider A.
        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "provider-a".into(),
                model: "gpt-4".into(),
                is_error: false,
                input_tokens: 100,
                output_tokens: 50,
                cost_micros: 5000,
            })
            .await
            .unwrap();

        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "provider-a".into(),
                model: "gpt-4".into(),
                is_error: false,
                input_tokens: 200,
                output_tokens: 100,
                cost_micros: 10000,
            })
            .await
            .unwrap();

        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "provider-a".into(),
                model: "gpt-4".into(),
                is_error: true,
                input_tokens: 0,
                output_tokens: 0,
                cost_micros: 0,
            })
            .await
            .unwrap();

        // Record 1 success for provider B.
        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "provider-b".into(),
                model: "claude-3".into(),
                is_error: false,
                input_tokens: 300,
                output_tokens: 150,
                cost_micros: 20000,
            })
            .await
            .unwrap();

        // Query all.
        let results = store.query_aggregated(None, None).await.unwrap();
        assert_eq!(results.len(), 2);

        let a = &results[0];
        assert_eq!(a.provider_id, "provider-a");
        assert_eq!(a.total_requests, 3);
        assert_eq!(a.total_errors, 1);
        assert_eq!(a.total_input_tokens, 300);
        assert_eq!(a.total_output_tokens, 150);
        assert_eq!(a.total_cost_micros, 15000);

        let b = &results[1];
        assert_eq!(b.provider_id, "provider-b");
        assert_eq!(b.total_requests, 1);
        assert_eq!(b.total_errors, 0);
        assert_eq!(b.total_input_tokens, 300);
        assert_eq!(b.total_output_tokens, 150);
        assert_eq!(b.total_cost_micros, 20000);
    }

    #[tokio::test]
    async fn test_query_with_time_range() {
        let store = setup().await;

        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "p1".into(),
                model: "m1".into(),
                is_error: false,
                input_tokens: 10,
                output_tokens: 5,
                cost_micros: 100,
            })
            .await
            .unwrap();

        // Query with future `since` -- should return nothing.
        let future = Utc::now() + chrono::Duration::hours(1);
        let results = store.query_aggregated(Some(future), None).await.unwrap();
        assert!(results.is_empty());

        // Query with past `since` -- should return the event.
        let past = Utc::now() - chrono::Duration::hours(1);
        let results = store.query_aggregated(Some(past), None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].total_requests, 1);
    }

    #[tokio::test]
    async fn test_cleanup_before() {
        let store = setup().await;

        store
            .record_event(&ProviderMetricsEvent {
                provider_id: "p1".into(),
                model: "m1".into(),
                is_error: false,
                input_tokens: 10,
                output_tokens: 5,
                cost_micros: 100,
            })
            .await
            .unwrap();

        // Cleanup with future cutoff -- should delete everything.
        let future = Utc::now() + chrono::Duration::hours(1);
        let deleted = store.cleanup_before(future).await.unwrap();
        assert_eq!(deleted, 1);

        // Verify empty.
        let results = store.query_aggregated(None, None).await.unwrap();
        assert!(results.is_empty());
    }
}
