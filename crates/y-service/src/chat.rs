//! Chat service — LLM turn lifecycle orchestration.
//!
//! Centralises the full LLM-turn lifecycle:
//! 1. Context assembly (system prompt via context pipeline)
//! 2. Build `ChatRequest` with tool definitions
//! 3. LLM call via `ProviderPool`
//! 4. Diagnostics recording (trace, generation, tool observations)
//! 5. Tool execution loop (up to `guardrails.max_tool_iterations`)
//! 6. Session message persistence
//! 7. Checkpoint creation
//!
//! The core LLM + tool loop has been extracted into [`crate::agent_service::AgentService`]
//! so that sub-agents (A2A) share the same execution path. `ChatService` is now
//! a thin session-management wrapper.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_agent::agent::definition::AgentDefinition;
use y_context::AssembledContext;
use y_core::permission_types::PermissionMode;
use y_core::provider::{GeneratedImage, RequestMode, ThinkingConfig, ToolCallingMode};
use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionNode};
use y_core::trust::TrustTier;
use y_core::types::{Message, Role, SessionId};

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError};
use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Record of a tool call executed during a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallRecord {
    /// Tool name.
    pub name: String,
    /// Serialised tool arguments (JSON string).
    pub arguments: String,
    /// Whether the tool executed successfully.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Result content (serialised JSON string).
    ///
    /// For Browser/WebFetch tools, this is usually the stripped result that
    /// only contains LLM-relevant fields (`url`, `title`, `text`).
    /// GUI-only metadata (for example `favicon_url`, `navigation`) is stripped
    /// before this field is set.
    pub result_content: String,
    /// Compact URL metadata for Browser/WebFetch tools (JSON with url, title,
    /// `favicon_url`). Extracted from the full result before stripping so
    /// base64 favicons survive for GUI rendering.
    pub url_meta: Option<String>,
    /// Optional structured metadata for presentation layers.
    ///
    /// Tools can attach render hints or other non-LLM metadata here without
    /// forcing frontends to infer meaning from `result_content`.
    pub metadata: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Turn progress events (for real-time observability)
// ---------------------------------------------------------------------------

/// Real-time progress event emitted during a turn's tool-call loop.
///
/// Presentation layers subscribe to these events via `execute_turn_with_progress`
/// and translate them into their native event format (Tauri events, SSE, TUI
/// redraws, etc.). No business logic should live in the presentation layer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    /// Emitted after each LLM response.
    LlmResponse {
        /// 1-based iteration counter.
        iteration: usize,
        /// Model that served this iteration.
        model: String,
        /// Input tokens for this call.
        input_tokens: u64,
        /// Output tokens for this call.
        output_tokens: u64,
        /// LLM call wall-clock duration (ms).
        duration_ms: u64,
        /// Cost for this single call (USD).
        cost_usd: f64,
        /// Names of tool calls requested by the LLM (empty if pure text).
        tool_calls_requested: Vec<String>,
        /// Serialised messages sent to the LLM (full JSON payload).
        prompt_preview: String,
        /// Assistant text returned by the LLM (or tool-call placeholder).
        response_text: String,
        /// Context window size of the serving provider (tokens).
        context_window: usize,
        /// Name of the agent that produced this event (e.g. `"chat-turn"`,
        /// `"title-generator"`). Allows presentation layers to distinguish
        /// root agent calls from subagent calls.
        agent_name: String,
    },
    /// Emitted after each tool execution.
    ToolResult {
        /// Tool name.
        name: String,
        /// Whether the tool succeeded.
        success: bool,
        /// Tool execution wall-clock duration (ms).
        duration_ms: u64,
        /// Serialised tool arguments (input sent to the tool).
        input_preview: String,
        /// First 500 chars of the result content.
        result_preview: String,
        /// Name of the agent that executed this tool call.
        agent_name: String,
        /// Compact URL metadata for Browser/WebFetch tools (JSON with url,
        /// title, `favicon_url`). Extracted from the full result before
        /// truncation so base64 favicons survive. `None` for non-URL tools.
        url_meta: Option<String>,
        /// Optional structured metadata for presentation layers.
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<serde_json::Value>,
    },
    /// Emitted when the tool-call loop limit is hit.
    LoopLimitHit {
        /// Number of iterations attempted.
        iterations: usize,
        /// Maximum allowed.
        max_iterations: usize,
    },
    /// Incremental text delta from the LLM stream.
    ///
    /// Emitted during streaming so presentation layers can display text
    /// as it arrives (typewriter effect). Only sent when a progress
    /// channel is provided (i.e., `execute_turn_with_progress`).
    StreamDelta {
        /// Incremental text content from the LLM.
        content: String,
        /// Name of the agent that produced this delta.
        agent_name: String,
    },
    /// Incremental reasoning/thinking delta from a thinking-mode LLM.
    ///
    /// Emitted during streaming for models that produce `reasoning_content`
    /// (e.g. DeepSeek-R1, `QwQ`). Presentation layers show this in a collapsible
    /// "Thinking..." section.
    StreamReasoningDelta {
        /// Incremental reasoning text from the LLM.
        content: String,
        /// Name of the agent that produced this delta.
        agent_name: String,
    },
    /// Partial image data from a streaming LLM response.
    StreamImageDelta {
        /// Image block index within the response.
        index: usize,
        /// MIME type of the image.
        mime_type: String,
        /// Partial base64 data to append.
        partial_data: String,
        /// Name of the agent that produced this image delta.
        agent_name: String,
    },
    /// A complete image has been generated.
    StreamImageComplete {
        /// Image block index within the response.
        index: usize,
        /// MIME type of the image.
        mime_type: String,
        /// Full base64 image data.
        data: String,
        /// Name of the agent that produced this image.
        agent_name: String,
    },
    /// Emitted when an LLM call fails (API error, network error, etc.).
    ///
    /// Allows presentation layers to show the failed call in the diagnostics
    /// timeline even when no successful `LlmResponse` was produced.
    LlmError {
        /// 1-based iteration counter where the error occurred.
        iteration: usize,
        /// Human-readable error description.
        error: String,
        /// LLM call wall-clock duration (ms) before the error was returned.
        duration_ms: u64,
        /// Model that was being called when the error occurred.
        model: String,
        /// Serialised messages sent to the LLM (full JSON payload).
        prompt_preview: String,
        /// Context window size of the serving provider (tokens).
        context_window: usize,
        /// Name of the agent where the error occurred.
        agent_name: String,
    },
    /// Emitted when the LLM calls `AskUser` and user input is needed.
    ///
    /// The presentation layer should render the questions and deliver answers
    /// back via [`PendingInteractions`] (GUI: `chat_answer_question` command;
    /// CLI: inline text selection).
    ///
    /// The tool execution loop is blocked while waiting for the answer.
    UserInteractionRequest {
        /// Unique interaction ID for correlating the answer.
        interaction_id: String,
        /// The structured questions from the `AskUser` tool call (JSON array).
        questions: serde_json::Value,
    },
    /// Emitted when a tool requires permission approval before execution.
    ///
    /// The presentation layer should render a permission prompt (inline card
    /// in GUI, terminal prompt in CLI) and deliver the response via
    /// [`PendingPermissions`].
    ///
    /// The tool execution loop is blocked while waiting for approval.
    PermissionRequest {
        /// Unique request ID for correlating the response.
        request_id: String,
        /// Tool name requesting permission.
        tool_name: String,
        /// Human-readable description of the action.
        action_description: String,
        /// Why permission is required (rule, dangerous, etc.).
        reason: String,
        /// Optional content preview (e.g., shell command, file path).
        content_preview: Option<String>,
    },
}

/// Channel sender for turn progress events.
pub type TurnEventSender = mpsc::UnboundedSender<TurnEvent>;

/// User decision from the permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPromptResponse {
    /// Approve only the current tool call.
    Approve,
    /// Deny only the current tool call.
    Deny,
    /// Approve the current tool call and bypass future permission prompts
    /// for the rest of the session.
    AllowAllForSession,
}

/// Shared map of pending user-interaction answer channels.
///
/// When `AskUser` is intercepted, the orchestrator inserts a `oneshot::Sender`
/// keyed by `interaction_id`. The presentation layer calls `chat_answer_question`,
/// which removes the sender and delivers the answer.
pub type PendingInteractions = std::sync::Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>,
    >,
>;

/// Shared map of pending permission-approval channels.
///
/// When a tool requires HITL permission, the executor inserts a `oneshot::Sender`
/// keyed by `request_id`. The presentation layer calls `chat_answer_permission`,
/// which removes the sender and delivers the user's decision.
pub type PendingPermissions = std::sync::Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<PermissionPromptResponse>>,
    >,
