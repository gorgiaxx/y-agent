//! Diagnostics query service.
//!
//! Wraps trace store queries so all frontends get consistent data
//! without importing `y-diagnostics` directly.

use std::collections::HashMap;
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
}

/// A single historical diagnostic entry.
///
/// Used by frontends to display a chronological timeline of LLM calls
/// and tool executions within a session.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoricalEntry {
    /// An LLM generation event.
    LlmResponse {
        iteration: usize,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
        cost_usd: f64,
        tool_calls_requested: Vec<String>,
        prompt_preview: String,
        response_text: String,
        timestamp: String,
    },
    /// A tool execution event.
    ToolResult {
        name: String,
        success: bool,
        duration_ms: u64,
        input_preview: String,
        result_preview: String,
        timestamp: String,
    },
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

    /// Fetch historical diagnostics for a session, ordered by time.
    ///
    /// Returns a flat list of [`HistoricalEntry`] values reconstructed from
    /// stored Traces and Observations. Limited to the `limit` most recent
    /// traces so the result does not grow unbounded for long-lived sessions.
    pub async fn get_session_history(
        store: Arc<dyn TraceStore>,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<HistoricalEntry>, String> {
        let traces = store
            .list_traces_by_session(session_id, limit)
            .await
            .map_err(|e| format!("Failed to list traces: {e}"))?;

        let trace_ids: Vec<uuid::Uuid> = traces.iter().map(|t| t.id).collect();
        let all_observations = store
            .get_observations_by_trace_ids(&trace_ids)
            .await
            .unwrap_or_default();

        let mut obs_by_trace: HashMap<uuid::Uuid, Vec<_>> = HashMap::new();
        for obs in all_observations {
            obs_by_trace.entry(obs.trace_id).or_default().push(obs);
        }

        let mut entries: Vec<(chrono::DateTime<chrono::Utc>, HistoricalEntry)> = Vec::new();

        for trace in &traces {
            let mut obs_sorted = obs_by_trace.remove(&trace.id).unwrap_or_default();
            obs_sorted.sort_by(|a, b| {
                a.sequence
                    .cmp(&b.sequence)
                    .then(a.started_at.cmp(&b.started_at))
            });

            let mut llm_iter = 0usize;

            for obs in &obs_sorted {
                let ts = obs.completed_at.unwrap_or(obs.started_at);
                let duration_ms = obs
                    .metadata
                    .get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                match obs.obs_type {
                    y_diagnostics::ObservationType::Generation => {
                        llm_iter += 1;
                        let model = obs.model.clone().unwrap_or_default();

                        let prompt_preview = if obs.input.is_null() {
                            trace
                                .user_input
                                .as_deref()
                                .unwrap_or("(input not captured)")
                                .to_string()
                        } else {
                            obs.input.to_string()
                        };

                        let response_text = if obs.output.is_null() {
                            trace
                                .metadata
                                .get("output")
                                .and_then(|v| v.as_str())
                                .unwrap_or("(output not captured)")
                                .to_string()
                        } else {
                            obs.output.to_string()
                        };

                        entries.push((
                            ts,
                            HistoricalEntry::LlmResponse {
                                iteration: llm_iter,
                                model,
                                input_tokens: obs.input_tokens,
                                output_tokens: obs.output_tokens,
                                duration_ms,
                                cost_usd: obs.cost_usd,
                                tool_calls_requested: vec![],
                                prompt_preview,
                                response_text,
                                timestamp: ts.to_rfc3339(),
                            },
                        ));
                    }
                    y_diagnostics::ObservationType::ToolCall => {
                        let success = obs.status != y_diagnostics::ObservationStatus::Failed;
                        let result_preview = obs.output.to_string();
                        let input_preview = if obs.input.is_null() {
                            String::new()
                        } else {
                            obs.input.to_string()
                        };

                        entries.push((
                            ts,
                            HistoricalEntry::ToolResult {
                                name: obs.name.clone(),
                                success,
                                duration_ms,
                                input_preview,
                                result_preview,
                                timestamp: ts.to_rfc3339(),
                            },
                        ));
                    }
                    _ => {}
                }
            }
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries.into_iter().map(|(_, e)| e).collect())
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
        }
    }
}

// ---------------------------------------------------------------------------
// DiagnosticsAgentDelegator — decorator for tracing subagent LLM calls
// ---------------------------------------------------------------------------

/// A decorator around [`AgentDelegator`] that records diagnostics (trace +
/// generation observation) for each delegation call.
///
/// Without this wrapper, subagent calls (title-generator, skill-ingestion,
/// etc.) are invisible in the DIAGNOSTICS panel because `AgentPool::delegate()`
/// calls `SingleTurnRunner::run()` which bypasses the diagnostics subscriber.
///
/// Uses `Uuid::nil()` as the session UUID since subagent calls are not
/// associated with any specific user session. The frontend's Global view
/// merges all sessions' entries, so these will appear there.
pub struct DiagnosticsAgentDelegator {
    inner: Arc<dyn y_core::agent::AgentDelegator>,
    diagnostics: Arc<y_diagnostics::DiagnosticsSubscriber<dyn y_diagnostics::TraceStore>>,
}

impl DiagnosticsAgentDelegator {
    /// Create a new diagnostics-aware delegator wrapping `inner`.
    pub fn new(
        inner: Arc<dyn y_core::agent::AgentDelegator>,
        diagnostics: Arc<y_diagnostics::DiagnosticsSubscriber<dyn y_diagnostics::TraceStore>>,
    ) -> Self {
        Self { inner, diagnostics }
    }
}

impl std::fmt::Debug for DiagnosticsAgentDelegator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiagnosticsAgentDelegator")
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait::async_trait]
impl y_core::agent::AgentDelegator for DiagnosticsAgentDelegator {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: y_core::agent::ContextStrategyHint,
    ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
        // Start a trace for this subagent execution.
        let session_id = uuid::Uuid::nil();
        let trace_name = format!("subagent:{agent_name}");
        let input_preview = match &input {
            serde_json::Value::String(s) => s.chars().take(200).collect::<String>(),
            other => {
                let s = other.to_string();
                s.chars().take(200).collect::<String>()
            }
        };
        let trace_id = self
            .diagnostics
            .on_trace_start(session_id, &trace_name, &input_preview)
            .await
            .ok();

        // Delegate to the inner delegator.
        let result = self
            .inner
            .delegate(agent_name, input, context_strategy)
            .await;

        match &result {
            Ok(output) => {
                // Record the generation observation.
                if let Some(tid) = trace_id {
                    let _ = self
                        .diagnostics
                        .on_generation(
                            tid,
                            None,
                            None,
                            &output.model_used,
                            output.input_tokens,
                            output.output_tokens,
                            crate::cost::CostService::compute_cost(
                                output.input_tokens,
                                output.output_tokens,
                            ),
                            serde_json::json!({
                                "agent": agent_name,
                                "type": "subagent_delegation",
                            }),
                            serde_json::from_str::<serde_json::Value>(&output.text)
                                .unwrap_or_else(|_| serde_json::Value::String(output.text.clone())),
                            output.duration_ms,
                        )
                        .await;

                    let _ = self
                        .diagnostics
                        .on_trace_end(tid, true, Some(&output.text))
                        .await;
                }
            }
            Err(_) => {
                // Mark the trace as failed.
                if let Some(tid) = trace_id {
                    let _ = self.diagnostics.on_trace_end(tid, false, None).await;
                }
            }
        }

        result
    }
}
