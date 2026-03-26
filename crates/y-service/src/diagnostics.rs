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
        /// Name of the agent/trace that produced this entry (e.g. `"chat-turn"`,
        /// `"subagent:title-generator"`). Enables frontends to distinguish
        /// root agent entries from subagent entries.
        agent_name: String,
    },
    /// A tool execution event.
    ToolResult {
        name: String,
        success: bool,
        duration_ms: u64,
        input_preview: String,
        result_preview: String,
        timestamp: String,
        /// Name of the agent/trace that produced this entry.
        agent_name: String,
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
                    .and_then(serde_json::Value::as_u64)
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
                                agent_name: trace.name.clone(),
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
                                agent_name: trace.name.clone(),
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

    /// Fetch all subagent traces regardless of session, ordered by time.
    ///
    /// Returns diagnostics for traces whose name starts with `subagent:`.
    /// This is used by the global diagnostics view to display all subagent
    /// calls, whether they are associated with a specific session or with
    /// `Uuid::nil()` (session-independent operations).
    pub async fn get_subagent_history(
        store: Arc<dyn TraceStore>,
        limit: usize,
    ) -> Result<Vec<HistoricalEntry>, String> {
        // Fetch recent traces across all sessions and filter for subagent prefix.
        let all_traces = store
            .list_traces(None, None, limit * 10)
            .await
            .map_err(|e| format!("Failed to list traces: {e}"))?;

        let subagent_traces: Vec<_> = all_traces
            .into_iter()
            .filter(|t| t.name.starts_with("subagent:"))
            .take(limit)
            .collect();

        if subagent_traces.is_empty() {
            return Ok(Vec::new());
        }

        let trace_ids: Vec<uuid::Uuid> = subagent_traces.iter().map(|t| t.id).collect();
        let all_observations = store
            .get_observations_by_trace_ids(&trace_ids)
            .await
            .unwrap_or_default();

        let mut obs_by_trace: HashMap<uuid::Uuid, Vec<_>> = HashMap::new();
        for obs in all_observations {
            obs_by_trace.entry(obs.trace_id).or_default().push(obs);
        }

        let mut entries: Vec<(chrono::DateTime<chrono::Utc>, HistoricalEntry)> = Vec::new();

        for trace in &subagent_traces {
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
                    .and_then(serde_json::Value::as_u64)
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
                                agent_name: trace.name.clone(),
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
                                agent_name: trace.name.clone(),
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
// DiagnosticsAgentDelegator -- decorator for tracing subagent LLM calls
// ---------------------------------------------------------------------------

// Task-local trace ID set by `DiagnosticsAgentDelegator` so that
// `ServiceAgentRunner` can pick it up and forward it into
// `AgentExecutionConfig.external_trace_id`. This avoids changing the
// `AgentDelegator` trait signature.
tokio::task_local! {
    pub static SUBAGENT_TRACE_ID: uuid::Uuid;
}

// Task-local progress sender set by callers (e.g. `skill_import`) so that
// `ServiceAgentRunner` can forward real-time `TurnEvent`s during subagent
// execution. When present, `AgentService::execute()` emits per-iteration
// LLM response and tool-call events through this channel.
tokio::task_local! {
    pub static SUBAGENT_PROGRESS: crate::chat::TurnEventSender;
}

/// A decorator around `AgentDelegator` that records diagnostics (trace +
/// generation observation) for each delegation call.
///
/// Creates a `subagent:<name>` trace and propagates its ID via the
/// `SUBAGENT_TRACE_ID` task-local so that `ServiceAgentRunner` can
/// forward it to `AgentService::execute()`. The execute loop then
/// records per-iteration generation and tool-call observations under
/// this trace, giving full visibility in the diagnostics panel.
///
/// When a `session_id` is provided by the caller, the subagent trace is
/// associated with that session so it appears in session-level diagnostics.
/// When `None`, uses `Uuid::nil()` (global-only visibility).
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
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl y_core::agent::AgentDelegator for DiagnosticsAgentDelegator {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: y_core::agent::ContextStrategyHint,
        session_id: Option<uuid::Uuid>,
    ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
        // Start a trace for this subagent execution.
        let trace_session_id = session_id.unwrap_or(uuid::Uuid::nil());
        let trace_name = format!("subagent:{agent_name}");
        let full_input = match &input {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // Keep the full input JSON for trace metadata (input is moved into delegate).
        let input_json = input.clone();
        let trace_id = self
            .diagnostics
            .on_trace_start(trace_session_id, &trace_name, &full_input)
            .await
            .ok();

        // Delegate to the inner delegator, propagating the trace_id via
        // task-local so ServiceAgentRunner can forward it.
        let result = if let Some(tid) = trace_id {
            SUBAGENT_TRACE_ID
                .scope(tid, async {
                    self.inner
                        .delegate(agent_name, input, context_strategy, session_id)
                        .await
                })
                .await
        } else {
            self.inner
                .delegate(agent_name, input, context_strategy, session_id)
                .await
        };

        // Store the full delegation input in the trace metadata so
        // diagnostics consumers can inspect the complete request body.
        if let Some(tid) = trace_id {
            if let Ok(mut trace) = self.diagnostics.store().get_trace(tid).await {
                if let serde_json::Value::Object(ref mut map) = trace.metadata {
                    map.insert("input".to_string(), input_json);
                } else {
                    trace.metadata = serde_json::json!({ "input": input_json });
                }
                let _ = self.diagnostics.store().update_trace(trace).await;
            }
        }

        // Close the trace.  Per-iteration generation and tool-call
        // observations have already been recorded by AgentService::execute()
        // under this same trace_id (via external_trace_id).
        match &result {
            Ok(output) => {
                if let Some(tid) = trace_id {
                    let _ = self
                        .diagnostics
                        .on_trace_end(tid, true, Some(&output.text))
                        .await;
                }
            }
            Err(_) => {
                if let Some(tid) = trace_id {
                    let _ = self.diagnostics.on_trace_end(tid, false, None).await;
                }
            }
        }

        result
    }
}