>;

/// Successful result of [`ChatService::execute_turn`].
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// Final assistant text content.
    pub content: String,
    /// Model that served the final request.
    pub model: String,
    /// Provider ID that served the final request.
    pub provider_id: Option<String>,
    /// Cumulative input tokens across all LLM iterations.
    pub input_tokens: u64,
    /// Cumulative output tokens across all LLM iterations.
    pub output_tokens: u64,
    /// Input tokens from the **last** LLM iteration (actual context occupancy).
    pub last_input_tokens: u64,
    /// Context window size of the serving provider.
    pub context_window: usize,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Tool calls executed during this turn.
    pub tool_calls_executed: Vec<ToolCallRecord>,
    /// Number of LLM iterations (>1 when tool loop occurs).
    pub iterations: usize,
    /// Images generated by the final assistant response.
    pub generated_images: Vec<GeneratedImage>,
    /// Messages generated during this turn (assistant + tool messages).
    pub new_messages: Vec<Message>,
}

/// Error returned by [`ChatService::execute_turn`].
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    /// LLM request failed.
    #[error("LLM error: {0}")]
    LlmError(String),
    /// Context assembly failed.
    #[error("Context error: {0}")]
    ContextError(String),
    /// Tool-call iteration limit exceeded.
    #[error("Tool call loop limit ({max_iterations}) exceeded")]
    ToolLoopLimitExceeded {
        /// Maximum allowed iterations.
        max_iterations: usize,
    },
    /// Tool-call count limit exceeded.
    #[error("Tool call limit ({max_tool_calls}) exceeded")]
    ToolCallLimitExceeded {
        /// Maximum allowed tool calls.
        max_tool_calls: usize,
    },
    /// The turn was explicitly cancelled by the caller.
    #[error("Cancelled")]
    Cancelled,
}

impl From<AgentExecutionError> for TurnError {
    fn from(err: AgentExecutionError) -> Self {
        match err {
            AgentExecutionError::LlmError { message, .. } => TurnError::LlmError(message),
            AgentExecutionError::ContextError(msg) => TurnError::ContextError(msg),
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations } => {
                TurnError::ToolLoopLimitExceeded { max_iterations }
            }
            AgentExecutionError::ToolCallLimitExceeded { max_tool_calls } => {
                TurnError::ToolCallLimitExceeded { max_tool_calls }
            }
            AgentExecutionError::Cancelled { .. } => TurnError::Cancelled,
        }
    }
}

/// Input for a turn execution.
pub struct TurnInput<'a> {
    /// The raw user input text.
    pub user_input: &'a str,
    /// Session ID for persistence.
    pub session_id: SessionId,
    /// Session UUID for diagnostics tracing.
    pub session_uuid: Uuid,
    /// Current conversation history (read-only; new messages are returned in `TurnResult`).
    pub history: &'a [Message],
    /// Current turn number for checkpoint creation.
    pub turn_number: u32,
    /// User-selected provider ID. None means auto (pool assigns).
    pub provider_id: Option<String>,
    /// High-level request mode for the turn.
    pub request_mode: RequestMode,
    /// Knowledge collection names selected by the user via slash command.
    pub knowledge_collections: Vec<String>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Plan mode: `"fast"` (default), `"auto"`, or `"plan"`.
    /// Controls whether plan-mode prompts are injected and whether a
    /// complexity-assessment sub-agent runs before the main turn.
    pub plan_mode: Option<String>,
    /// Human-readable execution label for diagnostics and progress events.
    pub agent_name: String,
    /// Whether tool calling is enabled for this turn.
    pub toolcall_enabled: bool,
    /// Preferred model identifiers for provider routing.
    pub preferred_models: Vec<String>,
    /// Provider routing tags for provider selection.
    pub provider_tags: Vec<String>,
    /// Temperature override for the turn.
    pub temperature: Option<f64>,
    /// Maximum completion tokens for the turn.
    pub max_completion_tokens: Option<u32>,
    /// Agent-specific maximum loop iterations.
    pub max_iterations: Option<usize>,
    /// Agent-specific maximum tool calls.
    pub max_tool_calls: Option<usize>,
    /// Trust tier of the bound agent, if any.
    pub trust_tier: Option<TrustTier>,
    /// Tools allowed by the bound agent.
    pub agent_allowed_tools: Vec<String>,
    /// Whether to prune historical tool results for the bound agent.
    pub prune_tool_history: bool,
    /// Effective MCP mode for this turn.
    pub mcp_mode: Option<String>,
    /// Effective MCP server selection for `"manual"` mode.
    pub mcp_servers: Vec<String>,
}
pub type TurnCancellationToken = CancellationToken;

/// Session-bound agent settings resolved from an `AgentDefinition`.
#[derive(Debug, Clone)]
pub struct SessionAgentFeatures {
    pub toolcall: bool,
    pub skills: bool,
    pub knowledge: bool,
}

#[derive(Debug, Clone)]
pub struct SessionAgentConfig {
    pub agent_id: String,
    pub agent_name: String,
    pub agent_mode: String,
    pub working_directory: Option<String>,
    pub features: SessionAgentFeatures,
    pub allowed_tools: Vec<String>,
    pub preset_skills: Vec<String>,
    pub knowledge_collections: Vec<String>,
    pub prompt_section_ids: Vec<String>,
    pub system_prompt: Option<String>,
    pub provider_id: Option<String>,
    pub preferred_models: Vec<String>,
    pub provider_tags: Vec<String>,
    pub temperature: Option<f64>,
    pub max_completion_tokens: Option<u32>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub trust_tier: TrustTier,
    pub prune_tool_history: bool,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Turn preparation (session resolve + message persist + TurnInput assembly)
// ---------------------------------------------------------------------------

/// Request to prepare a chat turn before execution.
#[derive(Debug, Default)]
pub struct PrepareTurnRequest {
    /// Existing session ID (`None` = create a new `Main` session).
    pub session_id: Option<SessionId>,
    /// User message text.
    pub user_input: String,
    /// Provider to route to (`None` = default routing).
    pub provider_id: Option<String>,
    /// High-level request mode for this turn.
    pub request_mode: Option<RequestMode>,
    /// Skill names attached to this message by the user.
    pub skills: Option<Vec<String>>,
    /// Knowledge collection names selected by the user.
    ///
    /// When non-empty, the context pipeline will perform embedding +
    /// semantic search against these collections.  When empty, knowledge
    /// retrieval is skipped entirely.
    pub knowledge_collections: Option<Vec<String>>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Additional metadata to attach to the user message (e.g. attachments).
    /// Merged into the `Message.metadata` field during `prepare_turn()`.
    pub user_message_metadata: Option<serde_json::Value>,
    /// Plan mode: `"fast"`, `"auto"`, or `"plan"` (`None` = `"fast"`).
    pub plan_mode: Option<String>,
    /// MCP mode: `"auto"`, `"manual"`, or `"disabled"` (`None` = agent/default).
    pub mcp_mode: Option<String>,
    /// MCP server names selected when `mcp_mode == "manual"`.
    pub mcp_servers: Option<Vec<String>>,
}

/// Fully resolved turn data, ready for `execute_turn()` or
/// `execute_turn_with_progress()`.
///
/// Owns all data needed for turn execution so callers do not need to
/// manage lifetimes of intermediate results (history, `session_uuid`, etc.).
#[derive(Debug)]
pub struct PreparedTurn {
    /// The resolved (or newly created) session ID.
    pub session_id: SessionId,
    /// UUID form of the session ID (for diagnostics tracing).
    pub session_uuid: Uuid,
    /// Full transcript history (includes the just-appended user message).
    pub history: Vec<Message>,
    /// Turn number derived from history length.
    pub turn_number: u32,
    /// User input (owned copy).
    pub user_input: String,
    /// Provider routing preference.
    pub provider_id: Option<String>,
    /// High-level request mode for the turn.
    pub request_mode: RequestMode,
    /// Whether this was a newly created session.
    pub session_created: bool,
    /// Knowledge collection names selected by the user.
    pub knowledge_collections: Vec<String>,
    /// Thinking/reasoning configuration.
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Plan mode: `"fast"`, `"auto"`, or `"plan"` (`None` = `"fast"`).
    pub plan_mode: Option<String>,
    /// Effective MCP mode (resolved from request or agent config).
    pub mcp_mode: Option<String>,
    /// Effective MCP server names selected for `"manual"` mode.
    pub mcp_servers: Vec<String>,
    /// Effective skills active for this turn.
    pub skills: Vec<String>,
    /// Bound session-agent settings, if this session belongs to an agent preset.
    pub agent_config: Option<SessionAgentConfig>,
}

impl PreparedTurn {
    /// Build a borrowing [`TurnInput`] from this prepared turn.
    pub fn as_turn_input(&self) -> TurnInput<'_> {
        let agent_name = self.agent_config.as_ref().map_or_else(
            || "chat-turn".to_string(),
            |config| config.agent_name.clone(),
        );
        TurnInput {
            user_input: &self.user_input,
            session_id: self.session_id.clone(),
            session_uuid: self.session_uuid,
            history: &self.history,
            turn_number: self.turn_number,
            provider_id: self.provider_id.clone(),
            request_mode: self.request_mode,
            knowledge_collections: self.knowledge_collections.clone(),
            thinking: self.thinking.clone(),
            plan_mode: self.plan_mode.clone(),
            agent_name,
            toolcall_enabled: self
                .agent_config
                .as_ref()
                .is_none_or(|config| config.features.toolcall),
            preferred_models: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.preferred_models.clone()),
            provider_tags: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.provider_tags.clone()),
            temperature: self
                .agent_config
                .as_ref()
                .and_then(|config| config.temperature),
            max_completion_tokens: self
                .agent_config
                .as_ref()
                .and_then(|config| config.max_completion_tokens),
            max_iterations: self
                .agent_config
                .as_ref()
                .map(|config| config.max_iterations),
            max_tool_calls: self
                .agent_config
                .as_ref()
                .map(|config| config.max_tool_calls),
            trust_tier: self.agent_config.as_ref().map(|config| config.trust_tier),
            agent_allowed_tools: self
                .agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.allowed_tools.clone()),
            prune_tool_history: self
                .agent_config
                .as_ref()
                .is_some_and(|config| config.prune_tool_history),
            mcp_mode: self.mcp_mode.clone(),
            mcp_servers: self.mcp_servers.clone(),
        }
    }
}

