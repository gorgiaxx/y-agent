//! Diagnostics query service.
//!
//! Wraps trace store queries so all frontends get consistent data
//! without importing `y-diagnostics` directly.

use std::sync::Arc;

use y_core::provider::ProviderPool;
use y_diagnostics::{TraceSearch, TraceSearchQuery, TraceStore};

use crate::container::ServiceContainer;

/// System health report returned by [`DiagnosticsService::health_check`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthCheckResult {
    /// Whether the trace store is reachable.
    pub trace_store_ok: bool,
    /// Number of traces in the last 30 days.
    pub recent_trace_count: usize,
    /// Active providers (not frozen).
    pub active_providers: usize,
    /// Frozen providers.
    pub frozen_providers: usize,
    /// Whether the PG feature is compiled in.
    pub pg_feature_enabled: bool,
}

/// Diagnostics query service.
pub struct DiagnosticsService;

impl DiagnosticsService {
    /// Search traces using a query.
    pub async fn search_traces(
        store: Arc<dyn TraceStore>,
        query: &TraceSearchQuery,
    ) -> Result<Vec<y_diagnostics::Trace>, String> {
        let search = TraceSearch::new(store);
        search.search(query).await.map_err(|e| format!("{e}"))
    }

    /// Get trace detail with observations.
    pub async fn get_trace(
        store: Arc<dyn TraceStore>,
        trace_id: uuid::Uuid,
    ) -> Result<y_diagnostics::Trace, String> {
        store.get_trace(trace_id).await.map_err(|e| format!("{e}"))
    }

    /// Get observations for a trace.
    pub async fn get_observations(
        store: Arc<dyn TraceStore>,
        trace_id: uuid::Uuid,
    ) -> Result<Vec<y_diagnostics::Observation>, String> {
        store
            .get_observations(trace_id)
            .await
            .map_err(|e| format!("{e}"))
    }

    /// System health check.
    pub async fn health_check(container: &ServiceContainer) -> HealthCheckResult {
        let store = container.diagnostics.store();

        let trace_store_ok = store.list_traces(None, None, 1).await.is_ok();

        let recent_trace_count = if trace_store_ok {
            let since = chrono::Utc::now() - chrono::Duration::days(30);
            store
                .list_traces(None, Some(since), 10_000)
                .await
                .map(|t| t.len())
                .unwrap_or(0)
        } else {
            0
        };

        let statuses = container.provider_pool().await.provider_statuses().await;
        let active = statuses.iter().filter(|s| !s.is_frozen).count();

        HealthCheckResult {
            trace_store_ok,
            recent_trace_count,
            active_providers: active,
            frozen_providers: statuses.len() - active,
            pg_feature_enabled: cfg!(feature = "diagnostics_pg"),
        }
    }
}
