//! Diagnostics-aware agent delegator.
//!
//! Wraps `AgentDelegator` to create `subagent:<name>` traces and propagate
//! the trace ID via `DIAGNOSTICS_CTX` task-local.

use std::sync::Arc;

use y_diagnostics::{DiagnosticsContext, DiagnosticsEvent, DIAGNOSTICS_CTX};

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
                } else {
                    trace.metadata = serde_json::json!({ "input": input_json });
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