/// Errors that can occur during turn preparation.
#[derive(Debug, thiserror::Error)]
pub enum PrepareTurnError {
    /// The requested session was not found.
    #[error("session not found: {0}")]
    SessionNotFound(String),
    /// Failed to create a new session.
    #[error("failed to create session: {0}")]
    SessionCreationFailed(String),
    /// Failed to persist the user message to the session transcript.
    #[error("failed to persist user message: {0}")]
    PersistFailed(String),
    /// Failed to read the session transcript.
    #[error("failed to read transcript: {0}")]
    TranscriptReadFailed(String),
    /// A session-bound agent could not be resolved.
    #[error("session agent not found: {0}")]
    SessionAgentNotFound(String),
    /// Session exceeded the configured user-turn limit for its bound agent.
    #[error("session turn limit reached for agent '{agent_id}' ({max_turns} turns)")]
    SessionTurnLimitReached { agent_id: String, max_turns: usize },
}

// ---------------------------------------------------------------------------
// Resend-turn preparation
// ---------------------------------------------------------------------------

/// Request to prepare a resend (keep user message, remove assistant reply,
/// rebuild turn context).
#[derive(Debug)]
pub struct ResendTurnRequest {
    /// Session to resend in.
    pub session_id: SessionId,
    /// Checkpoint ID marking the turn boundary to resend from.
    pub checkpoint_id: String,
    /// User-selected provider ID (None = default routing).
    pub provider_id: Option<String>,
    /// Optional override for the request mode.
    pub request_mode: Option<RequestMode>,
    /// Knowledge collection names selected by the user.
    pub knowledge_collections: Option<Vec<String>>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Plan mode: `"fast"`, `"auto"`, or `"plan"` (`None` = fall back to agent config).
    pub plan_mode: Option<String>,
}

/// Errors that can occur during resend-turn preparation.
#[derive(Debug, thiserror::Error)]
pub enum ResendTurnError {
    /// The requested checkpoint was not found.
    #[error("checkpoint not found: {0}")]
    CheckpointNotFound(String),
    /// Transcript truncation failed.
    #[error("truncation failed: {0}")]
    TruncateFailed(String),
    /// Transcript is empty after truncation — nothing to resend.
    #[error("transcript empty after truncation")]
    TranscriptEmpty,
    /// Failed to read the session transcript.
    #[error("failed to read transcript: {0}")]
    TranscriptReadFailed(String),
    /// A session-bound agent could not be resolved.
    #[error("session agent not found: {0}")]
    SessionAgentNotFound(String),
}

// ---------------------------------------------------------------------------
// Turn metadata summary
// ---------------------------------------------------------------------------

/// Metadata summary of the last completed turn in a session.
///
/// Returned by [`ChatService::get_last_turn_meta`] so frontends can restore
/// status-bar information when switching between sessions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnMetaSummary {
    /// Provider that served the turn.
    pub provider_id: Option<String>,
    /// Model name.
    pub model: String,
    /// Cumulative input tokens across all LLM iterations.
    pub input_tokens: u64,
    /// Cumulative output tokens across all LLM iterations.
    pub output_tokens: u64,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Context window size of the serving provider.
    pub context_window: usize,
    /// Input tokens from the last LLM iteration (actual context occupancy).
    pub context_tokens_used: u64,
}

// ---------------------------------------------------------------------------
// ChatService
// ---------------------------------------------------------------------------

/// LLM chat turn orchestration service.
///
/// All methods are static — they accept a `&ServiceContainer` reference
/// to access domain services. This keeps the API simple and avoids
/// lifetime issues with holding container references.
pub struct ChatService;

impl ChatService {
    /// Execute a single chat turn (no progress events).
    pub async fn execute_turn(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
    ) -> Result<TurnResult, TurnError> {
        Self::execute_turn_inner(container, input, None, None).await
    }

    /// Execute a single chat turn with real-time progress events.
    ///
    /// The sender receives [`TurnEvent`] values that presentation layers
    /// can translate into Tauri events, SSE payloads, TUI redraws, etc.
    ///
    /// Pass a [`TurnCancellationToken`] to support mid-turn cancellation.
    /// When the token is cancelled the function returns `Err(TurnError::Cancelled)`
    /// as soon as it is safe to do so (typically within one LLM HTTP round-trip).
    pub async fn execute_turn_with_progress(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        progress: TurnEventSender,
        cancel: Option<TurnCancellationToken>,
    ) -> Result<TurnResult, TurnError> {
        Self::execute_turn_inner(container, input, Some(progress), cancel).await
    }

    fn build_execution_config(
        input: &TurnInput<'_>,
        tool_defs: Vec<serde_json::Value>,
        tool_calling_mode: ToolCallingMode,
        max_tool_iterations: usize,
    ) -> AgentExecutionConfig {
        let max_iterations = input
            .max_iterations
            .map_or(max_tool_iterations, |value| value.min(max_tool_iterations));

        AgentExecutionConfig {
            agent_name: input.agent_name.clone(),
            system_prompt: String::new(), // Uses context pipeline instead
            max_iterations,
            max_tool_calls: input.max_tool_calls.unwrap_or(usize::MAX),
            tool_definitions: tool_defs,
            tool_calling_mode,
            messages: input.history.to_vec(),
            provider_id: input.provider_id.clone(),
            preferred_models: input.preferred_models.clone(),
            provider_tags: input.provider_tags.clone(),
            request_mode: input.request_mode,
            temperature: input.temperature,
            max_tokens: input.max_completion_tokens,
            thinking: input.thinking.clone(),
            session_id: Some(input.session_id.clone()),
            session_uuid: input.session_uuid,
            knowledge_collections: input.knowledge_collections.clone(),
            use_context_pipeline: true,
            user_query: input.user_input.to_string(),
            external_trace_id: None,
            trust_tier: input.trust_tier,
            agent_allowed_tools: input.agent_allowed_tools.clone(),
            prune_tool_history: input.prune_tool_history,
            response_format: None,
        }
    }

    fn session_agent_config_from_definition(definition: &AgentDefinition) -> SessionAgentConfig {
        SessionAgentConfig {
            agent_id: definition.id.clone(),
            agent_name: definition.id.clone(),
            agent_mode: format!("{:?}", definition.mode).to_lowercase(),
            working_directory: definition.working_directory.clone(),
            features: SessionAgentFeatures {
                toolcall: definition.toolcall_enabled_resolved(),
                skills: definition.skills_enabled_resolved(),
                knowledge: definition.knowledge_enabled_resolved(),
            },
            allowed_tools: definition.allowed_tools.clone(),
            preset_skills: definition.skills.clone(),
            knowledge_collections: definition.knowledge_collections.clone(),
            prompt_section_ids: definition.prompt_section_ids.clone(),
            system_prompt: (!definition.system_prompt.trim().is_empty())
                .then(|| definition.system_prompt.clone()),
            provider_id: definition.provider_id.clone(),
            preferred_models: definition.preferred_models.clone(),
            provider_tags: definition.provider_tags.clone(),
            temperature: definition.temperature,
            max_completion_tokens: definition
                .max_completion_tokens
                .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            thinking: definition.thinking_config(),
            plan_mode: definition.plan_mode.clone(),
            permission_mode: definition.permission_mode,
            max_iterations: definition.max_iterations,
            max_tool_calls: definition.max_tool_calls,
            trust_tier: definition.trust_tier,
            prune_tool_history: definition.prune_tool_history,
            mcp_mode: definition.mcp_mode.clone(),
            mcp_servers: definition.mcp_servers.clone(),
        }
    }

