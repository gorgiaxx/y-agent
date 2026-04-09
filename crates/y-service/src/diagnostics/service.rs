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

                        entries.push((
                            ts,
                            HistoricalEntry::LlmResponse {
                                iteration: llm_iter,
                                model,
                                input_tokens: obs.input_tokens,
                                output_tokens: obs.output_tokens,
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
