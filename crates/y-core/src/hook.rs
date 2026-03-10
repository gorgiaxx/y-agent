//! Hook, middleware, and event bus traits.
//!
//! Design reference: hooks-plugin-design.md
//!
//! Architecture:
//! - Hooks: observe lifecycle events (read-only)
//! - Middleware: transform data flowing through chains (mutable)
//! - Event Bus: async fire-and-forget notifications
//!
//! Three middleware chains: Context, Tool, LLM.
//! Guardrails are implemented *as* middleware, not as a parallel system.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Middleware chain types
// ---------------------------------------------------------------------------

/// Identifies which middleware chain a middleware belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainType {
    /// Context assembly pipeline (prompt building, memory injection, etc.).
    Context,
    /// Tool execution pipeline (validation, journaling, guardrails, etc.).
    Tool,
    /// LLM call pipeline (rate limiting, caching, auditing, etc.).
    Llm,
    /// Context compaction pipeline.
    Compaction,
    /// Memory storage pipeline.
    Memory,
}

/// Opaque context passed through a middleware chain.
///
/// Each chain type carries its own payload. The middleware receives a
/// mutable reference and can inspect or modify it.
#[derive(Debug)]
pub struct MiddlewareContext {
    /// The chain this context belongs to.
    pub chain_type: ChainType,
    /// The mutable payload (chain-specific data as JSON).
    pub payload: serde_json::Value,
    /// Metadata accumulated by prior middleware in the chain.
    pub metadata: serde_json::Value,
    /// If true, a middleware has decided to abort the chain.
    pub aborted: bool,
    /// Abort reason (set when `aborted` is true).
    pub abort_reason: Option<String>,
}

impl MiddlewareContext {
    /// Create a new middleware context.
    pub fn new(chain_type: ChainType, payload: serde_json::Value) -> Self {
        Self {
            chain_type,
            payload,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            aborted: false,
            abort_reason: None,
        }
    }

    /// Abort the chain with a reason.
    pub fn abort(&mut self, reason: impl Into<String>) {
        self.aborted = true;
        self.abort_reason = Some(reason.into());
    }
}

/// Result of middleware execution.
#[derive(Debug)]
pub enum MiddlewareResult {
    /// Continue to the next middleware in the chain.
    Continue,
    /// Skip remaining middleware and return current context.
    ShortCircuit,
}

// ---------------------------------------------------------------------------
// Middleware trait
// ---------------------------------------------------------------------------

/// A middleware that transforms data flowing through a chain.
///
/// Middleware is ordered by priority (lower runs first). Each middleware
/// receives a mutable context and can modify the payload, add metadata,
/// or abort the chain.
///
/// Guardrails, file journaling, tool gap detection, skill auditing,
/// and context status injection are all implemented as middleware.
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Execute this middleware, potentially modifying the context.
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError>;

    /// The chain this middleware belongs to.
    fn chain_type(&self) -> ChainType;

    /// Execution priority (lower = earlier). Default middleware priorities:
    /// - 100: `BuildSystemPrompt`
    /// - 200: `InjectBootstrap`
    /// - 300: `InjectMemory`
    /// - 350: `InjectKnowledge`
    /// - 400: `InjectSkills`
    /// - 500: `InjectTools`
    /// - 600: `LoadHistory`
    /// - 700: `InjectContextStatus`
    fn priority(&self) -> u32;

    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &str;
}

/// Errors from middleware execution.
#[derive(Debug, thiserror::Error)]
pub enum MiddlewareError {
    #[error("middleware {name} timed out after {timeout_ms}ms")]
    Timeout { name: String, timeout_ms: u64 },

    #[error("middleware {name} panicked: {message}")]
    Panic { name: String, message: String },

    #[error("middleware {name} error: {message}")]
    ExecutionError { name: String, message: String },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Hook points
// ---------------------------------------------------------------------------

/// Lifecycle hook point identifiers.
///
/// Hooks are read-only observers. They cannot modify the data flowing
/// through the system -- only middleware can do that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    PreLlmCall,
    PostLlmCall,
    PreToolExecute,
    PostToolExecute,
    MemoryStored,
    MemoryRecalled,
    SessionCreated,
    SessionClosed,
    PreCompaction,
    PostCompaction,
    WorkflowStarted,
    WorkflowCompleted,
    AgentLoopStart,
    AgentLoopEnd,
    PrePipelineStep,
    PostPipelineStep,
    ToolGapDetected,
    ToolGapResolved,
    AgentGapDetected,
    AgentGapResolved,
    DynamicAgentCreated,
    DynamicAgentDeactivated,
}

impl fmt::Display for HookPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Data payload for a hook invocation.
#[derive(Debug, Clone)]
pub struct HookData {
    pub hook_point: HookPoint,
    pub payload: serde_json::Value,
}

/// A read-only hook handler.
#[async_trait]
pub trait HookHandler: Send + Sync {
    /// Handle a hook event. Panics are caught and logged; they do not
    /// propagate or abort the operation that triggered the hook.
    async fn handle(&self, data: &HookData);

    /// Which hook points this handler is interested in.
    fn hook_points(&self) -> Vec<HookPoint>;
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

/// Event types for the async event bus (fire-and-forget).
///
/// Events are delivered to subscribers via per-subscriber bounded channels.
/// Slow subscribers drop oldest events (no backpressure).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    LlmCallCompleted {
        provider: String,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
    },
    ToolExecuted {
        tool_name: String,
        success: bool,
        duration_ms: u64,
    },
    WorkflowEvent {
        workflow_id: String,
        event: String,
    },
    RuntimeEvent {
        backend: String,
        event: String,
    },
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}

/// Subscriber to the event bus.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Handle an event. Implementations should be fast and non-blocking.
    async fn on_event(&self, event: &Event);

    /// Which event types this subscriber is interested in.
    /// Return None to receive all events.
    fn event_filter(&self) -> Option<Vec<String>>;
}
