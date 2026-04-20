//! Hook, middleware, and event bus traits.
//!
//! Design reference: hooks-plugin-design.md
//!
//! Architecture:
//! - Hooks: observe lifecycle events (read-only)
//! - Middleware: transform data flowing through chains (mutable)
//! - Event Bus: async fire-and-forget notifications
//! - Hook Handlers: configuration-driven external extensibility (command/HTTP/prompt/agent)
//!
//! Five middleware chains: Context, Tool, LLM, Compaction, Memory.
//! Guardrails are implemented *as* middleware, not as a parallel system.

use std::fmt;
use std::time::Duration;

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
    ContextOverflow,
    PostSkillInjection,
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

/// Event category for filtering in the event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Llm,
    Tool,
    Memory,
    Session,
    Compaction,
    Context,
    Orchestration,
    Agent,
    Pipeline,
    Autonomy,
    Runtime,
    Custom,
}

// ---------------------------------------------------------------------------
// Domain sub-enums
// ---------------------------------------------------------------------------

/// LLM lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmEvent {
    CallStarted {
        model: String,
        message_count: u32,
        tool_count: u32,
    },
    CallCompleted {
        provider: String,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
    },
    CallFailed {
        model: String,
        error: String,
        retry_attempt: u32,
    },
}

/// Tool execution events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolEvent {
    Executed {
        tool_name: String,
        success: bool,
        duration_ms: u64,
    },
    Failed {
        tool_name: String,
        error_type: String,
        args_summary: String,
    },
}

/// Memory storage and retrieval events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryEvent {
    Stored {
        memory_id: String,
        memory_type: String,
        importance: f64,
    },
    Recalled {
        query_summary: String,
        result_count: u32,
        top_score: f64,
    },
}

/// Session lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    Created {
        session_id: String,
        parent_session_id: Option<String>,
    },
    Closed {
        session_id: String,
        message_count: u32,
    },
}

/// Context management events (compaction, overflow, pruning, sync, repair).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextEvent {
    CompactionTriggered {
        session_id: String,
        strategy: String,
        tokens_before: u32,
        tokens_after: u32,
    },
    CompactionFailed {
        session_id: String,
        error: String,
        fallback_used: bool,
    },
    Overflow {
        session_id: String,
        estimated_tokens: u32,
        budget: u32,
    },
    CanonicalSynced {
        canonical_id: String,
        source_channel: String,
        message_count: u32,
    },
    SessionRepaired {
        session_id: String,
        fixes: serde_json::Value,
    },
    PruningApplied {
        session_id: String,
        strategy: String,
        messages_pruned: u32,
        tokens_before: u32,
        tokens_after: u32,
    },
    PruningSkipped {
        session_id: String,
        reason: String,
    },
    PruningFailed {
        session_id: String,
        strategy: String,
        error: String,
    },
}

/// Agent loop events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    LoopIteration {
        run_id: String,
        iteration: u32,
        tool_calls_count: u32,
    },
}

/// Multi-step pipeline events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PipelineEvent {
    Started {
        pipeline_id: String,
        template_name: String,
        step_count: u32,
    },
    StepCompleted {
        pipeline_id: String,
        step_name: String,
        duration_ms: u64,
        tokens_used: u32,
    },
    StepFailed {
        pipeline_id: String,
        step_name: String,
        error: String,
        retry_count: u32,
    },
    Completed {
        pipeline_id: String,
        total_duration_ms: u64,
        total_tokens: u32,
    },
    WorkingMemorySlotWritten {
        pipeline_id: String,
        slot_key: String,
        category: String,
        token_estimate: u32,
    },
}

/// Self-evolution and autonomy events (tool/agent gaps, dynamic registration).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutonomyEvent {
    ToolGapDetected {
        gap_id: String,
        gap_type: String,
        tool_name: String,
        session_id: String,
    },
    ToolGapResolved {
        gap_id: String,
        resolution_type: String,
        tool_name: String,
        duration_ms: u64,
    },
    AgentGapDetected {
        gap_id: String,
        gap_type: String,
        agent_name: String,
        session_id: String,
    },
    AgentGapResolved {
        gap_id: String,
        resolution_type: String,
        agent_name: String,
        duration_ms: u64,
    },
    DynamicToolRegistered {
        tool_name: String,
        implementation_type: String,
        created_by: String,
    },
    DynamicAgentRegistered {
        agent_name: String,
        trust_tier: String,
        mode: String,
        created_by: String,
        delegation_depth: u32,
    },
    DynamicAgentDeactivated {
        agent_name: String,
        reason: String,
        deactivated_by: String,
    },
    WorkflowTemplateCreated {
        template_id: String,
        template_name: String,
        parameter_count: u32,
        created_by: String,
    },
}

// ---------------------------------------------------------------------------
// Top-level Event enum
// ---------------------------------------------------------------------------