    async fn resolve_session_agent_config(
        container: &ServiceContainer,
        session: &SessionNode,
    ) -> Result<Option<SessionAgentConfig>, String> {
        let Some(agent_id) = session.agent_id.as_ref() else {
            return Ok(None);
        };

        let registry = container.agent_registry.lock().await;
        let definition = registry
            .get(agent_id.as_str())
            .ok_or_else(|| agent_id.as_str().to_string())?;
        Ok(Some(Self::session_agent_config_from_definition(definition)))
    }

    fn resolve_turn_skills(
        requested_skills: Option<Vec<String>>,
        agent_config: Option<&SessionAgentConfig>,
        inject_preset_skills: bool,
    ) -> Vec<String> {
        let mut resolved = if agent_config.is_some_and(|config| !config.features.skills) {
            Vec::new()
        } else if inject_preset_skills {
            agent_config.map_or_else(Vec::new, |config| config.preset_skills.clone())
        } else {
            Vec::new()
        };

        if agent_config.is_some_and(|config| !config.features.skills) {
            return resolved;
        }

        for skill in requested_skills.unwrap_or_default() {
            if !resolved.contains(&skill) {
                resolved.push(skill);
            }
        }

        resolved
    }

    fn resolve_turn_knowledge(
        requested_collections: Option<Vec<String>>,
        agent_config: Option<&SessionAgentConfig>,
    ) -> Vec<String> {
        let Some(config) = agent_config else {
            return requested_collections.unwrap_or_default();
        };

        if !config.features.knowledge {
            return Vec::new();
        }

        let requested = requested_collections.unwrap_or_default();
        if requested.is_empty() {
            config.knowledge_collections.clone()
        } else {
            requested
        }
    }

