//! Gateway 2: Diagnostics-aware tool call wrapper.
//!
//! Records tool call observations and emits real-time
//! `DiagnosticsEvent::ToolCallCompleted` without any manual wiring
//! in business logic.

use std::sync::Arc;

use y_diagnostics::{DiagnosticsEvent, DiagnosticsSubscriber, TraceStore, DIAGNOSTICS_CTX};

/// A thin gateway that records tool call observations after execution.
///
/// Does NOT own the tool dispatch logic. The caller executes the tool and
/// then calls [`record`] to persist the observation and emit a broadcast
/// event. This keeps the gateway stateless and composable.
pub struct DiagnosticsToolGateway {
    diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,
    broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
}

impl DiagnosticsToolGateway {
    pub fn new(
        diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,
        broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
    ) -> Self {
        Self {
            diagnostics,
            broadcast_tx,
        }
    }

    /// Record a completed tool call in the diagnostics subsystem.
    ///
    /// Reads `DIAGNOSTICS_CTX` from the task-local to obtain the trace ID
    /// and parent generation ID. If no context is set, this is a no-op.
    pub async fn record(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        output: serde_json::Value,
        duration_ms: u64,
        success: bool,
    ) {
        let Ok(ctx) = DIAGNOSTICS_CTX.try_with(Clone::clone) else {
            return;
        };

        let parent_id = *ctx.last_gen_id.lock().await;

        let _ = self
            .diagnostics
            .on_tool_call(
                ctx.trace_id,
                parent_id,
                ctx.session_id,
                tool_name,
                input.clone(),
                output.clone(),
                duration_ms,
                success,
            )
            .await;

        let input_preview = serde_json::to_string(&input).unwrap_or_default();
        let result_preview = serde_json::to_string(&output).unwrap_or_default();

        let _ = self.broadcast_tx.send(DiagnosticsEvent::ToolCallCompleted {
            trace_id: ctx.trace_id,
            session_id: ctx.session_id,
            agent_name: ctx.agent_name.clone(),
            tool_name: tool_name.to_string(),
            success,
            duration_ms,
            input_preview,
            result_preview,
        });
    }

    /// Convenience: record a tool call from a pre-serialized result string.
    pub async fn record_from_str(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
        result_content: &str,
        duration_ms: u64,
        success: bool,
    ) {
        let output: serde_json::Value = serde_json::from_str(result_content)
            .unwrap_or(serde_json::Value::String(result_content.to_string()));
        self.record(tool_name, arguments.clone(), output, duration_ms, success)
            .await;
    }
}
