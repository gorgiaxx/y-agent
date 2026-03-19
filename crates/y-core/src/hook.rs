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

/// Event types for the async event bus (fire-and-forget).
///
/// Events are delivered to subscribers via per-subscriber bounded channels.
/// Slow subscribers drop oldest events (no backpressure on publishers).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // --- LLM events ---
    LlmCallStarted {
        model: String,
        message_count: u32,
        tool_count: u32,
    },
    LlmCallCompleted {
        provider: String,
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
    },
    LlmCallFailed {
        model: String,
        error: String,
        retry_attempt: u32,
    },

    // --- Tool events ---
    ToolExecuted {
        tool_name: String,
        success: bool,
        duration_ms: u64,
    },
    ToolFailed {
        tool_name: String,
        error_type: String,
        args_summary: String,
    },

    // --- Memory events ---
    MemoryStored {
        memory_id: String,
        memory_type: String,
        importance: f64,
    },
    MemoryRecalled {
        query_summary: String,
        result_count: u32,
        top_score: f64,
    },

    // --- Session events ---
    SessionCreated {
        session_id: String,
        parent_session_id: Option<String>,
    },
    SessionClosed {
        session_id: String,
        message_count: u32,
    },

    // --- Compaction events ---
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

    // --- Context events ---
    ContextOverflow {
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

    // --- Pruning events ---
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

    // --- Orchestration events ---
    WorkflowEvent {
        workflow_id: String,
        event: String,
    },

    // --- Agent events ---
    AgentLoopIteration {
        run_id: String,
        iteration: u32,
        tool_calls_count: u32,
    },

    // --- Pipeline events ---
    PipelineStarted {
        pipeline_id: String,
        template_name: String,
        step_count: u32,
    },
    PipelineStepCompleted {
        pipeline_id: String,
        step_name: String,
        duration_ms: u64,
        tokens_used: u32,
    },
    PipelineStepFailed {
        pipeline_id: String,
        step_name: String,
        error: String,
        retry_count: u32,
    },
    PipelineCompleted {
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

    // --- Autonomy events ---
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

    // --- Runtime events ---
    RuntimeEvent {
        backend: String,
        event: String,
    },

    // --- Custom events ---
    Custom {
        name: String,
        payload: serde_json::Value,
    },
}

impl Event {
    /// Get the category of this event for filtering.
    pub fn category(&self) -> EventCategory {
        match self {
            Self::LlmCallStarted { .. }
            | Self::LlmCallCompleted { .. }
            | Self::LlmCallFailed { .. } => EventCategory::Llm,

            Self::ToolExecuted { .. } | Self::ToolFailed { .. } => EventCategory::Tool,

            Self::MemoryStored { .. } | Self::MemoryRecalled { .. } => EventCategory::Memory,

            Self::SessionCreated { .. } | Self::SessionClosed { .. } => EventCategory::Session,

            Self::CompactionTriggered { .. } | Self::CompactionFailed { .. } => {
                EventCategory::Compaction
            }

            Self::ContextOverflow { .. }
            | Self::CanonicalSynced { .. }
            | Self::SessionRepaired { .. }
            | Self::PruningApplied { .. }
            | Self::PruningSkipped { .. }
            | Self::PruningFailed { .. } => EventCategory::Context,

            Self::WorkflowEvent { .. } => EventCategory::Orchestration,

            Self::AgentLoopIteration { .. } => EventCategory::Agent,

            Self::PipelineStarted { .. }
            | Self::PipelineStepCompleted { .. }
            | Self::PipelineStepFailed { .. }
            | Self::PipelineCompleted { .. }
            | Self::WorkingMemorySlotWritten { .. } => EventCategory::Pipeline,

            Self::ToolGapDetected { .. }
            | Self::ToolGapResolved { .. }
            | Self::AgentGapDetected { .. }
            | Self::AgentGapResolved { .. }
            | Self::DynamicToolRegistered { .. }
            | Self::DynamicAgentRegistered { .. }
            | Self::DynamicAgentDeactivated { .. }
            | Self::WorkflowTemplateCreated { .. } => EventCategory::Autonomy,

            Self::RuntimeEvent { .. } => EventCategory::Runtime,

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
