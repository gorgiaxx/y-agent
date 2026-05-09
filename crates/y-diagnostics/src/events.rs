//! Real-time diagnostic events emitted by gateways.
//!
//! Subscribers (GUI, CLI, web SSE) receive these via
//! `tokio::sync::broadcast` without any business-layer involvement.

/// Real-time diagnostic event emitted by provider and tool gateways.
///
/// Sent over `broadcast::Sender<DiagnosticsEvent>` and consumed by any
/// number of subscribers (Tauri, CLI dashboard, SSE endpoint).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiagnosticsEvent {
    /// A streaming LLM call started (observation created with Running status).
    LlmCallStarted {
        trace_id: uuid::Uuid,
        observation_id: uuid::Uuid,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        iteration: u32,
        model: String,
    },
    /// An LLM call completed (observation finalized with Completed status).
    LlmCallCompleted {
        trace_id: uuid::Uuid,
        observation_id: uuid::Uuid,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        iteration: u32,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
        cost_usd: f64,
        tool_calls_requested: Vec<String>,
        prompt_preview: String,
        response_text: String,
        context_window: usize,
    },
    /// An LLM call failed.
    LlmCallFailed {
        trace_id: uuid::Uuid,
        observation_id: Option<uuid::Uuid>,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        iteration: u32,
        model: String,
        error: String,
        duration_ms: u64,
    },
    /// A tool call completed (or failed).
    ToolCallCompleted {
        trace_id: uuid::Uuid,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        tool_name: String,
        success: bool,
        duration_ms: u64,
        input_preview: String,
        result_preview: String,
    },
    /// Incremental streaming content delta (for Tier 2 real-time output
    /// refresh).
    StreamDelta {
        observation_id: uuid::Uuid,
        content: String,
    },
    /// Incremental streaming reasoning delta.
    StreamReasoningDelta {
        observation_id: uuid::Uuid,
        content: String,
    },
    /// A subagent delegation completed (or failed).
    ///
    /// Emitted by `DiagnosticsAgentDelegator` after `on_trace_end`.
    /// Presentation layers use this to trigger DB history reload
    /// without any manual per-caller wiring.
    SubagentCompleted {
        trace_id: uuid::Uuid,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        success: bool,
    },
    /// A top-level or sub-agent trace finished and is ready for export.
    ///
    /// Emitted after `DiagnosticsSubscriber::on_trace_end` finalizes the
    /// trace row. Used by the optional Langfuse export bridge to flush
    /// completed traces without touching business logic.
    TraceCompleted {
        trace_id: uuid::Uuid,
        session_id: Option<uuid::Uuid>,
        agent_name: String,
        success: bool,
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_cost_usd: f64,
        duration_ms: u64,
    },
}