    fn request_mode_from_metadata(metadata: &serde_json::Value) -> Option<RequestMode> {
        metadata
            .get("request_mode")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    /// Prepare a turn: resolve/create session, persist user message, read
    /// transcript, compute turn number, and assemble all data needed for
    /// `execute_turn()`.
    ///
    /// The returned [`PreparedTurn`] owns all intermediate data. Callers
    /// use [`PreparedTurn::as_turn_input()`] to obtain the borrowing
    /// [`TurnInput`] expected by `execute_turn*`.
    pub async fn prepare_turn(
        container: &ServiceContainer,
        request: PrepareTurnRequest,
    ) -> Result<PreparedTurn, PrepareTurnError> {
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::{generate_message_id, now};

        // 1. Resolve or create session.
        let (session, session_created) = if let Some(sid) = request.session_id {
            let session = container
                .session_manager
                .get_session(&sid)
                .await
                .map_err(|e| PrepareTurnError::SessionNotFound(e.to_string()))?;
            (session, false)
        } else {
            let session = container
                .session_manager
                .create_session(CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: None,
                })
                .await
                .map_err(|e| PrepareTurnError::SessionCreationFailed(e.to_string()))?;
            (session, true)
        };
        let session_id = session.id.clone();
        let agent_config = Self::resolve_session_agent_config(container, &session)
            .await
            .map_err(PrepareTurnError::SessionAgentNotFound)?;
        let existing_user_turns = container
            .session_manager
            .read_display_transcript(&session_id)
            .await
            .map_err(|e| PrepareTurnError::TranscriptReadFailed(e.to_string()))?
            .into_iter()
            .filter(|message| message.role == Role::User)
            .count();

        if let Some(config) = agent_config.as_ref() {
            if existing_user_turns >= config.max_iterations {
                return Err(PrepareTurnError::SessionTurnLimitReached {
                    agent_id: config.agent_id.clone(),
                    max_turns: config.max_iterations,
                });
            }
        }

        let skills = Self::resolve_turn_skills(
            request.skills,
            agent_config.as_ref(),
            existing_user_turns == 0,
        );
        let knowledge_collections =
            Self::resolve_turn_knowledge(request.knowledge_collections, agent_config.as_ref());
        let provider_id = request.provider_id.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.provider_id.clone())
        });
        let thinking = request.thinking.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.thinking.clone())
        });
        let plan_mode = request.plan_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.plan_mode.clone())
        });
        let mcp_mode = request.mcp_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.mcp_mode.clone())
        });
        let mcp_servers = request.mcp_servers.unwrap_or_else(|| {
            agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.mcp_servers.clone())
        });
        let request_mode = request.request_mode.unwrap_or_default();

        // 2. Build and persist the user message.
        let metadata = {
            let mut meta = serde_json::Map::new();
            if !skills.is_empty() {
                meta.insert("skills".into(), serde_json::json!(skills));
            }
            if let Some(extra) = &request.user_message_metadata {
                if let Some(obj) = extra.as_object() {
                    for (k, v) in obj {
                        meta.insert(k.clone(), v.clone());
                    }
                }
            }
            if request_mode != RequestMode::TextChat {
                meta.insert(
                    "request_mode".into(),
                    serde_json::to_value(request_mode).unwrap_or(serde_json::Value::Null),
                );
            }
            if meta.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::Object(meta)
            }
        };
        let user_msg = Message {
            message_id: generate_message_id(),
            role: Role::User,
            content: request.user_input.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: now(),
            metadata,
        };
        container
            .session_manager
            .append_message(&session_id, &user_msg)
            .await
            .map_err(|e| PrepareTurnError::PersistFailed(e.to_string()))?;

        // Mirror to SQLite chat_message_store so the pruning engine can
        // detect candidates. Fire-and-forget: failure here must not block
        // the turn.
        Self::mirror_to_chat_message_store(
            container,
            &session_id,
            &user_msg,
            None, // no model for user messages
            None,
            None,
            None,
            None,
        )
        .await;

        // 2b. File history snapshot (rewind support).
        //     Ensure a FileHistoryManager exists for this session, then
        //     create a snapshot at this user-message boundary. If the
        //     manager cannot be created, log and continue (rewind is
        //     best-effort, not turn-blocking).
        if let Err(e) = crate::rewind::RewindService::ensure_manager(
            &container.file_history_managers,
            &session_id,
            &container.data_dir,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to initialize file history manager");
        }
        crate::rewind::RewindService::make_snapshot(
            &container.file_history_managers,
            &session_id,
            &user_msg.message_id,
        )
        .await;

        // 3. Read class transcript for LLM context (may be compacted).
        //    The context transcript is the source of truth for what the LLM
        //    sees. After compaction, older messages are replaced by a summary
        //    system message, so the LLM receives a shorter history.
        let history = container
            .session_manager
            .read_transcript(&session_id)
            .await
            .map_err(|e| PrepareTurnError::TranscriptReadFailed(e.to_string()))?;

        // 4. Derive turn number from the *display* transcript length (which is
        //    never compacted) so checkpoint bookkeeping stays consistent.
        let display_len = container
            .session_manager
            .read_display_transcript(&session_id)
            .await
            .map(|t| t.len())
            .unwrap_or(history.len());
        let turn_number = u32::try_from(display_len).unwrap_or(u32::MAX);
        let session_uuid = Uuid::parse_str(session_id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        Ok(PreparedTurn {
            session_id,
            session_uuid,
            history,
            turn_number,
            user_input: request.user_input,
            provider_id,
            request_mode,
            session_created,
            knowledge_collections,
            thinking,
            plan_mode,
            mcp_mode,
            mcp_servers,
            skills,
            agent_config,
        })
    }

    /// Prepare a resend turn: keep the original user message, truncate the
    /// assistant reply + tool messages, invalidate newer checkpoints, and
    /// return a [`PreparedTurn`] ready for execution.
    ///
    /// This mirrors [`Self::prepare_turn`] but for the resend case where no new
    /// user message is appended — the existing one is reused.
    pub async fn prepare_resend_turn(
        container: &ServiceContainer,
        request: ResendTurnRequest,
    ) -> Result<PreparedTurn, ResendTurnError> {
        let session = container
            .session_manager
            .get_session(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;
        let agent_config = Self::resolve_session_agent_config(container, &session)
            .await
            .map_err(ResendTurnError::SessionAgentNotFound)?;

        // 1. Load the checkpoint to find message_count_before.
        let checkpoint = container
            .chat_checkpoint_manager
            .checkpoint_store()
            .load(&request.checkpoint_id)
            .await
            .map_err(|e| ResendTurnError::CheckpointNotFound(e.to_string()))?;

        // 2. Partial truncation: keep user message (message_count_before + 1),
        //    remove assistant reply and any tool messages after it.
        let keep_count = checkpoint.message_count_before as usize + 1;
        container
            .session_manager
            .display_transcript_store()
            .truncate(&request.session_id, keep_count)
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;
        container
            .session_manager
            .transcript_store()
            .truncate(&request.session_id, keep_count)
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;

        // 3. Invalidate this checkpoint and all newer ones.
        container
            .chat_checkpoint_manager
            .checkpoint_store()
            .invalidate_after(
                &request.session_id,
                checkpoint.turn_number.saturating_sub(1),
            )
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;

        // 4. Read context transcript (may be compacted) for LLM messages.
        let history = container
            .session_manager
            .read_transcript(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;

        if history.is_empty() {
            return Err(ResendTurnError::TranscriptEmpty);
        }

        // The last message after truncation must be the original user message.
        let Some(last_msg) = history.last() else {
            // Unreachable: guarded by is_empty() above.
            return Err(ResendTurnError::TranscriptEmpty);
        };
        if last_msg.role != Role::User {
            return Err(ResendTurnError::TruncateFailed(format!(
                "expected last message to be User after truncation, found {:?}",
                last_msg.role
            )));
        }
        let requested_skills = last_msg
            .metadata
            .get("skills")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            });
        let user_turns = history
            .iter()
            .filter(|message| message.role == Role::User)
            .count();
        let skills =
            Self::resolve_turn_skills(requested_skills, agent_config.as_ref(), user_turns == 1);
        let knowledge_collections =
            Self::resolve_turn_knowledge(request.knowledge_collections, agent_config.as_ref());
        let provider_id = request.provider_id.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.provider_id.clone())
        });
        let thinking = request.thinking.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.thinking.clone())
        });
        let plan_mode = request.plan_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.plan_mode.clone())
        });
        let request_mode = request
            .request_mode
            .or_else(|| Self::request_mode_from_metadata(&last_msg.metadata))
            .unwrap_or_default();
        let mcp_mode = agent_config
            .as_ref()
            .and_then(|config| config.mcp_mode.clone());
        let mcp_servers = agent_config
            .as_ref()
            .map_or_else(Vec::new, |config| config.mcp_servers.clone());
        let user_input = last_msg.content.clone();

        // Derive turn number from display transcript (never compacted) for
        // checkpoint consistency.
        let display_len = container
            .session_manager
            .read_display_transcript(&request.session_id)
            .await
            .map(|t| t.len())
            .unwrap_or(history.len());
        let turn_number = u32::try_from(display_len).unwrap_or(0);
        let session_uuid =
            Uuid::parse_str(request.session_id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        Ok(PreparedTurn {
            session_id: request.session_id,
            session_uuid,
            history,
            turn_number,
            user_input,
            provider_id,
            request_mode,
            session_created: false,
            knowledge_collections,
            thinking,
            plan_mode,
            mcp_mode,
            mcp_servers,
            skills,
            agent_config,
        })
    }

    /// Look up metadata for the last completed LLM turn in a session.
    ///
    /// Queries the diagnostics store for the most recent trace belonging to
    /// the session, extracts the model from the last Generation observation,
    /// and resolves `context_window` from the provider pool by model match.
    ///
    /// Returns `None` if no trace data exists for this session.
    pub async fn get_last_turn_meta(
        container: &ServiceContainer,
        session_id: &str,
    ) -> Result<Option<TurnMetaSummary>, String> {
        let Ok(session_uuid) = Uuid::parse_str(session_id) else {
            return Ok(None);
        };

        let store = container.diagnostics.store();
        let traces = store
            .list_traces_by_session(&session_uuid.to_string(), 1)
            .await
            .unwrap_or_default();

        let Some(trace) = traces.first() else {
            return Ok(None);
        };

        let observations = store.get_observations(trace.id).await.unwrap_or_default();
        let last_gen = observations
            .iter()
            .rev()
            .find(|o| o.obs_type == y_diagnostics::ObservationType::Generation);

        let model = last_gen.and_then(|o| o.model.clone()).unwrap_or_default();
        let last_gen_input_tokens = last_gen.map_or(0, |o| o.input_tokens);

        let pool = container.provider_pool().await;
        let metadata_list = pool.list_metadata();
        let matched = metadata_list.iter().find(|m| m.model == model);
        let context_window = matched.map_or(0, |m| m.context_window);
        let provider_id = matched.map(|m| m.id.to_string());

        Ok(Some(TurnMetaSummary {
            provider_id,
            model,
            input_tokens: trace.total_input_tokens,
            output_tokens: trace.total_output_tokens,
            cost_usd: trace.total_cost_usd,
            context_window,
            context_tokens_used: last_gen_input_tokens,
        }))
    }

    /// Internal implementation shared by both entry points.
    ///
    /// Delegates the entire LLM + tool execution loop to [`AgentService::execute`],
    /// then handles session-specific post-processing (message persistence,
    /// checkpointing, metadata enrichment).
    async fn execute_turn_inner(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        progress: Option<TurnEventSender>,
        cancel: Option<TurnCancellationToken>,
    ) -> Result<TurnResult, TurnError> {
        use crate::agent_service::AgentService;

        // 1. Build tool definitions (all tools for root agent).
        //
        // Tool definitions are always built regardless of mode. In Native mode
        // they are sent via the provider's API; in PromptBased mode the provider
        // ignores them (they are injected into the prompt instead). The fallback
        // path in agent_service handles both cases.
        // Resolve tool_calling_mode from the provider that will serve this
        // request. When the user selects a specific provider_id, use its mode;
        // otherwise fall back to the first available provider's mode or the
        // enum default (Native).
        let tool_calling_mode = {
            let pool = container.provider_pool().await;
            let metadata_list = pool.list_metadata();
            if let Some(ref pid) = input.provider_id {
                metadata_list
                    .iter()
                    .find(|m| m.id.to_string() == *pid)
                    .map_or(ToolCallingMode::default(), |m| m.tool_calling_mode)
            } else {
                metadata_list
                    .first()
                    .map_or(ToolCallingMode::default(), |m| m.tool_calling_mode)
            }
        };
        let mut tool_defs = if input.toolcall_enabled && input.request_mode == RequestMode::TextChat
        {
            if input.trust_tier.is_none() && input.agent_allowed_tools.is_empty() {
                Self::build_essential_tool_definitions(container).await
            } else {
                crate::agent_service::AgentService::build_filtered_tool_definitions(
                    container,
                    &input.agent_allowed_tools,
                )
                .await
            }
        } else {
            vec![]
        };

        // 1a. Apply MCP mode filtering.
        if input.request_mode == RequestMode::TextChat {
            Self::apply_mcp_mode_filter(
                &mut tool_defs,
                input.mcp_mode.as_deref(),
                &input.mcp_servers,
            );
        }

        // 1a'. Set mcp.enabled flag so the MCP hint prompt section is included.
        //
        // MCP tools live in the connection manager (not the tool registry),
        // so we check for connected MCP servers directly.
        {
            let mcp_mode = input.mcp_mode.as_deref().unwrap_or("auto");
            let has_mcp =
                if input.request_mode != RequestMode::ImageGeneration && mcp_mode != "disabled" {
                    container.mcp_manager.connected_count().await > 0
                } else {
                    false
                };
            let mut pctx = container.prompt_context.write().await;
            if has_mcp {
                pctx.config_flags.insert("mcp.enabled".into(), true);
            } else {
                pctx.config_flags.remove("mcp.enabled");
            }
        }

        // 1b. Inject plan_mode.active config flag based on the user's mode selection.
        //
        // - "fast" (default/None): no plan mode prompts injected.
        // - "plan": always inject plan_mode_active prompt section.
        // - "auto": run a lightweight complexity classification, inject if complex.
        {
            let plan_mode = input.plan_mode.as_deref().unwrap_or("fast");
            tracing::info!(
                plan_mode = %plan_mode,
                raw_plan_mode = ?input.plan_mode,
                "plan mode received from frontend"
            );
            match plan_mode {
                "plan" => {
                    let mut pctx = container.prompt_context.write().await;
                    if input.request_mode == RequestMode::TextChat {
                        pctx.config_flags.insert("plan_mode.active".into(), true);
                    } else {
                        pctx.config_flags.remove("plan_mode.active");
                    }
                    tracing::info!("plan_mode.active flag SET in prompt context");
                }
                "auto" => {
                    let needs_plan = input.request_mode == RequestMode::TextChat
                        && crate::plan_orchestrator::assess_complexity(
                            container,
                            input.user_input,
                            input.provider_id.as_deref(),
                        )
                        .await;
                    if needs_plan {
                        let mut pctx = container.prompt_context.write().await;
                        pctx.config_flags.insert("plan_mode.active".into(), true);
                        tracing::info!("plan_mode.active flag SET (auto: complex)");
                    } else {
                        tracing::info!("plan_mode.active flag NOT set (auto: simple)");
                    }
                }
                _ => {
                    // "fast" or unknown: ensure flag is cleared.
                    let mut pctx = container.prompt_context.write().await;
                    pctx.config_flags.remove("plan_mode.active");
                    tracing::info!("plan_mode.active flag CLEARED (fast mode)");
                }
            }
        }

        // 1c. Inject Plan tool schema when plan mode is active.
        if input.request_mode == RequestMode::TextChat {
            Self::apply_plan_mode_tool_adjustments(container, &mut tool_defs).await;
        }

        // 2. Construct execution config for the root agent.
        let max_tool_iterations = container.guardrail_manager.config().max_tool_iterations;
        let exec_config =
            Self::build_execution_config(input, tool_defs, tool_calling_mode, max_tool_iterations);

        // 3. Delegate to AgentService.
        let result = match AgentService::execute(container, &exec_config, progress, cancel).await {
            Ok(r) => r,
            Err(AgentExecutionError::LlmError {
                message,
                partial_messages,
            }) => {
                // Persist intermediate messages (assistant + tool results from
                // earlier successful iterations) so the conversation history
                // survives the error and the user can continue / retry.
                for msg in &partial_messages {
                    let _ = container
                        .session_manager
                        .append_message(&input.session_id, msg)
                        .await;
                }
                if !partial_messages.is_empty() {
                    tracing::info!(
                        count = partial_messages.len(),
                        session = %input.session_id.0,
                        "persisted partial messages before LLM error"
                    );
                }
                return Err(TurnError::LlmError(message));
            }
            Err(AgentExecutionError::Cancelled {
                partial_messages,
                accumulated_content,
                iteration_texts,
                iteration_reasonings,
                iteration_reasoning_durations_ms,
                iteration_tool_counts,
                tool_calls_executed,
                iterations,
                input_tokens,
                output_tokens,
                cost_usd,
                model,
                generated_images,
            }) => {
                // Persist intermediate messages (assistant + tool results from
                // earlier successful iterations) to the CONTEXT transcript only.
                // These are raw protocol messages (individual assistant msgs with
                // tool_calls + tool role msgs) that the LLM needs for continuity
                // on resume, but they are NOT suitable for GUI display (the
                // frontend expects a single consolidated assistant message).
                let ctx_store = container.session_manager.transcript_store();
                for msg in &partial_messages {
                    let _ = ctx_store.append(&input.session_id, msg).await;
                }

                // Build and persist a consolidated assistant message with all
                // accumulated content and metadata. This goes to BOTH transcripts
                // so the GUI can render it properly and the LLM sees the final
                // state on resume.
                if !accumulated_content.trim().is_empty() || !tool_calls_executed.is_empty() {
                    let tool_results_meta: Vec<serde_json::Value> =
                        Self::build_tool_results_metadata(&tool_calls_executed);

                    let mut meta = serde_json::json!({
                        "model": model,
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cost_usd": cost_usd,
                        "tool_results": tool_results_meta,
                        "iteration_texts": iteration_texts,
                        "iteration_reasonings": iteration_reasonings,
                        "iteration_reasoning_durations_ms": iteration_reasoning_durations_ms,
                        "iteration_tool_counts": iteration_tool_counts,
                        "cancelled": true,
                    });

                    if !generated_images.is_empty() {
                        meta["generated_images"] = serde_json::to_value(&generated_images)
                            .unwrap_or(serde_json::Value::Array(vec![]));
                    }

                    let assistant_msg = Message {
                        message_id: y_core::types::generate_message_id(),
                        role: Role::Assistant,
                        content: accumulated_content.clone(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: meta,
                    };

                    // Display transcript: consolidated message for GUI rendering.
                    let _ = container
                        .session_manager
                        .display_transcript_store()
                        .append(&input.session_id, &assistant_msg)
                        .await;

                    // Context transcript: consolidated message for LLM context.
                    let _ = ctx_store.append(&input.session_id, &assistant_msg).await;
                }

                if !partial_messages.is_empty() || !accumulated_content.trim().is_empty() {
                    tracing::info!(
                        partial_count = partial_messages.len(),
                        accumulated_len = accumulated_content.len(),
                        iterations,
                        session = %input.session_id.0,
                        "persisted partial state on cancellation"
                    );
                }

                // No checkpoint or post-turn optimization for cancelled turns.
                return Err(TurnError::Cancelled);
            }
            Err(e) => return Err(TurnError::from(e)),
        };

        // 4. Session-specific post-processing: persist final assistant message,
        //    create checkpoint. AgentService doesn't handle session storage —
        //    that's the ChatService's responsibility.

        // Build tool_results metadata for frontend rendering after session reload.
        let tool_results_meta: Vec<serde_json::Value> =
            Self::build_tool_results_metadata(&result.tool_calls_executed);

        let mut meta = serde_json::json!({
            "model": result.model,
            "input_tokens": result.input_tokens,
            "output_tokens": result.output_tokens,
            "cost_usd": result.cost_usd,
            "tool_results": tool_results_meta,
            "context_window": result.context_window,
            "context_tokens_used": result.last_input_tokens,
            "final_response": result.final_response,
            "iteration_texts": result.iteration_texts,
            "iteration_reasonings": result.iteration_reasonings,
            "iteration_reasoning_durations_ms": result.iteration_reasoning_durations_ms,
            "iteration_tool_counts": result.iteration_tool_counts,
        });

        if !result.generated_images.is_empty() {
            meta["generated_images"] = serde_json::to_value(&result.generated_images)
                .unwrap_or(serde_json::Value::Array(vec![]));
        }

        // Preserve reasoning_content: prefer the direct field (always available),
        // then fall back to scanning new_messages (for multi-iteration cases where
        // reasoning was produced in an earlier iteration).
        if let Some(ref rc) = result.reasoning_content {
            meta["reasoning_content"] = serde_json::Value::String(rc.clone());
        } else if let Some(last_assistant) = result
            .new_messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
        {
            if let Some(rc) = last_assistant.metadata.get("reasoning_content") {
                meta["reasoning_content"] = rc.clone();
            }
        }

        // Persist reasoning/thinking duration so the frontend can show it
        // after page reload (without relying on client-side timestamps).
        if let Some(rd) = result.reasoning_duration_ms {
            meta["reasoning_duration_ms"] = serde_json::json!(rd);
        }

        let assistant_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: result.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: meta,
        };

        if let Err(e) = container
            .session_manager
            .append_message(&input.session_id, &assistant_msg)
            .await
        {
            tracing::warn!(
                error = %e,
                session_id = %input.session_id,
                "failed to persist assistant message to session transcript"
            );
        }

        // Mirror to SQLite chat_message_store for pruning engine visibility.
        Self::mirror_to_chat_message_store(
            container,
            &input.session_id,
            &assistant_msg,
            Some(&result.model),
            Some(result.input_tokens),
            Some(result.output_tokens),
            Some(result.cost_usd),
            Some(result.context_window),
        )
        .await;

        let mut new_messages = result.new_messages.clone();
        new_messages.push(assistant_msg);

        // Checkpoint.
        let msg_count_before = u32::try_from(input.history.len().saturating_sub(1)).unwrap_or(0);
        let turn = input.turn_number + 1;
        let scope_id = format!("turn-{}-{}", input.session_id.0, turn);
        if let Err(e) = container
            .chat_checkpoint_manager
            .create_checkpoint(&input.session_id, turn, msg_count_before, scope_id)
            .await
        {
            tracing::warn!(error = %e, "failed to create chat checkpoint");
        }

        // Post-turn context optimization (pruning + conditional compaction).
        if let Err(e) = crate::context_optimization::ContextOptimizationService::optimize_post_turn(
            container,
            &input.session_id,
            result.context_window,
        )
        .await
        {
            tracing::warn!(error = %e, "post-turn context optimization failed");
        }

        Ok(TurnResult {
            content: result.content,
            model: result.model,
            provider_id: result.provider_id,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            last_input_tokens: result.last_input_tokens,
            context_window: result.context_window,
            cost_usd: result.cost_usd,
            tool_calls_executed: result.tool_calls_executed,
            iterations: result.iterations,
            generated_images: result.generated_images,
            new_messages,
        })
    }

    /// Build tool results metadata for persisting in assistant message metadata.
    ///
    /// Shared by the normal completion path and the cancellation persistence
    /// path to avoid duplicating the URL metadata extraction logic.
    fn build_tool_results_metadata(
        tool_calls: &[crate::agent_service::ToolCallRecord],
    ) -> Vec<serde_json::Value> {
        tool_calls
            .iter()
            .map(|tc| {
                let mut entry = serde_json::json!({
                    "name": tc.name,
                    "arguments": tc.arguments,
                    "success": tc.success,
                    "duration_ms": tc.duration_ms,
                    "result_preview": &tc.result_content,
                });
                // Use pre-extracted url_meta directly (survives result
                // stripping for Browser/WebFetch tools).
                if let Some(ref meta_str) = tc.url_meta {
                    if let Ok(meta_val) = serde_json::from_str::<serde_json::Value>(meta_str) {
                        entry["url_meta"] = meta_val;
                    }
                }
                if let Some(ref meta) = tc.metadata {
                    entry["metadata"] = meta.clone();
                }
                entry
            })
            .collect()
    }

    /// Adjust tool definitions for plan mode.
    ///
    /// When `plan_mode.active` is set in the prompt context, injects the
    /// `Plan` tool schema so the LLM can trigger the planning workflow.
    /// Unlike the old system, no tools are blocked -- the Plan tool
    /// orchestrator handles everything via sub-agent delegation.
    async fn apply_plan_mode_tool_adjustments(
        container: &ServiceContainer,
        tool_defs: &mut Vec<serde_json::Value>,
    ) {
        let is_active = {
            let pctx = container.prompt_context.read().await;
            pctx.config_flags
                .get("plan_mode.active")
                .copied()
                .unwrap_or(false)
        };
        if !is_active {
            return;
        }

        // Inject Plan tool schema if not already present.
        let already_present = tool_defs.iter().any(|def| {
            def.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("Plan")
        });
        if already_present {
            return;
        }

        let tn = y_core::types::ToolName::from_string("Plan");
        if let Some(def) = container.tool_registry.get_definition(&tn).await {
            tool_defs.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": def.name.as_str(),
                    "description": def.description,
                    "parameters": def.parameters,
                }
            }));
        }

        tracing::info!(
            final_count = tool_defs.len(),
            "plan mode: injected Plan tool schema"
        );
    }

    /// Filter MCP tool definitions according to the user's MCP mode.
    ///
    /// - `"auto"` (default / `None`): no filtering (all MCP tools pass through).
    /// - `"manual"`: keep only MCP tools whose server name is in `allowed_servers`.
    /// - `"disabled"`: remove every tool whose name starts with the `mcp_` prefix.
    ///
    /// Non-MCP tools (no `mcp_` prefix) are never removed.
    fn apply_mcp_mode_filter(
        tool_defs: &mut Vec<serde_json::Value>,
        mcp_mode: Option<&str>,
        allowed_servers: &[String],
    ) {
        let mode = mcp_mode.unwrap_or("auto");
        if mode == "auto" {
            return;
        }

        let before = tool_defs.len();
        tool_defs.retain(|def| {
            let name = def
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let Some((server, _)) = y_tools::mcp_integration::split_qualified_tool_name(name)
            else {
                return true;
            };
            match mode {
                "disabled" => false,
                "manual" => allowed_servers.iter().any(|s| s == server),
                _ => true,
            }
        });

        tracing::info!(
            mcp_mode = mode,
            before = before,
            after = tool_defs.len(),
            "mcp mode filter applied"
        );
    }

    /// Build LLM messages by prepending system prompt from assembled context.
    ///
    /// Delegates to [`crate::message_builder::build_chat_messages`].
    pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
        crate::message_builder::build_chat_messages(assembled, history)
    }

    /// Build tool definitions in `OpenAI` function-calling JSON format.
    ///
    /// Returns definitions for ALL registered tools. Prefer
    /// [`Self::build_essential_tool_definitions`] for root agent turns to enforce
    /// lazy loading.
    pub async fn build_tool_definitions(container: &ServiceContainer) -> Vec<serde_json::Value> {
        let defs = container.tool_registry.get_all_definitions().await;
        defs.iter()
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                })
            })
            .collect()
    }

    /// Build tool definitions for essential tools only (lazy loading).
    ///
    /// Returns definitions for `ESSENTIAL_TOOL_NAMES` -- the minimal set
    /// required for every LLM call. Additional tools are injected
    /// dynamically after `ToolSearch` activates them.
    pub async fn build_essential_tool_definitions(
        container: &ServiceContainer,
    ) -> Vec<serde_json::Value> {
        use crate::container::ESSENTIAL_TOOL_NAMES;

        let mut defs = Vec::with_capacity(ESSENTIAL_TOOL_NAMES.len());
        for &name in ESSENTIAL_TOOL_NAMES {
            if let Some(def) = container
                .tool_registry
                .get_definition(&y_core::types::ToolName::from_string(name))
                .await
            {
                defs.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                }));
            }
        }
        defs
    }

    /// Determine whether title generation should be triggered for this turn.
    ///
    /// Business rule: generate a title when the session has at least one user
    /// message and (`user_msg_count == 1` OR `user_msg_count` is a multiple of
    /// `title_summarize_interval`). Disabled when `title_summarize_interval` is 0.
    pub fn should_generate_title(container: &ServiceContainer, history: &[Message]) -> bool {
        let title_interval = container.session_manager.config().title_summarize_interval;
        if title_interval == 0 {
            return false;
        }
        let user_msg_count = history.iter().filter(|m| m.role == Role::User).count();
        user_msg_count > 0 && (user_msg_count == 1 || user_msg_count % title_interval as usize == 0)
    }

    /// Mirror a `Message` to the `ChatMessageStore` (`SQLite`) so that the
    /// pruning engine can detect candidates and invoke `pruning-summarizer`.
    ///
    /// This is fire-and-forget: a failure is logged but never propagated,
    /// because the JSONL transcript is the primary persistence layer.
    async fn mirror_to_chat_message_store(
        container: &ServiceContainer,
        session_id: &SessionId,
        msg: &Message,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
        context_window: Option<usize>,
    ) {
        let role_str = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        let record = ChatMessageRecord {
            id: msg.message_id.clone(),
            session_id: session_id.clone(),
            role: role_str.to_string(),
            content: msg.content.clone(),
            status: ChatMessageStatus::Active,
            checkpoint_id: None,
            model: model.map(std::string::ToString::to_string),
            input_tokens: input_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            output_tokens: output_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            cost_usd,
            context_window: context_window.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            parent_message_id: None,
            pruning_group_id: None,
            created_at: msg.timestamp,
        };

        if let Err(e) = container.chat_message_store.insert(&record).await {
            tracing::warn!(
                error = %e,
                session_id = %session_id,
                message_id = %msg.message_id,
                "failed to mirror message to chat_message_store"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use y_context::{ContextCategory, ContextItem};

    fn make_history() -> Vec<Message> {
        vec![
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: "Hello".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ]
    }

    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent, a helpful AI assistant.".to_string(),
            token_estimate: 10,
            priority: 100,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("y-agent"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[2].role, Role::Assistant);
    }

    #[test]
    fn test_build_chat_messages_no_system_when_empty() {
        let assembled = AssembledContext::default();
        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_build_chat_messages_joins_multiple_system_items() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part one".to_string(),
            token_estimate: 5,
            priority: 100,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Status,
            content: "status info".to_string(),
            token_estimate: 5,
            priority: 500,
        });
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part two".to_string(),
            token_estimate: 5,
            priority: 200,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Skills,
            content: "### Skill: code_review\nReviews code.".to_string(),
            token_estimate: 10,
            priority: 400,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 3);
        assert!(messages[0].content.contains("Part one"));
        assert!(messages[0].content.contains("Part two"));
        assert!(messages[0].content.contains("### Skill: code_review")); // Skills included
        assert!(!messages[0].content.contains("status info")); // Status excluded
    }

    #[test]
    fn test_build_chat_messages_includes_skills() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent.".to_string(),
            token_estimate: 5,
            priority: 100,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Skills,
            content: "### Skill: refactor\nRefactors code to improve structure.".to_string(),
            token_estimate: 10,
            priority: 400,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 3); // system + 2 history
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("You are y-agent."));
        assert!(messages[0].content.contains("### Skill: refactor"));
    }

    #[test]
    fn test_turn_error_display() {
        assert!(TurnError::LlmError("timeout".into())
            .to_string()
            .contains("timeout"));
        assert!(TurnError::ToolLoopLimitExceeded { max_iterations: 10 }
            .to_string()
            .contains("10"));
    }

    #[test]
    fn test_build_execution_config_preserves_none_temperature() {
        let history = make_history();
        let input = TurnInput {
            user_input: "hello",
            session_id: SessionId::from_string("session-1"),
            session_uuid: Uuid::new_v4(),
            history: &history,
            turn_number: 2,
            provider_id: None,
            request_mode: RequestMode::TextChat,
            knowledge_collections: vec![],
            thinking: None,
            plan_mode: None,
            agent_name: "chat-turn".into(),
            toolcall_enabled: true,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: None,
            max_completion_tokens: None,
            max_iterations: None,
            max_tool_calls: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: vec![],
        };

        let config =
            ChatService::build_execution_config(&input, vec![], ToolCallingMode::default(), 8);
        assert_eq!(config.temperature, None);
    }

    #[test]
    fn test_build_execution_config_preserves_explicit_temperature() {
        let history = make_history();
        let input = TurnInput {
            user_input: "hello",
            session_id: SessionId::from_string("session-1"),
            session_uuid: Uuid::new_v4(),
            history: &history,
            turn_number: 2,
            provider_id: None,
            request_mode: RequestMode::TextChat,
            knowledge_collections: vec![],
            thinking: None,
            plan_mode: None,
            agent_name: "chat-turn".into(),
            toolcall_enabled: true,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: Some(1.0),
            max_completion_tokens: None,
            max_iterations: None,
            max_tool_calls: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: vec![],
        };

        let config =
            ChatService::build_execution_config(&input, vec![], ToolCallingMode::default(), 8);
        assert_eq!(config.temperature, Some(1.0));
    }

    // -----------------------------------------------------------------------
    // prepare_turn tests
    // -----------------------------------------------------------------------

    async fn make_test_container() -> (crate::container::ServiceContainer, tempfile::TempDir) {
        let tmpdir = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let container = crate::container::ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    #[tokio::test]
    async fn prepare_turn_creates_new_session() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("prepare_turn should succeed");
        assert!(prepared.session_created);
        assert!(!prepared.session_id.as_str().is_empty());
        assert!(!prepared.history.is_empty());
    }

    #[tokio::test]
    async fn prepare_turn_resolves_existing_session() {
        use y_core::session::{CreateSessionOptions, SessionType};

        let (container, _tmp) = make_test_container().await;
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let request = PrepareTurnRequest {
            session_id: Some(session.id.clone()),
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("should resolve existing session");
        assert!(!prepared.session_created);
        assert_eq!(prepared.session_id, session.id);
    }

    #[tokio::test]
    async fn prepare_turn_invalid_session_returns_not_found() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: Some(SessionId("nonexistent-id".into())),
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let err = ChatService::prepare_turn(&container, request)
            .await
            .unwrap_err();
        assert!(matches!(err, PrepareTurnError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn prepare_turn_persists_user_message() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "test message".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();

        // History should contain at least the user message.
        let last = prepared
            .history
            .last()
            .expect("history should not be empty");
        assert_eq!(last.role, Role::User);
        assert_eq!(last.content, "test message");
    }

    #[tokio::test]
    async fn prepare_turn_persists_image_generation_request_mode() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "draw a lighthouse".into(),
            provider_id: None,
            request_mode: Some(RequestMode::ImageGeneration),
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("prepare_turn should succeed");

        let last = prepared
            .history
            .last()
            .expect("history should not be empty");
        assert_eq!(prepared.request_mode, RequestMode::ImageGeneration);
        assert_eq!(
            prepared.as_turn_input().request_mode,
            RequestMode::ImageGeneration
        );
        assert_eq!(
            last.metadata
                .get("request_mode")
                .and_then(|value| value.as_str()),
            Some("image_generation")
        );
    }

    #[tokio::test]
    async fn prepare_resend_turn_restores_request_mode_from_user_metadata() {
        let (container, _tmp) = make_test_container().await;
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "generate a skyline at dusk".into(),
                provider_id: None,
                request_mode: Some(RequestMode::ImageGeneration),
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");

        let assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "done".into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        container
            .session_manager
            .append_message(&prepared.session_id, &assistant)
            .await
            .expect("assistant message should persist");

        let checkpoint = container
            .chat_checkpoint_manager
            .create_checkpoint(&prepared.session_id, 1, 0, "scope-1".to_string())
            .await
            .expect("checkpoint should create");

        let resent = ChatService::prepare_resend_turn(
            &container,
            ResendTurnRequest {
                session_id: prepared.session_id.clone(),
                checkpoint_id: checkpoint.checkpoint_id,
                provider_id: None,
                request_mode: None,
                knowledge_collections: None,
                thinking: None,
                plan_mode: None,
            },
        )
        .await
        .expect("prepare_resend_turn should succeed");

        assert_eq!(resent.request_mode, RequestMode::ImageGeneration);
        assert_eq!(
            resent.as_turn_input().request_mode,
            RequestMode::ImageGeneration
        );
        assert_eq!(resent.history.len(), 1);
        assert_eq!(resent.history[0].role, Role::User);
    }

    #[tokio::test]
    async fn prepare_turn_as_turn_input_matches() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "hello".into(),
            provider_id: Some("test-provider".into()),
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();
        let input = prepared.as_turn_input();
        assert_eq!(input.user_input, "hello");
        assert_eq!(input.session_id, prepared.session_id);
        assert_eq!(input.session_uuid, prepared.session_uuid);
        assert_eq!(input.turn_number, prepared.turn_number);
        assert_eq!(input.provider_id, Some("test-provider".into()));
    }

    #[tokio::test]
    async fn prepare_turn_turn_number_equals_history_len() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "first".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let p1 = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();
        assert_eq!(p1.turn_number, p1.history.len() as u32);

        // Second message in same session.
        let request2 = PrepareTurnRequest {
            session_id: Some(p1.session_id.clone()),
            user_input: "second".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
        };
        let p2 = ChatService::prepare_turn(&container, request2)
            .await
            .unwrap();
        assert_eq!(p2.turn_number, p2.history.len() as u32);
        assert!(p2.turn_number > p1.turn_number);
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_applies_agent_defaults() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::provider::ThinkingEffort;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "agent-session"
name = "Agent Session"
description = "Preset-backed chat session"
mode = "general"
trust_tier = "user_defined"
system_prompt = "You are the bound agent."
provider_id = "preset-provider"
skills = ["workspace-skill"]
knowledge_enabled = true
knowledge_collections = ["project-notes"]
plan_mode = "plan"
thinking_effort = "high"
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("agent-session")),
                title: None,
            })
            .await
            .expect("session should create");

        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "hello".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect("agent session prepare_turn should succeed");

        assert_eq!(prepared.provider_id.as_deref(), Some("preset-provider"));
        assert_eq!(prepared.skills, vec!["workspace-skill"]);
        assert_eq!(prepared.knowledge_collections, vec!["project-notes"]);
        assert_eq!(prepared.plan_mode.as_deref(), Some("plan"));
        assert_eq!(
            prepared.thinking.as_ref().map(|config| config.effort),
            Some(ThinkingEffort::High)
        );
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_injects_preset_skills_only_on_first_turn() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "skill-agent"
name = "Skill Agent"
description = "Injects preset skills only once"
mode = "general"
trust_tier = "user_defined"
system_prompt = "Use the preset skill."
skills = ["workspace-skill"]
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("skill-agent")),
                title: None,
            })
            .await
            .expect("session should create");

        let first = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id.clone()),
                user_input: "first".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect("first turn should succeed");
        assert_eq!(first.skills, vec!["workspace-skill"]);

        let second = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "second".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect("second turn should succeed");
        assert!(second.skills.is_empty());
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_uses_max_iterations_as_turn_limit() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "limited-agent"
name = "Limited Agent"
description = "Single-turn session agent"
mode = "general"
trust_tier = "user_defined"
system_prompt = "One turn only."
max_iterations = 1
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("limited-agent")),
                title: None,
            })
            .await
            .expect("session should create");

        ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id.clone()),
                user_input: "first".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect("first turn should succeed");

        let err = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "second".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
            },
        )
        .await
        .expect_err("second turn should hit the session turn limit");

        assert!(matches!(
            err,
            PrepareTurnError::SessionTurnLimitReached { .. }
        ));
    }
}
