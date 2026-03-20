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

use std::fmt;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_context::{AssembledContext, ContextCategory};
use y_core::types::{Message, Role, SessionId};

use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Record of a tool call executed during a turn.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallRecord {
    /// Tool name.
    pub name: String,
    /// Whether the tool executed successfully.
    pub success: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Result content (serialised JSON string).
    pub result_content: String,
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
        /// First 1 000 chars of the serialised messages sent to the LLM.
        prompt_preview: String,
        /// Assistant text returned by the LLM (or tool-call placeholder).
        response_text: String,
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
    },
    /// Incremental reasoning/thinking delta from a thinking-mode LLM.
    ///
    /// Emitted during streaming for models that produce `reasoning_content`
    /// (e.g. DeepSeek-R1, `QwQ`). Presentation layers show this in a collapsible
    /// "Thinking..." section.
    StreamReasoningDelta {
        /// Incremental reasoning text from the LLM.
        content: String,
    },
}

/// Channel sender for turn progress events.
pub type TurnEventSender = mpsc::UnboundedSender<TurnEvent>;

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
#[derive(Debug)]
pub enum TurnError {
    /// LLM request failed.
    LlmError(String),
    /// Context assembly failed.
    ContextError(String),
    /// Tool-call iteration limit exceeded.
    ToolLoopLimitExceeded {
        /// Maximum allowed iterations.
        max_iterations: usize,
    },
    /// The turn was explicitly cancelled by the caller.
    Cancelled,
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnError::LlmError(msg) => write!(f, "LLM error: {msg}"),
            TurnError::ContextError(msg) => write!(f, "Context error: {msg}"),
            TurnError::ToolLoopLimitExceeded { max_iterations } => {
                write!(f, "Tool call loop limit ({max_iterations}) exceeded")
            }
            TurnError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl std::error::Error for TurnError {}

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
}

/// Token passed to `execute_turn_with_progress` to support mid-turn cancellation.
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
}

/// A fully prepared turn, ready for `execute_turn()` or
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
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Context window size of the serving provider.
    pub context_window: usize,
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
        let metadata = match &request.skills {
            Some(skills) if !skills.is_empty() => {
                serde_json::json!({ "skills": skills })
            }
            _ => serde_json::Value::Null,
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

        // 3. Read full display transcript (includes the just-appended user message).
        //    The display transcript is used for GUI-facing history; it is never
        //    compacted, so users always see the complete conversation.
        let history = container
            .session_manager
            .read_display_transcript(&session_id)
            .await
            .map_err(|e| PrepareTurnError::TranscriptReadFailed(e.to_string()))?;

        // 4. Derive turn number and session UUID.
        let turn_number = u32::try_from(history.len()).unwrap_or(u32::MAX);
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

        // 4. Read display transcript (now ends with the original user message).
        let history = container
            .session_manager
            .read_display_transcript(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;

        if history.is_empty() {
            return Err(ResendTurnError::TranscriptEmpty);
        }

        // 5. Build PreparedTurn from the truncated transcript.
        let user_input = if let Some(msg) = history.last() {
            msg.content.clone()
        } else {
            return Err(ResendTurnError::TranscriptEmpty);
        };
        let turn_number = u32::try_from(history.len()).unwrap_or(0);
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
        let tool_calling_mode = ToolCallingMode::default();
        let tool_defs = match tool_calling_mode {
            ToolCallingMode::Native => Self::build_tool_definitions(container).await,
            ToolCallingMode::PromptBased => vec![],
        };

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
            session_id: Some(input.session_id.clone()),
            session_uuid: input.session_uuid,
            knowledge_collections: input.knowledge_collections.clone(),
            use_context_pipeline: true,
            user_query: input.user_input.to_string(),
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
            Err(AgentExecutionError::ContextError(msg)) => {
                return Err(TurnError::ContextError(msg));
            }
            Err(AgentExecutionError::ToolLoopLimitExceeded { max_iterations }) => {
                return Err(TurnError::ToolLoopLimitExceeded { max_iterations });
            }
            Err(AgentExecutionError::Cancelled) => {
                return Err(TurnError::Cancelled);
            }
        };

        // 4. Session-specific post-processing: persist final assistant message,
        //    create checkpoint. AgentService doesn't handle session storage —
        //    that's the ChatService's responsibility.

        // Build tool_results metadata for frontend rendering after session reload.
        let tool_results_meta: Vec<serde_json::Value> = result
            .tool_calls_executed
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "name": tc.name,
                    "success": tc.success,
                    "duration_ms": tc.duration_ms,
                    "result_preview": &tc.result_content[..tc.result_content.floor_char_boundary(2000)],
                })
            })
            .collect();

        let mut meta = serde_json::json!({
            "model": result.model,
            "input_tokens": result.input_tokens,
            "output_tokens": result.output_tokens,
            "cost_usd": result.cost_usd,
            "tool_results": tool_results_meta,
            "context_window": result.context_window,
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

        let assistant_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: result.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: meta,
        };

        let _ = container
            .session_manager
            .append_message(&input.session_id, &assistant_msg)
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

    /// Build LLM messages by prepending system prompt from assembled context.
    pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
        let system_parts: Vec<&str> = assembled
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.category,
                    ContextCategory::SystemPrompt
                        | ContextCategory::Skills
                        | ContextCategory::Knowledge
                )
            })
            .map(|item| item.content.as_str())
            .collect();

        let mut messages = Vec::with_capacity(history.len() + 1);

        if !system_parts.is_empty() {
            let system_content = system_parts.join("\n\n");
            messages.push(Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::System,
                content: system_content,
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            });
        }

        messages.extend_from_slice(history);
        messages
    }

    /// Build tool definitions in `OpenAI` function-calling JSON format.
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
        };
        let p2 = ChatService::prepare_turn(&container, request2)
            .await
            .unwrap();
        assert_eq!(p2.turn_number, p2.history.len() as u32);
        assert!(p2.turn_number > p1.turn_number);
    }
}
