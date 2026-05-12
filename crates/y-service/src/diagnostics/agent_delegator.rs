//! Diagnostics-aware agent delegator.
//!
//! Wraps `AgentDelegator` to create `subagent:<name>` traces, link them from
//! the parent trace, and propagate the trace ID via `DIAGNOSTICS_CTX` task-local.

use std::sync::Arc;
use std::time::Instant;

use y_diagnostics::{
    DiagnosticsContext, DiagnosticsEvent, SubagentCompleteParams, SubagentStartParams,
    DIAGNOSTICS_CTX,
};

/// A decorator around `AgentDelegator` that records diagnostics (trace +
/// generation observation) for each delegation call.
///
/// Creates a `subagent:<name>` trace and propagates its ID via the
/// `DIAGNOSTICS_CTX` task-local so that gateways can forward it to
/// `AgentService::execute()`. The execute loop then records per-iteration
/// generation and tool-call observations under this trace, giving full
/// visibility in the diagnostics panel.
///
/// When a `session_id` is provided by the caller, the subagent trace is
/// associated with that session so it appears in session-level diagnostics.
/// When `None`, uses `Uuid::nil()` (global-only visibility).
///
/// On delegation completion, emits `DiagnosticsEvent::SubagentCompleted`
/// through the broadcast channel so that presentation layers can trigger
/// DB history reloads without any manual per-caller wiring.
pub struct DiagnosticsAgentDelegator {
    inner: Arc<dyn y_core::agent::AgentDelegator>,
    diagnostics: Arc<y_diagnostics::DiagnosticsSubscriber<dyn y_diagnostics::TraceStore>>,
    broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
}

