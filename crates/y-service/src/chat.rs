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

use y_context::AssembledContext;
use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore};
use y_core::types::{Message, Role, SessionId};

use crate::agent_service::AgentExecutionError;
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
    /// For Browser/WebFetch tools, this is the **stripped** result that only
    /// contains LLM-relevant fields (`url`, `title`, `text`). GUI-only
    /// metadata (`favicon_url`, `navigation`, `action`, etc.) is removed
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
    /// Knowledge collection names selected by the user via slash command.
    pub knowledge_collections: Vec<String>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Plan mode: `"fast"` (default), `"auto"`, or `"plan"`.
    /// Controls whether plan-mode prompts are injected and whether a
    /// complexity-assessment sub-agent runs before the main turn.
    pub plan_mode: Option<String>,
}
pub type TurnCancellationToken = CancellationToken;

// ---------------------------------------------------------------------------
// Turn preparation (session resolve + message persist + TurnInput assembly)
// ---------------------------------------------------------------------------

/// Request to prepare a chat turn before execution.
#[derive(Debug)]
pub struct PrepareTurnRequest {
    /// Existing session ID (`None` = create a new `Main` session).
    pub session_id: Option<SessionId>,
    /// User message text.
    pub user_input: String,
    /// Provider to route to (`None` = default routing).
    pub provider_id: Option<String>,
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
    /// Whether this was a newly created session.
    pub session_created: bool,
    /// Knowledge collection names selected by the user.
    pub knowledge_collections: Vec<String>,
    /// Thinking/reasoning configuration.
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Plan mode: `"fast"`, `"auto"`, or `"plan"` (`None` = `"fast"`).
    pub plan_mode: Option<String>,
}

impl PreparedTurn {
    /// Build a borrowing [`TurnInput`] from this prepared turn.
    pub fn as_turn_input(&self) -> TurnInput<'_> {
        TurnInput {
            user_input: &self.user_input,
            session_id: self.session_id.clone(),
            session_uuid: self.session_uuid,
            history: &self.history,
            turn_number: self.turn_number,
            provider_id: self.provider_id.clone(),
            knowledge_collections: self.knowledge_collections.clone(),
            thinking: self.thinking.clone(),
            plan_mode: self.plan_mode.clone(),
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
    /// Knowledge collection names selected by the user.
    pub knowledge_collections: Option<Vec<String>>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
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
        let (session_id, session_created) = if let Some(sid) = request.session_id {
            container
                .session_manager
                .get_session(&sid)
                .await
                .map_err(|e| PrepareTurnError::SessionNotFound(e.to_string()))?;
            (sid, false)
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
            (session.id, true)
        };

        // 2. Build and persist the user message.
        let metadata = {
            let mut meta = serde_json::Map::new();
            if let Some(skills) = &request.skills {
                if !skills.is_empty() {
                    meta.insert("skills".into(), serde_json::json!(skills));
                }
            }
            if let Some(extra) = &request.user_message_metadata {
                if let Some(obj) = extra.as_object() {
                    for (k, v) in obj {
                        meta.insert(k.clone(), v.clone());
                    }
                }
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
            provider_id: request.provider_id,
            session_created,
            knowledge_collections: request.knowledge_collections.unwrap_or_default(),
            thinking: request.thinking,
            plan_mode: request.plan_mode,
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
            provider_id: request.provider_id,
            session_created: false,
            knowledge_collections: request.knowledge_collections.unwrap_or_default(),
            thinking: request.thinking,
            plan_mode: None,
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
        use crate::agent_service::{AgentExecutionConfig, AgentExecutionError, AgentService};
        use y_core::provider::ToolCallingMode;

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
        let mut tool_defs = Self::build_essential_tool_definitions(container).await;

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
                    pctx.config_flags.insert("plan_mode.active".into(), true);
                    tracing::info!("plan_mode.active flag SET in prompt context");
                }
                "auto" => {
                    let needs_plan = crate::plan_orchestrator::assess_complexity(
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
        Self::apply_plan_mode_tool_adjustments(container, &mut tool_defs).await;

        // 2. Construct execution config for the root agent.
        let max_tool_iterations = container.guardrail_manager.config().max_tool_iterations;
        let exec_config = AgentExecutionConfig {
            agent_name: "chat-turn".to_string(),
            system_prompt: String::new(), // Uses context pipeline instead
            max_iterations: max_tool_iterations,
            tool_definitions: tool_defs,
            tool_calling_mode,
            messages: input.history.to_vec(),
            provider_id: input.provider_id.clone(),
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: Some(0.7),
            max_tokens: None,
            thinking: input.thinking.clone(),
            session_id: Some(input.session_id.clone()),
            session_uuid: input.session_uuid,
            knowledge_collections: input.knowledge_collections.clone(),
            use_context_pipeline: true,
            user_query: input.user_input.to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
        };

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

                    let meta = serde_json::json!({
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
    async fn prepare_turn_as_turn_input_matches() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "hello".into(),
            provider_id: Some("test-provider".into()),
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
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
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
        };
        let p2 = ChatService::prepare_turn(&container, request2)
            .await
            .unwrap();
        assert_eq!(p2.turn_number, p2.history.len() as u32);
        assert!(p2.turn_number > p1.turn_number);
    }
}