/// Event types for the async event bus (fire-and-forget).
///
/// Events are delivered to subscribers via per-subscriber bounded channels.
/// Slow subscribers drop oldest events (no backpressure on publishers).
///
/// Each domain has its own sub-enum; the top-level `Event` delegates to
/// the appropriate variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Llm(LlmEvent),
    Tool(ToolEvent),
    Memory(MemoryEvent),
    Session(SessionEvent),
    Context(ContextEvent),
    Agent(AgentEvent),
    Pipeline(PipelineEvent),
    Autonomy(AutonomyEvent),
    Orchestration {
        workflow_id: String,
        event: String,
    },
    Runtime {
        backend: String,
        event: String,
    },
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}

impl Event {
    /// Get the category of this event for filtering.
    pub fn category(&self) -> EventCategory {
        match self {
            Self::Llm(_) => EventCategory::Llm,
            Self::Tool(_) => EventCategory::Tool,
            Self::Memory(_) => EventCategory::Memory,
            Self::Session(_) => EventCategory::Session,
            Self::Context(ctx) => match ctx {
                ContextEvent::CompactionTriggered { .. }
                | ContextEvent::CompactionFailed { .. } => EventCategory::Compaction,
                _ => EventCategory::Context,
            },
            Self::Agent(_) => EventCategory::Agent,
            Self::Pipeline(_) => EventCategory::Pipeline,
            Self::Autonomy(_) => EventCategory::Autonomy,
            Self::Orchestration { .. } => EventCategory::Orchestration,
            Self::Runtime { .. } => EventCategory::Runtime,
            Self::Custom { .. } => EventCategory::Custom,
        }
    }
}

/// Filter for event bus subscriptions.
///
/// If `categories` is empty, all events are received.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Event categories to accept. Empty means all.
    pub categories: Vec<EventCategory>,
}

impl EventFilter {
    /// Create a filter that accepts all events.
    pub fn all() -> Self {
        Self {
            categories: Vec::new(),
        }
    }

    /// Create a filter for specific categories.
    pub fn categories(categories: Vec<EventCategory>) -> Self {
        Self { categories }
    }

    /// Check if an event matches this filter.
    pub fn matches(&self, event: &Event) -> bool {
        self.categories.is_empty() || self.categories.contains(&event.category())
    }
}

/// Subscriber to the event bus.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Handle an event. Implementations should be fast and non-blocking.
    async fn on_event(&self, event: &Event);

    /// Event filter for this subscriber.
    /// Return `EventFilter::all()` to receive all events.
    fn event_filter(&self) -> EventFilter;
}

// ---------------------------------------------------------------------------
// Hook handler DI traits
// ---------------------------------------------------------------------------

/// Single-turn LLM evaluation for prompt hooks.
///
/// This trait decouples `y-hooks` from `y-provider`. The implementation
/// (in `y-provider`) wraps `ProviderPool::chat_completion` with a system
/// prompt instructing the LLM to return structured `{ok, reason}` JSON.
///
/// Injected into `HookHandlerExecutor` via `HookSystem::set_llm_runner()`.
#[async_trait]
pub trait HookLlmRunner: Send + Sync {
    /// Send a single-turn completion request and return the raw response text.
    ///
    /// The caller (`execute_prompt` in `y-hooks`) is responsible for parsing
    /// the response as `PromptAgentDecision`.
    ///
    /// # Arguments
    /// - `system_prompt` — instructs the LLM on the response format
    /// - `user_message` — the expanded prompt (with `$ARGUMENTS` replaced)
    /// - `model` — optional model override; `None` = fastest available
    /// - `timeout` — maximum time to wait for the response
    async fn evaluate(
        &self,
        system_prompt: &str,
        user_message: &str,
        model: Option<&str>,
        timeout: Duration,
    ) -> Result<String, String>;
}

/// Multi-turn subagent execution for agent hooks.
///
/// This trait decouples `y-hooks` from `y-agent`. The implementation
/// (in `y-agent`) spawns a restricted subagent with read-only tools
/// (`Read`, `Grep`, `Glob`) and runs it for up to `max_turns` turns.
///
/// Injected into `HookHandlerExecutor` via `HookSystem::set_agent_runner()`.
#[async_trait]
pub trait HookAgentRunner: Send + Sync {
    /// Run a subagent with the given task prompt.
    ///
    /// The subagent has access to read-only tools only. After completing
    /// (or hitting `max_turns`), it returns a raw JSON response string
    /// that the caller parses as `PromptAgentDecision`.
    ///
    /// # Arguments
    /// - `task_prompt` — the expanded task description (with `$ARGUMENTS` replaced)
    /// - `model` — optional model override; `None` = fastest available
    /// - `max_turns` — maximum agent loop iterations (typically 50)
    /// - `timeout` — maximum wall-clock time for the entire agent run
    async fn run_agent(
        &self,
        task_prompt: &str,
        model: Option<&str>,
        max_turns: u32,
        timeout: Duration,
    ) -> Result<String, String>;
}
