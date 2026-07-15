//! Diagnostics query service.
//!
//! Wraps trace store queries so all frontends get consistent data
//! without importing `y-diagnostics` directly.

use std::collections::HashMap;
use std::sync::Arc;

use y_core::provider::ProviderPool;
use y_diagnostics::{TraceSearch, TraceSearchQuery, TraceStore};

use super::adaptation_metrics;
use super::{DynamicAgentRegressionFinding, DynamicAgentVersionMetrics, OrchestrationModeMetrics};
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
        /// Prompt tokens served from cache (subset of total input).
        cache_read_tokens: u64,
        /// Prompt tokens written to cache (subset of total input).
        cache_write_tokens: u64,
        /// Total prompt tokens processed (fresh + cache). Authoritative
        /// context-window occupancy figure.
        context_tokens_used: u64,
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
    /// Aggregate completed trace outcomes by the orchestration mode recorded
    /// in `trace.metadata.orchestration.selected_mode`.
    pub async fn orchestration_mode_metrics(
        store: Arc<dyn TraceStore>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: usize,
    ) -> Result<Vec<OrchestrationModeMetrics>, String> {
        let traces = store
            .list_traces(None, since, limit)
            .await
            .map_err(|error| format!("Failed to list orchestration traces: {error}"))?;
        Ok(adaptation_metrics::orchestration_mode_metrics(traces))
    }

    /// Aggregate durable outcomes by dynamic-agent ID and version.
    pub async fn dynamic_agent_version_metrics(
        store: Arc<dyn TraceStore>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: usize,
    ) -> Result<Vec<DynamicAgentVersionMetrics>, String> {
        let traces = store
            .list_traces(None, since, limit)
            .await
            .map_err(|error| format!("Failed to list dynamic-agent traces: {error}"))?;
        Ok(adaptation_metrics::dynamic_agent_version_metrics(traces))
    }

    /// Detect adjacent dynamic-agent versions with enough repeated evidence
    /// and a success-rate drop at or above the configured threshold.
    pub async fn dynamic_agent_regressions(
        store: Arc<dyn TraceStore>,
        since: Option<chrono::DateTime<chrono::Utc>>,
        limit: usize,
        min_samples: usize,
        max_success_rate_drop: f64,
    ) -> Result<Vec<DynamicAgentRegressionFinding>, String> {
        let metrics = Self::dynamic_agent_version_metrics(store, since, limit).await?;
        Ok(adaptation_metrics::dynamic_agent_regressions(
            &metrics,
            min_samples.max(1),
            max_success_rate_drop.clamp(0.0, 1.0),
        ))
    }

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
        Self::get_history_for_session_ids(store, &[session_id.to_string()], limit).await
    }

    /// Fetch historical diagnostics for a session and all descendant sessions.
    pub async fn get_session_history_including_descendants(
        container: &ServiceContainer,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<HistoricalEntry>, String> {
        let session_id = y_core::types::SessionId::from_string(session_id);
        let nodes = container
            .session_manager
            .descendants_including_self(&session_id)
            .await
            .map_err(|e| format!("Failed to collect descendant sessions: {e}"))?;
        let session_ids: Vec<String> = nodes.into_iter().map(|node| node.id.to_string()).collect();

        Self::get_history_for_session_ids(container.diagnostics.store(), &session_ids, limit).await
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

        Self::build_history_entries(store, subagent_traces).await
    }

    /// Delete diagnostics history for a session and all descendant sessions.
    pub async fn clear_session_history_including_descendants(
        container: &ServiceContainer,
        session_id: &str,
    ) -> Result<u64, String> {
        let session_id = y_core::types::SessionId::from_string(session_id);
        let nodes = container
            .session_manager
            .descendants_including_self(&session_id)
            .await
            .map_err(|e| format!("Failed to collect descendant sessions: {e}"))?;
        let session_ids: Vec<String> = nodes.into_iter().map(|node| node.id.to_string()).collect();

        Self::clear_history_for_session_ids(container.diagnostics.store(), &session_ids).await
    }

    /// Delete every diagnostics trace and related row.
    pub async fn clear_all_history(store: Arc<dyn TraceStore>) -> Result<u64, String> {
        store
            .delete_all_traces()
            .await
            .map_err(|e| format!("Failed to delete diagnostics history: {e}"))
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

    async fn get_history_for_session_ids(
        store: Arc<dyn TraceStore>,
        session_ids: &[String],
        limit: usize,
    ) -> Result<Vec<HistoricalEntry>, String> {
        let mut traces = Vec::new();
        let mut seen_trace_ids = std::collections::HashSet::new();

        for session_id in session_ids {
            let session_traces = store
                .list_traces_by_session(session_id, limit)
                .await
                .map_err(|e| format!("Failed to list traces: {e}"))?;
            for trace in session_traces {
                if seen_trace_ids.insert(trace.id) {
                    traces.push(trace);
                }
            }
        }

        traces.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        if traces.len() > limit {
            let keep_from = traces.len().saturating_sub(limit);
            traces = traces.split_off(keep_from);
        }

        Self::build_history_entries(store, traces).await
    }

    async fn clear_history_for_session_ids(
        store: Arc<dyn TraceStore>,
        session_ids: &[String],
    ) -> Result<u64, String> {
        let mut deleted = 0;
        for session_id in session_ids {
            deleted += store
                .delete_traces_by_session(session_id)
                .await
                .map_err(|e| format!("Failed to delete traces for session {session_id}: {e}"))?;
        }
        Ok(deleted)
    }

    async fn build_history_entries(
        store: Arc<dyn TraceStore>,
        traces: Vec<y_diagnostics::Trace>,
    ) -> Result<Vec<HistoricalEntry>, String> {
        let trace_ids: Vec<uuid::Uuid> = traces.iter().map(|trace| trace.id).collect();
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
                                .and_then(|value| value.as_str())
                                .unwrap_or("(output not captured)")
                                .to_string()
                        } else {
                            obs.output.to_string()
                        };

                        // Cache breakdown is not a DB column; recover it from
                        // the normalized values the subscriber stores in the
                        // observation metadata.
                        let (cache_read_tokens, cache_write_tokens) =
                            extract_cache_tokens(&obs.metadata);

                        entries.push((
                            ts,
                            HistoricalEntry::LlmResponse {
                                iteration: llm_iter,
                                model,
                                input_tokens: obs.input_tokens,
                                output_tokens: obs.output_tokens,
                                cache_read_tokens,
                                cache_write_tokens,
                                context_tokens_used: obs
                                    .input_tokens
                                    .saturating_add(cache_read_tokens)
                                    .saturating_add(cache_write_tokens),
                                duration_ms,
                                cost_usd: obs.cost_usd,
                                tool_calls_requested: extract_tool_calls_requested(&obs.output),
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
        Ok(entries.into_iter().map(|(_, entry)| entry).collect())
    }
}

/// Recover the cache-token breakdown stored in a generation observation's
/// metadata.
///
/// Cache reads/writes are not persisted as observation columns, so the
/// diagnostics subscriber writes the normalized values into the observation
/// metadata. Returns `(cache_read, cache_write)`, defaulting to zero when
/// absent (older rows or providers without caching).
fn extract_cache_tokens(metadata: &serde_json::Value) -> (u64, u64) {
    let read = metadata
        .get("cache_read_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let write = metadata
        .get("cache_write_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    (read, write)
}

fn extract_tool_calls_requested(output: &serde_json::Value) -> Vec<String> {
    fn function_name_from_call(call: &serde_json::Value) -> Option<String> {
        call.get("function")
            .and_then(|function| function.get("name"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                call.get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
    }

    fn names_from_array(array: &[serde_json::Value]) -> Vec<String> {
        array.iter().filter_map(function_name_from_call).collect()
    }

    if let Some(calls) = output
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(serde_json::Value::as_array)
    {
        return names_from_array(calls);
    }

    if let Some(calls) = output
        .get("message")
        .and_then(|message| message.get("tool_calls"))
        .and_then(serde_json::Value::as_array)
    {
        return names_from_array(calls);
    }

    if let Some(calls) = output
        .get("tool_calls")
        .and_then(serde_json::Value::as_array)
    {
        return names_from_array(calls);
    }

    if let Some(blocks) = output.get("content").and_then(serde_json::Value::as_array) {
        let names: Vec<String> = blocks
            .iter()
            .filter(|block| {
                block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use")
            })
            .filter_map(|block| {
                block
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        if !names.is_empty() {
            return names;
        }
    }

    if let Some(parts) = output
        .get("candidates")
        .and_then(|candidates| candidates.get(0))
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(serde_json::Value::as_array)
    {
        let names: Vec<String> = parts
            .iter()
            .filter_map(|part| {
                part.get("functionCall")
                    .or_else(|| part.get("function_call"))
                    .and_then(|call| call.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        if !names.is_empty() {
            return names;
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::{DiagnosticsService, HistoricalEntry};
    use y_diagnostics::{
        InMemoryTraceStore, Observation, ObservationStatus, ObservationType, Trace, TraceStatus,
        TraceStore,
    };

    #[tokio::test]
    async fn orchestration_metrics_group_trace_outcomes_by_selected_mode() {
        let store = Arc::new(InMemoryTraceStore::new());
        let session_id = Uuid::new_v4();

        let mut successful_plan = Trace::new(session_id, "chat-turn");
        successful_plan.metadata = serde_json::json!({
            "orchestration": { "selected_mode": "plan" }
        });
        successful_plan.total_input_tokens = 100;
        successful_plan.total_output_tokens = 20;
        successful_plan.total_cost_usd = 0.4;
        successful_plan.complete();
        store.insert_trace(successful_plan).await.unwrap();

        let mut failed_plan = Trace::new(session_id, "chat-turn");
        failed_plan.metadata = serde_json::json!({
            "orchestration": { "selected_mode": "plan" }
        });
        failed_plan.total_input_tokens = 60;
        failed_plan.total_output_tokens = 10;
        failed_plan.total_cost_usd = 0.2;
        failed_plan.fail();
        store.insert_trace(failed_plan).await.unwrap();

        let mut successful_loop = Trace::new(session_id, "chat-turn");
        successful_loop.metadata = serde_json::json!({
            "orchestration": { "selected_mode": "loop" }
        });
        successful_loop.complete();
        store.insert_trace(successful_loop).await.unwrap();

        let metrics = DiagnosticsService::orchestration_mode_metrics(store, None, 100)
            .await
            .unwrap();

        let plan = metrics.iter().find(|metric| metric.mode == "plan").unwrap();
        assert_eq!(plan.total_runs, 2);
        assert_eq!(plan.successful_runs, 1);
        assert_eq!(plan.failed_runs, 1);
        assert_eq!(plan.success_rate, 0.5);
        assert_eq!(plan.average_tokens, 95.0);
        assert!((plan.average_cost_usd - 0.3).abs() < f64::EPSILON);

        let loop_mode = metrics.iter().find(|metric| metric.mode == "loop").unwrap();
        assert_eq!(loop_mode.total_runs, 1);
        assert_eq!(loop_mode.success_rate, 1.0);
    }

    #[tokio::test]
    async fn dynamic_agent_metrics_group_outcomes_by_agent_version() {
        let store = Arc::new(InMemoryTraceStore::new());
        let session_id = Uuid::new_v4();

        for (version, status) in [
            (1, TraceStatus::Completed),
            (1, TraceStatus::Completed),
            (2, TraceStatus::Completed),
            (2, TraceStatus::Failed),
        ] {
            let mut trace = Trace::new(session_id, "dyn-code-scout");
            trace.metadata = serde_json::json!({
                "dynamic_agent": {
                    "id": "dyn-code-scout",
                    "version": version
                }
            });
            match status {
                TraceStatus::Completed => trace.complete(),
                TraceStatus::Failed => trace.fail(),
                _ => unreachable!(),
            }
            store.insert_trace(trace).await.unwrap();
        }

        let metrics = DiagnosticsService::dynamic_agent_version_metrics(store, None, 100)
            .await
            .unwrap();

        let version_one = metrics.iter().find(|metric| metric.version == 1).unwrap();
        assert_eq!(version_one.total_runs, 2);
        assert_eq!(version_one.success_rate, 1.0);

        let version_two = metrics.iter().find(|metric| metric.version == 2).unwrap();
        assert_eq!(version_two.total_runs, 2);
        assert_eq!(version_two.success_rate, 0.5);
    }

    #[tokio::test]
    async fn dynamic_agent_metrics_treat_explicit_negative_feedback_as_failure() {
        let store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
        let mut trace = Trace::new(Uuid::new_v4(), "subagent:test");
        trace.metadata = serde_json::json!({
            "dynamic_agent": { "id": "dyn-test", "version": 2 },
            "user_feedback": { "score": 0.0 }
        });
        trace.complete();
        store.insert_trace(trace).await.unwrap();

        let metrics = DiagnosticsService::dynamic_agent_version_metrics(store, None, 10)
            .await
            .unwrap();

        assert_eq!(metrics[0].failed_runs, 1);
        assert_eq!(metrics[0].successful_runs, 0);
    }

    #[tokio::test]
    async fn dynamic_agent_regressions_require_repeated_version_evidence() {
        let store = Arc::new(InMemoryTraceStore::new());
        let session_id = Uuid::new_v4();

        for version in [1_u64, 2] {
            for sample in 0..5 {
                let mut trace = Trace::new(session_id, "dyn-code-scout");
                trace.metadata = serde_json::json!({
                    "dynamic_agent": { "id": "dyn-code-scout", "version": version }
                });
                if version == 1 || sample == 0 {
                    trace.complete();
                } else {
                    trace.fail();
                }
                store.insert_trace(trace).await.unwrap();
            }
        }

        let findings = DiagnosticsService::dynamic_agent_regressions(store, None, 100, 5, 0.25)
            .await
            .unwrap();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].agent_id, "dyn-code-scout");
        assert_eq!(findings[0].baseline_version, 1);
        assert_eq!(findings[0].current_version, 2);
        assert!((findings[0].success_rate_drop - 0.8).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_get_history_for_session_ids_includes_descendants_and_tool_requests() {
        let store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
        let parent_session = Uuid::new_v4();
        let child_session = Uuid::new_v4();
        let unrelated_session = Uuid::new_v4();

        let mut parent_trace = Trace::new(parent_session, "chat-turn");
        parent_trace.status = TraceStatus::Completed;
        parent_trace.started_at = Utc::now() - Duration::seconds(10);
        parent_trace.completed_at = Some(parent_trace.started_at + Duration::seconds(1));
        store.insert_trace(parent_trace.clone()).await.unwrap();

        let mut child_trace = Trace::new(child_session, "subagent:plan-writer");
        child_trace.status = TraceStatus::Completed;
        child_trace.started_at = Utc::now() - Duration::seconds(5);
        child_trace.completed_at = Some(child_trace.started_at + Duration::seconds(1));
        store.insert_trace(child_trace.clone()).await.unwrap();

        let mut unrelated_trace = Trace::new(unrelated_session, "chat-turn");
        unrelated_trace.status = TraceStatus::Completed;
        unrelated_trace.started_at = Utc::now() - Duration::seconds(1);
        unrelated_trace.completed_at = Some(unrelated_trace.started_at + Duration::seconds(1));
        store.insert_trace(unrelated_trace.clone()).await.unwrap();

        let mut parent_obs =
            Observation::new(parent_trace.id, ObservationType::Generation, "chat-turn");
        parent_obs.session_id = Some(parent_session);
        parent_obs.status = ObservationStatus::Completed;
        parent_obs.sequence = 1;
        parent_obs.started_at = parent_trace.started_at;
        parent_obs.completed_at = parent_trace.completed_at;
        parent_obs.model = Some("gpt-root".into());
        parent_obs.input_tokens = 100;
        parent_obs.output_tokens = 50;
        parent_obs.cost_usd = 0.01;
        parent_obs.metadata = serde_json::json!({ "duration_ms": 250 });
        parent_obs.input = serde_json::json!([{ "role": "user", "content": "build plan" }]);
        parent_obs.output = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "I will create a plan",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {
                            "name": "Plan",
                            "arguments": {
                                "request": "build plan"
                            }
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        store.insert_observation(parent_obs).await.unwrap();

        let mut child_obs = Observation::new(
            child_trace.id,
            ObservationType::Generation,
            "subagent:plan-writer",
        );
        child_obs.session_id = Some(child_session);
        child_obs.status = ObservationStatus::Completed;
        child_obs.sequence = 1;
        child_obs.started_at = child_trace.started_at;
        child_obs.completed_at = child_trace.completed_at;
        child_obs.model = Some("gpt-child".into());
        child_obs.input_tokens = 80;
        child_obs.output_tokens = 40;
        child_obs.cost_usd = 0.008;
        child_obs.metadata = serde_json::json!({ "duration_ms": 180 });
        child_obs.input = serde_json::json!([{ "role": "user", "content": "write plan" }]);
        child_obs.output = serde_json::json!({
            "content": "plan ready",
        });
        store.insert_observation(child_obs).await.unwrap();

        let mut unrelated_obs =
            Observation::new(unrelated_trace.id, ObservationType::Generation, "chat-turn");
        unrelated_obs.session_id = Some(unrelated_session);
        unrelated_obs.status = ObservationStatus::Completed;
        unrelated_obs.sequence = 1;
        unrelated_obs.started_at = unrelated_trace.started_at;
        unrelated_obs.completed_at = unrelated_trace.completed_at;
        unrelated_obs.model = Some("gpt-unrelated".into());
        store.insert_observation(unrelated_obs).await.unwrap();

        let entries = DiagnosticsService::get_history_for_session_ids(
            store,
            &[parent_session.to_string(), child_session.to_string()],
            50,
        )
        .await
        .unwrap();

        assert_eq!(entries.len(), 2);

        match &entries[0] {
            HistoricalEntry::LlmResponse {
                agent_name,
                tool_calls_requested,
                ..
            } => {
                assert_eq!(agent_name, "chat-turn");
                assert_eq!(tool_calls_requested, &vec!["Plan".to_string()]);
            }
            other => panic!("expected llm response, got {other:?}"),
        }

        match &entries[1] {
            HistoricalEntry::LlmResponse { agent_name, .. } => {
                assert_eq!(agent_name, "subagent:plan-writer");
            }
            other => panic!("expected llm response, got {other:?}"),
        }
    }
}