impl DiagnosticsAgentDelegator {
    /// Create a new diagnostics-aware delegator wrapping `inner`.
    pub fn new(
        inner: Arc<dyn y_core::agent::AgentDelegator>,
        diagnostics: Arc<y_diagnostics::DiagnosticsSubscriber<dyn y_diagnostics::TraceStore>>,
        broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
    ) -> Self {
        Self {
            inner,
            diagnostics,
            broadcast_tx,
        }
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
        let parent_ctx = DIAGNOSTICS_CTX.try_with(Clone::clone).ok();

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

        let parent_observation =
            if let (Some(parent), Some(child_trace_id)) = (parent_ctx.as_ref(), trace_id) {
                let parent_id = *parent.last_gen_id.lock().await;
                self.diagnostics
                    .on_subagent_start(SubagentStartParams {
                        trace_id: parent.trace_id,
                        parent_id,
                        session_id: parent.session_id,
                        agent_name: agent_name.to_string(),
                        input: input_json.clone(),
                        child_trace_id: Some(child_trace_id),
                        child_session_id: session_id,
                    })
                    .await
                    .ok()
                    .map(|observation_id| {
                        (
                            parent.trace_id,
                            observation_id,
                            child_trace_id,
                            Instant::now(),
                        )
                    })
            } else {
                None
            };

        // Delegate to the inner delegator, propagating the trace_id via
        // DIAGNOSTICS_CTX task-local so gateways and ServiceAgentRunner can
        // read it.
        let result = if let Some(tid) = trace_id {
            let ctx = DiagnosticsContext::new(tid, session_id, agent_name.to_string());
            DIAGNOSTICS_CTX
                .scope(ctx, async {
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
                    if let Some((parent_trace_id, parent_obs_id, _, _)) = parent_observation {
                        map.insert(
                            "parent_trace_id".to_string(),
                            serde_json::Value::String(parent_trace_id.to_string()),
                        );
                        map.insert(
                            "parent_observation_id".to_string(),
                            serde_json::Value::String(parent_obs_id.to_string()),
                        );
                    }
                } else {
                    let mut metadata = serde_json::json!({ "input": input_json });
                    if let serde_json::Value::Object(ref mut map) = metadata {
                        if let Some((parent_trace_id, parent_obs_id, _, _)) = parent_observation {
                            map.insert(
                                "parent_trace_id".to_string(),
                                serde_json::Value::String(parent_trace_id.to_string()),
                            );
                            map.insert(
                                "parent_observation_id".to_string(),
                                serde_json::Value::String(parent_obs_id.to_string()),
                            );
                        }
                    }
                    trace.metadata = metadata;
                }
                let _ = self.diagnostics.store().update_trace(trace).await;
            }
        }

        // Close the trace and broadcast completion. Per-iteration generation
        // and tool-call observations have already been recorded by the
        // gateways under this same trace_id (via DIAGNOSTICS_CTX).
        let success = result.is_ok();
        let mut trace_summary = None;
        match &result {
            Ok(output) => {
                if let Some(tid) = trace_id {
                    trace_summary = self
                        .diagnostics
                        .on_trace_end(tid, true, Some(&output.text))
                        .await
                        .ok();
                }
            }
            Err(_) => {
                if let Some(tid) = trace_id {
                    trace_summary = self.diagnostics.on_trace_end(tid, false, None).await.ok();
                }
            }
        }

        if let Some((parent_trace_id, parent_obs_id, _, started_at)) = parent_observation {
            let duration_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(0);
            let (output, error_message) = match &result {
                Ok(output) => (
                    Some(serde_json::json!({
                        "text": output.text.clone(),
                        "model_used": output.model_used.clone(),
                        "tokens_used": output.tokens_used,
                        "input_tokens": output.input_tokens,
                        "output_tokens": output.output_tokens,
                        "duration_ms": output.duration_ms,
                    })),
                    None,
                ),
                Err(error) => (None, Some(error.to_string())),
            };

            let _ = self
                .diagnostics
                .on_subagent_complete(SubagentCompleteParams {
                    trace_id: parent_trace_id,
                    observation_id: parent_obs_id,
                    success,
                    output,
                    error_message,
                    duration_ms,
                })
                .await;
        }

        // Emit SubagentCompleted through the broadcast channel so
        // presentation layers can trigger DB history reloads without
        // any manual per-caller wiring.
        if let Some(tid) = trace_id {
            let _ = self.broadcast_tx.send(DiagnosticsEvent::SubagentCompleted {
                trace_id: tid,
                session_id,
                agent_name: agent_name.to_string(),
                success,
            });
        }

        // Emit TraceCompleted for the Langfuse export bridge.
        if let Some(summary) = trace_summary {
            let _ = self.broadcast_tx.send(DiagnosticsEvent::TraceCompleted {
                trace_id: summary.trace_id,
                session_id: Some(summary.session_id),
                agent_name: summary.agent_name,
                success: summary.success,
                total_input_tokens: summary.total_input_tokens,
                total_output_tokens: summary.total_output_tokens,
                total_cost_usd: summary.total_cost_usd,
                duration_ms: summary.duration_ms,
            });
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use y_core::agent::{AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput};
    use y_diagnostics::{
        DiagnosticsContext, DiagnosticsSubscriber, InMemoryTraceStore, ObservationStatus,
        ObservationType, TraceStore, DIAGNOSTICS_CTX,
    };

    use super::*;

    #[derive(Debug)]
    struct MockDelegator;

    #[async_trait]
    impl AgentDelegator for MockDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Ok(DelegationOutput {
                text: "subagent result".to_string(),
                tokens_used: 3,
                input_tokens: 1,
                output_tokens: 2,
                model_used: "test-model".to_string(),
                duration_ms: 25,
            })
        }
    }

    #[tokio::test]
    async fn delegate_records_subagent_observation_on_parent_trace() {
        let store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
        let diagnostics = Arc::new(DiagnosticsSubscriber::new(Arc::clone(&store)));
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let delegator =
            DiagnosticsAgentDelegator::new(Arc::new(MockDelegator), Arc::clone(&diagnostics), tx);

        let parent_session_id = uuid::Uuid::new_v4();
        let parent_trace_id = diagnostics
            .on_trace_start(parent_session_id, "chat-turn", "delegate work")
            .await
            .unwrap();
        let parent_ctx =
            DiagnosticsContext::new(parent_trace_id, Some(parent_session_id), "chat".into());

        DIAGNOSTICS_CTX
            .scope(parent_ctx, async {
                delegator
                    .delegate(
                        "worker",
                        serde_json::json!({"task": "inspect"}),
                        ContextStrategyHint::None,
                        Some(uuid::Uuid::new_v4()),
                    )
                    .await
                    .unwrap();
            })
            .await;

        let observations = store.get_observations(parent_trace_id).await.unwrap();
        let subagent_obs = observations
            .iter()
            .find(|obs| obs.obs_type == ObservationType::SubAgent)
            .expect("parent trace should contain a subagent observation");

        assert_eq!(subagent_obs.name, "agent.delegate.worker");
        assert_eq!(subagent_obs.status, ObservationStatus::Completed);
        assert_eq!(subagent_obs.input["task"], "inspect");
        assert_eq!(subagent_obs.output["text"], "subagent result");
        assert!(subagent_obs.metadata["child_trace_id"].as_str().is_some());
    }
}
