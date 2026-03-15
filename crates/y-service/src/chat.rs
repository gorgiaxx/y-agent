//! Chat service — LLM turn lifecycle orchestration.
//!
//! Centralises the full LLM-turn lifecycle:
//! 1. Context assembly (system prompt via context pipeline)
//! 2. Build `ChatRequest` with tool definitions
//! 3. LLM call via `ProviderPool`
//! 4. Diagnostics recording (trace, generation, tool observations)
//! 5. Tool execution loop (up to `MAX_TOOL_ITERATIONS`)
//! 6. Session message persistence
//! 7. Checkpoint creation

use std::fmt;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use y_context::{AssembledContext, ContextCategory};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::tool::ToolInput;
use y_core::types::{Message, ProviderId, Role, SessionId, ToolCallRequest, ToolName};
use y_tools::{format_tool_result, parse_tool_calls, strip_tool_call_blocks};
use y_core::runtime::CommandRunner;

use crate::container::ServiceContainer;
use crate::cost::CostService;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Maximum consecutive LLM calls within a single turn (tool-call loop).
const MAX_TOOL_ITERATIONS: usize = 10;

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
    /// (e.g. DeepSeek-R1, QwQ). Presentation layers show this in a collapsible
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
    ToolLoopLimitExceeded,
    /// The turn was explicitly cancelled by the caller.
    Cancelled,
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnError::LlmError(msg) => write!(f, "LLM error: {msg}"),
            TurnError::ContextError(msg) => write!(f, "Context error: {msg}"),
            TurnError::ToolLoopLimitExceeded => {
                write!(f, "Tool call loop limit ({MAX_TOOL_ITERATIONS}) exceeded")
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
}

/// A fully prepared turn, ready for `execute_turn()` or
/// `execute_turn_with_progress()`.
///
/// Owns all data needed for turn execution so callers do not need to
/// manage lifetimes of intermediate results (history, session_uuid, etc.).
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
        let (session_id, session_created) = match request.session_id {
            Some(sid) => {
                container
                    .session_manager
                    .get_session(&sid)
                    .await
                    .map_err(|e| PrepareTurnError::SessionNotFound(e.to_string()))?;
                (sid, false)
            }
            None => {
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
            }
        };

        // 2. Build and persist the user message.
        let user_msg = Message {
            message_id: generate_message_id(),
            role: Role::User,
            content: request.user_input.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: now(),
            metadata: serde_json::Value::Null,
        };
        let _ = container
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
        let session_uuid = Uuid::parse_str(session_id.as_str())
            .unwrap_or_else(|_| Uuid::new_v4());

        Ok(PreparedTurn {
            session_id,
            session_uuid,
            history,
            turn_number,
            user_input: request.user_input,
            provider_id: request.provider_id,
            session_created,
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
        let session_uuid = match Uuid::parse_str(session_id) {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };

        let store = container.diagnostics.store();
        let traces = store
            .list_traces_by_session(&session_uuid.to_string(), 1)
            .await
            .unwrap_or_default();

        let trace = match traces.first() {
            Some(t) => t,
            None => return Ok(None),
        };

        let observations = store.get_observations(trace.id).await.unwrap_or_default();
        let last_gen = observations
            .iter()
            .rev()
            .find(|o| o.obs_type == y_diagnostics::ObservationType::Generation);

        let model = last_gen
            .and_then(|o| o.model.clone())
            .unwrap_or_default();

        let pool = container.provider_pool().await;
        let metadata_list = pool.list_metadata();
        let matched = metadata_list.iter().find(|m| m.model == model);
        let context_window = matched.map(|m| m.context_window).unwrap_or(0);
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
    async fn execute_turn_inner(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        progress: Option<TurnEventSender>,
        cancel: Option<TurnCancellationToken>,
    ) -> Result<TurnResult, TurnError> {
        // 1. Assemble context pipeline (system prompt + status).
        let assembled = container
            .context_pipeline
            .assemble()
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "context pipeline assembly failed; using empty context");
                AssembledContext::default()
            });

        // 2. Build tool definitions from registry (Native mode only).
        let tool_calling_mode = ToolCallingMode::default();
        let tool_defs = match tool_calling_mode {
            ToolCallingMode::Native => Self::build_tool_definitions(container).await,
            ToolCallingMode::PromptBased => vec![],
        };

        // 3. Start diagnostics trace.
        let trace_id = container
            .diagnostics
            .on_trace_start(input.session_uuid, "chat-turn", input.user_input)
            .await
            .ok();

        // Mutable state for the tool-call loop.
        let mut iteration = 0usize;
        let mut last_gen_id: Option<Uuid> = None;
        let mut tool_calls_executed: Vec<ToolCallRecord> = Vec::new();
        let mut new_messages: Vec<Message> = Vec::new();
        let mut cumulative_input_tokens: u64 = 0;
        let mut cumulative_output_tokens: u64 = 0;
        let mut cumulative_cost: f64 = 0.0;
        #[allow(unused_assignments)]
        let mut final_model = String::new();
        #[allow(unused_assignments)]
        let mut final_provider_id: Option<String> = None;

        // Accumulate text from all LLM iterations so the final persisted
        // message contains content from every call (not just the last one).
        // This preserves tool_call XML and intermediate text for the frontend.
        let mut accumulated_content = String::new();

        let mut working_history: Vec<Message> = input.history.to_vec();

        loop {
            // Check for cancellation at the top of every iteration.
            if let Some(ref tok) = cancel {
                if tok.is_cancelled() {
                    return Err(TurnError::Cancelled);
                }
            }

            iteration += 1;
            if iteration > MAX_TOOL_ITERATIONS {
                if let Some(ref tx) = progress {
                    let _ = tx.send(TurnEvent::LoopLimitHit {
                        iterations: iteration - 1,
                        max_iterations: MAX_TOOL_ITERATIONS,
                    });
                }
                if let Some(tid) = trace_id {
                    let _ = container
                        .diagnostics
                        .on_trace_end(tid, false, Some("tool loop limit exceeded"))
                        .await;
                }
                return Err(TurnError::ToolLoopLimitExceeded);
            }

            let messages = Self::build_chat_messages(&assembled, &working_history);

            // Fallback prompt preview from internal messages; will be replaced
            // by the real HTTP request body (`raw_request`) after the LLM call.
            let prompt_preview_fallback = serde_json::to_string(&messages).unwrap_or_default();

            let request = ChatRequest {
                messages,
                model: None,
                max_tokens: None,
                temperature: Some(0.7),
                top_p: None,
                tools: tool_defs.clone(),
                tool_calling_mode,
                stop: vec![],
                extra: serde_json::Value::Null,
            };

            let route = RouteRequest {
                preferred_provider_id: input
                    .provider_id
                    .as_ref()
                    .map(|id| ProviderId::from_string(id)),
                ..RouteRequest::default()
            };
            let llm_start = std::time::Instant::now();

            // TODO(middleware): Route this through LlmMiddleware chain before
            // calling provider_pool directly. The middleware chain should handle
            // guardrail validation, rate limiting, caching, and auditing.
            // See hooks-plugin-design.md §Middleware Chains → LlmMiddleware.
            let pool = container.provider_pool().await;

            // When a progress channel exists, use streaming so presentation
            // layers can display text as it arrives. Otherwise use the
            // simpler non-streaming path.
            let llm_result = if progress.is_some() {
                Self::call_llm_streaming(
                    &*pool,
                    &request,
                    &route,
                    progress.as_ref(),
                    cancel.as_ref(),
                )
                .await
            } else {
                let llm_future = pool.chat_completion(&request, &route);
                if let Some(ref tok) = cancel {
                    tokio::select! {
                        res = llm_future => res,
                        _ = tok.cancelled() => {
                            return Err(TurnError::Cancelled);
                        }
                    }
                } else {
                    llm_future.await
                }
            };

            match llm_result {
                Ok(response) => {
                    let llm_elapsed_ms = llm_start.elapsed().as_millis() as u64;
                    let resp_input_tokens = u64::from(response.usage.input_tokens);
                    let resp_output_tokens = u64::from(response.usage.output_tokens);
                    let cost = CostService::compute_cost(resp_input_tokens, resp_output_tokens);

                    cumulative_input_tokens += resp_input_tokens;
                    cumulative_output_tokens += resp_output_tokens;
                    cumulative_cost += cost;
                    final_model = response.model.clone();
                    final_provider_id =
                        response.provider_id.as_ref().map(|id| id.to_string());

                    // Use the real HTTP request body for the prompt preview so
                    // diagnostics show the actual payload sent to the provider
                    // (standard OpenAI format) rather than the internal Message
                    // struct which carries extra fields like message_id,
                    // timestamp, and metadata.
                    let prompt_preview = response
                        .raw_request
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
                        .unwrap_or_else(|| prompt_preview_fallback.clone());

                    // Use the full raw HTTP response JSON for the response text
                    // so diagnostics show the actual payload received from the
                    // provider rather than just the extracted content string.
                    let response_text_raw = response
                        .raw_response
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| {
                            serde_json::json!({
                                "content": response.content.clone().unwrap_or_default(),
                                "model": response.model,
                                "usage": {
                                    "input_tokens": resp_input_tokens,
                                    "output_tokens": resp_output_tokens,
                                }
                            }).to_string()
                        });

                    // Diagnostics: record generation observation using raw HTTP payloads.
                    // In the streaming path, raw_request/raw_response are None.
                    // Fall back to the internal messages JSON (prompt_preview_fallback)
                    // and a synthetic output so the diagnostics panel shows useful
                    // data after a restart instead of null.
                    if let Some(tid) = trace_id {
                        let diag_input = response
                            .raw_request
                            .clone()
                            .unwrap_or_else(|| {
                                serde_json::from_str(&prompt_preview_fallback)
                                    .unwrap_or(serde_json::Value::Null)
                            });
                        let diag_output = response
                            .raw_response
                            .clone()
                            .unwrap_or_else(|| {
                                serde_json::json!({
                                    "content": response.content.clone().unwrap_or_default(),
                                    "model": response.model,
                                    "usage": {
                                        "input_tokens": resp_input_tokens,
                                        "output_tokens": resp_output_tokens,
                                    }
                                })
                            });

                        let gen_id = container
                            .diagnostics
                            .on_generation(
                                tid,
                                None,
                                Some(input.session_uuid),
                                &response.model,
                                resp_input_tokens,
                                resp_output_tokens,
                                cost,
                                diag_input,
                                diag_output,
                                llm_elapsed_ms,
                            )
                            .await
                            .ok();
                        last_gen_id = gen_id;

                        tracing::debug!(
                            trace_id = %tid,
                            model = %response.model,
                            input_tokens = resp_input_tokens,
                            output_tokens = resp_output_tokens,
                            llm_ms = llm_elapsed_ms,
                            "Diagnostics: LLM call recorded"
                        );
                    }

                    // Gather tool call names for the progress event.
                    let native_tc_names: Vec<String> =
                        response.tool_calls.iter().map(|tc| tc.name.clone()).collect();

                    // Handle tool calls.
                    if !response.tool_calls.is_empty() {
                        // Emit LlmResponse progress event (with requested tool calls).
                        if let Some(ref tx) = progress {
                            let _ = tx.send(TurnEvent::LlmResponse {
                                iteration,
                                model: response.model.clone(),
                                input_tokens: resp_input_tokens,
                                output_tokens: resp_output_tokens,
                                duration_ms: llm_elapsed_ms,
                                cost_usd: cost,
                                tool_calls_requested: native_tc_names,
                                prompt_preview: prompt_preview.clone(),
                                response_text: response_text_raw.clone(),
                            });
                        }

                        let mut meta = serde_json::json!({ "model": response.model });
                        if let Some(ref rc) = response.reasoning_content {
                            meta["reasoning_content"] = serde_json::Value::String(rc.clone());
                        }
                        let assistant_msg = Message {
                            message_id: y_core::types::generate_message_id(),
                            role: Role::Assistant,
                            content: response.content.clone().unwrap_or_default(),
                            tool_call_id: None,
                            tool_calls: response.tool_calls.clone(),
                            timestamp: y_core::types::now(),
                            metadata: meta,
                        };
                        // Accumulate intermediate assistant text for the final message.
                        accumulated_content.push_str(&assistant_msg.content);
                        accumulated_content.push('\n');
                        working_history.push(assistant_msg.clone());
                        new_messages.push(assistant_msg);

                        for tc in &response.tool_calls {
                            let tool_start = std::time::Instant::now();
                            let tool_result =
                                Self::execute_tool_call(container, tc, &input.session_id).await;
                            let tool_elapsed_ms = tool_start.elapsed().as_millis() as u64;

                            let (tool_success, result_content) = match &tool_result {
                                Ok(output) => {
                                    let content = serde_json::to_string(&output.content)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    (output.success, content)
                                }
                                Err(e) => {
                                    let content = serde_json::json!({ "error": e.to_string() })
                                        .to_string();
                                    (false, content)
                                }
                            };

                            tool_calls_executed.push(ToolCallRecord {
                                name: tc.name.clone(),
                                success: tool_success,
                                duration_ms: tool_elapsed_ms,
                                result_content: result_content.clone(),
                            });

                            // Emit ToolResult progress event.
                            if let Some(ref tx) = progress {
                                let _ = tx.send(TurnEvent::ToolResult {
                                    name: tc.name.clone(),
                                    success: tool_success,
                                    duration_ms: tool_elapsed_ms,
                                    result_preview: result_content.clone(),
                                });
                            }

                            // Diagnostics: record tool call observation.
                            if let Some(tid) = trace_id {
                                let tool_output_json: serde_json::Value =
                                    serde_json::from_str(&result_content)
                                        .unwrap_or(serde_json::Value::String(
                                            result_content.clone(),
                                        ));
                                let _ = container
                                    .diagnostics
                                    .on_tool_call(
                                        tid,
                                        last_gen_id,
                                        Some(input.session_uuid),
                                        &tc.name,
                                        tc.arguments.clone(),
                                        tool_output_json,
                                        tool_elapsed_ms,
                                        tool_success,
                                    )
                                    .await;
                            }

                            let tool_msg = Message {
                                message_id: y_core::types::generate_message_id(),
                                role: Role::Tool,
                                content: result_content,
                                tool_call_id: Some(tc.id.clone()),
                                tool_calls: vec![],
                                timestamp: y_core::types::now(),
                                metadata: serde_json::Value::Null,
                            };
                            working_history.push(tool_msg.clone());
                            new_messages.push(tool_msg);
                        }

                        continue;
                    }

                    // PromptBased mode: parse <tool_call> tags from text.
                    if tool_calling_mode == ToolCallingMode::PromptBased {
                        if let Some(ref text) = response.content {
                            let parse_result = parse_tool_calls(text);
                            if !parse_result.tool_calls.is_empty() {
                                // Emit LlmResponse progress event for prompt-based.
                                if let Some(ref tx) = progress {
                                    let prompt_tc_names: Vec<String> = parse_result
                                        .tool_calls
                                        .iter()
                                        .map(|ptc| ptc.name.clone())
                                        .collect();
                                    let _ = tx.send(TurnEvent::LlmResponse {
                                        iteration,
                                        model: response.model.clone(),
                                        input_tokens: resp_input_tokens,
                                        output_tokens: resp_output_tokens,
                                        duration_ms: llm_elapsed_ms,
                                        cost_usd: cost,
                                        tool_calls_requested: prompt_tc_names,
                                        prompt_preview: prompt_preview.clone(),
                                        response_text: response_text_raw.clone(),
                                    });
                                }
                                let mut meta = serde_json::json!({ "model": response.model });
                                if let Some(ref rc) = response.reasoning_content {
                                    meta["reasoning_content"] = serde_json::Value::String(rc.clone());
                                }
                                // Record assistant message with original text.
                                let assistant_msg = Message {
                                    message_id: y_core::types::generate_message_id(),
                                    role: Role::Assistant,
                                    content: text.clone(),
                                    tool_call_id: None,
                                    tool_calls: vec![],
                                    timestamp: y_core::types::now(),
                                    metadata: meta,
                                };
                                // Accumulate intermediate assistant text (with tool_call XML)
                                // for the final persisted message.
                                accumulated_content.push_str(text);
                                accumulated_content.push('\n');
                                working_history.push(assistant_msg.clone());
                                new_messages.push(assistant_msg);

                                // Execute each parsed tool call.
                                let mut result_blocks = Vec::new();
                                for ptc in &parse_result.tool_calls {
                                    let tc = ToolCallRequest {
                                        id: format!("prompt_{}", uuid::Uuid::new_v4()),
                                        name: ptc.name.clone(),
                                        arguments: ptc.arguments.clone(),
                                    };

                                    let tool_start = std::time::Instant::now();
                                    let tool_result = Self::execute_tool_call(
                                        container,
                                        &tc,
                                        &input.session_id,
                                    )
                                    .await;
                                    let tool_elapsed_ms =
                                        tool_start.elapsed().as_millis() as u64;

                                    let (tool_success, result_content) = match &tool_result {
                                        Ok(output) => {
                                            let content =
                                                serde_json::to_string(&output.content)
                                                    .unwrap_or_else(|_| "{}".to_string());
                                            (output.success, content)
                                        }
                                        Err(e) => {
                                            let content =
                                                serde_json::json!({ "error": e.to_string() })
                                                    .to_string();
                                            (false, content)
                                        }
                                    };

                                    tool_calls_executed.push(ToolCallRecord {
                                        name: tc.name.clone(),
                                        success: tool_success,
                                        duration_ms: tool_elapsed_ms,
                                        result_content: result_content.clone(),
                                    });

                                    // Emit ToolResult progress event.
                                    if let Some(ref tx) = progress {
                                        let _ = tx.send(TurnEvent::ToolResult {
                                            name: tc.name.clone(),
                                            success: tool_success,
                                            duration_ms: tool_elapsed_ms,
                                            result_preview: result_content.clone(),
                                        });
                                    }

                                    // Diagnostics.
                                    if let Some(tid) = trace_id {
                                        let tool_output_json: serde_json::Value =
                                            serde_json::from_str(&result_content)
                                                .unwrap_or(serde_json::Value::String(
                                                    result_content.clone(),
                                                ));
                                        let _ = container
                                            .diagnostics
                                            .on_tool_call(
                                                tid,
                                                last_gen_id,
                                                Some(input.session_uuid),
                                                &tc.name,
                                                tc.arguments.clone(),
                                                tool_output_json,
                                                tool_elapsed_ms,
                                                tool_success,
                                            )
                                            .await;
                                    }

                                    let result_value: serde_json::Value =
                                        serde_json::from_str(&result_content)
                                            .unwrap_or(serde_json::Value::String(
                                                result_content.clone(),
                                            ));
                                    result_blocks.push(format_tool_result(
                                        &tc.name,
                                        tool_success,
                                        &result_value,
                                    ));
                                }

                                // Append results as a user message.
                                let results_text = result_blocks.join("\n");
                                let user_msg = Message {
                                    message_id: y_core::types::generate_message_id(),
                                    role: Role::User,
                                    content: results_text,
                                    tool_call_id: None,
                                    tool_calls: vec![],
                                    timestamp: y_core::types::now(),
                                    metadata: serde_json::json!({ "type": "tool_result" }),
                                };
                                working_history.push(user_msg.clone());
                                new_messages.push(user_msg);

                                continue;
                            }
                        }
                    }

                    // No tool calls — text response.
                    // Emit LlmResponse progress event for final iteration.
                    let raw_content = response
                        .content
                        .clone()
                        .unwrap_or_else(|| "(no content)".to_string());
                    if let Some(ref tx) = progress {
                        let _ = tx.send(TurnEvent::LlmResponse {
                            iteration,
                            model: response.model.clone(),
                            input_tokens: resp_input_tokens,
                            output_tokens: resp_output_tokens,
                            duration_ms: llm_elapsed_ms,
                            cost_usd: cost,
                            tool_calls_requested: vec![],
                            prompt_preview: prompt_preview.clone(),
                            response_text: response_text_raw.clone(),
                        });
                    }

                    // Sanitize: strip any remaining <tool_call> XML from the
                    // response so the user never sees raw protocol tags.
                    let content = if tool_calling_mode == ToolCallingMode::PromptBased {
                        let stripped = strip_tool_call_blocks(&raw_content);
                        if stripped.is_empty() { raw_content } else { stripped }
                    } else {
                        raw_content
                    };

                    if let Some(tid) = trace_id {
                        let _ = container
                            .diagnostics
                            .on_trace_end(tid, true, Some(&content))
                            .await;
                    }

                    // Build the final content: if there were prior iterations,
                    // prepend their accumulated text so the persisted message
                    // contains the full multi-iteration content.
                    let final_content = if accumulated_content.is_empty() {
                        content.clone()
                    } else {
                        format!("{}{}", accumulated_content, content)
                    };

                    // Build tool_results metadata for frontend rendering after
                    // session reload (so tool call cards persist).
                    let tool_results_meta: Vec<serde_json::Value> = tool_calls_executed
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "name": tc.name,
                                "success": tc.success,
                                "duration_ms": tc.duration_ms,
                                "result_preview": &tc.result_content[..tc.result_content.len().min(2000)],
                            })
                        })
                        .collect();

                    let mut meta = serde_json::json!({
                        "model": response.model,
                        "usage": {
                            "input_tokens": response.usage.input_tokens,
                            "output_tokens": response.usage.output_tokens,
                        },
                        "tool_results": tool_results_meta,
                    });
                    if let Some(ref rc) = response.reasoning_content {
                        meta["reasoning_content"] = serde_json::Value::String(rc.clone());
                    }

                    let assistant_msg = Message {
                        message_id: y_core::types::generate_message_id(),
                        role: Role::Assistant,
                        content: final_content,
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: meta,
                    };

                    let _ = container
                        .session_manager
                        .append_message(&input.session_id, &assistant_msg)
                        .await;

                    new_messages.push(assistant_msg);

                    // Checkpoint.
                    // `input.history` includes the user message appended by
                    // `prepare_turn`, so subtract 1 to get the count *before*
                    // the turn started (i.e. before the user message).
                    let msg_count_before = input.history.len().saturating_sub(1) as u32;
                    let turn = input.turn_number + 1;
                    let scope_id = format!("turn-{}-{}", input.session_id.0, turn);
                    if let Err(e) = container
                        .chat_checkpoint_manager
                        .create_checkpoint(&input.session_id, turn, msg_count_before, scope_id)
                        .await
                    {
                        tracing::warn!(error = %e, "failed to create chat checkpoint");
                    }

                    let metadata_list = container.provider_pool().await.list_metadata();
                    let ctx_window = if let Some(ref pid) = final_provider_id {
                        metadata_list
                            .iter()
                            .find(|m| m.id.to_string() == *pid)
                            .map_or(0, |m| m.context_window)
                    } else {
                        metadata_list.first().map_or(0, |m| m.context_window)
                    };

                    return Ok(TurnResult {
                        content,
                        model: final_model,
                        provider_id: final_provider_id,
                        input_tokens: cumulative_input_tokens,
                        output_tokens: cumulative_output_tokens,
                        context_window: ctx_window,
                        cost_usd: cumulative_cost,
                        tool_calls_executed,
                        iterations: iteration,
                        new_messages,
                    });
                }
                Err(e) => {
                    // Convert streaming cancellation into TurnError::Cancelled.
                    if matches!(e, y_core::provider::ProviderError::Cancelled) {
                        return Err(TurnError::Cancelled);
                    }
                    if let Some(tid) = trace_id {
                        let _ = container
                            .diagnostics
                            .on_trace_end(tid, false, Some(&e.to_string()))
                            .await;
                    }
                    return Err(TurnError::LlmError(format!("{e}")));
                }
            }
        }
    }

    /// Build LLM messages by prepending system prompt from assembled context.
    pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
        let system_parts: Vec<&str> = assembled
            .items
            .iter()
            .filter(|item| item.category == ContextCategory::SystemPrompt)
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
    pub async fn build_tool_definitions(
        container: &ServiceContainer,
    ) -> Vec<serde_json::Value> {
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

    /// Execute a tool call from an LLM response — delegated from the main loop.
    ///
    /// Special handling for `tool_search`: delegates to [`ToolSearchOrchestrator`]
    /// which performs real taxonomy/registry lookups and activates discovered tools
    /// in the session's [`ToolActivationSet`].
    async fn execute_tool_call(
        container: &ServiceContainer,
        tc: &ToolCallRequest,
        session_id: &SessionId,
    ) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
        // Intercept tool_search calls — delegate to the orchestrator
        // for real taxonomy/registry lookups + activation.
        if tc.name == "tool_search" {
            let result = crate::tool_search_orchestrator::ToolSearchOrchestrator::handle(
                &tc.arguments,
                &container.tool_registry,
                &container.tool_taxonomy,
                &container.tool_activation_set,
            )
            .await;

            // Sync activated tool schemas into the shared dynamic_tool_schemas
            // so InjectTools can inject them in the next context assembly.
            if result.is_ok() {
                let activation_set = container.tool_activation_set.read().await;
                let schemas: Vec<String> = activation_set
                    .active_definitions()
                    .iter()
                    .map(|def| {
                        format!(
                            "### {}\n{}\nParameters: {}",
                            def.name.as_str(),
                            def.description,
                            serde_json::to_string_pretty(&def.parameters)
                                .unwrap_or_else(|_| "{}".to_string()),
                        )
                    })
                    .collect();
                let mut dynamic = container.dynamic_tool_schemas.write().await;
                *dynamic = schemas;
            }

            return result;
        }

        let tool_name = ToolName::from_string(&tc.name);

        let tool = container
            .tool_registry
            .get_tool(&tool_name)
            .await
            .ok_or_else(|| y_core::tool::ToolError::NotFound {
                name: tc.name.clone(),
            })?;

        let input = ToolInput {
            call_id: tc.id.clone(),
            name: tool_name,
            arguments: tc.arguments.clone(),
            session_id: session_id.clone(),
            command_runner: Some(Arc::clone(&container.runtime_manager) as Arc<dyn CommandRunner>),
        };

        // TODO(middleware): Route through ToolMiddleware chain instead of
        // calling tool.execute() directly. The middleware chain should handle:
        // - ToolGuardMiddleware (permission checks)
        // - FileJournalMiddleware (pre-mutation file snapshots)
        // - LoopDetectorMiddleware (redundant call detection)
        // - CapabilityGapMiddleware (tool gap resolution)
        // See hooks-plugin-design.md §Middleware Chains → ToolMiddleware.
        tool.execute(input).await
    }

    // -----------------------------------------------------------------------
    // Streaming LLM call helper
    // -----------------------------------------------------------------------

    /// Call the LLM via streaming and emit `TurnEvent::StreamDelta` events for
    /// each incremental text chunk. Returns a fully assembled `ChatResponse`
    /// equivalent to the non-streaming path.
    ///
    /// Supports mid-stream cancellation via the optional `CancellationToken`.
    async fn call_llm_streaming(
        pool: &dyn y_core::provider::ProviderPool,
        request: &y_core::provider::ChatRequest,
        route: &y_core::provider::RouteRequest,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<y_core::provider::ChatResponse, y_core::provider::ProviderError> {
        use y_core::provider::{ChatResponse, FinishReason, ProviderError};
        use y_core::types::TokenUsage;

        // The provider captures the raw HTTP request body it serialized.
        // We receive it here via ChatStreamResponse so diagnostics stores
        // the exact payload sent over the wire.
        let stream_response = pool.chat_completion_stream(request, route).await?;
        let raw_request = stream_response.raw_request;
        let provider_id = stream_response.provider_id;
        let model_name = stream_response.model;
        let mut stream = stream_response.stream;

        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut tool_calls = Vec::new();
        let mut usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let mut finish_reason = FinishReason::Stop;

        loop {
            // Check cancellation between chunks.
            if let Some(tok) = cancel {
                if tok.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
            }

            let chunk_result = if let Some(tok) = cancel {
                tokio::select! {
                    next = stream.next() => next,
                    _ = tok.cancelled() => {
                        return Err(ProviderError::Cancelled);
                    }
                }
            } else {
                stream.next().await
            };

            match chunk_result {
                Some(Ok(chunk)) => {
                    // Emit text delta to presentation layers.
                    if let Some(ref delta) = chunk.delta_content {
                        if !delta.is_empty() {
                            content.push_str(delta);
                            if let Some(tx) = progress {
                                let _ = tx.send(TurnEvent::StreamDelta {
                                    content: delta.clone(),
                                });
                            }
                        }
                    }

                    // Emit reasoning/thinking delta to presentation layers.
                    if let Some(ref reasoning) = chunk.delta_reasoning_content {
                        if !reasoning.is_empty() {
                            tracing::debug!(len = reasoning.len(), "streaming reasoning delta");
                            reasoning_content.push_str(reasoning);
                            if let Some(tx) = progress {
                                let _ = tx.send(TurnEvent::StreamReasoningDelta {
                                    content: reasoning.clone(),
                                });
                            }
                        }
                    }

                    // Collect tool calls on finish.
                    if !chunk.delta_tool_calls.is_empty() {
                        tool_calls.extend(chunk.delta_tool_calls);
                    }

                    // Capture usage from the final chunk.
                    if let Some(u) = chunk.usage {
                        usage = u;
                    }

                    if let Some(fr) = chunk.finish_reason {
                        finish_reason = fr;
                    }
                }
                Some(Err(e)) => {
                    return Err(e);
                }
                None => {
                    // Stream ended.
                    break;
                }
            }
        }

        // Streaming has no single HTTP response body. We assemble a
        // synthetic response from the accumulated chunks for diagnostics.
        let finish_reason_str = match finish_reason {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
            FinishReason::ToolUse => "tool_calls",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::Unknown => "stop",
        };
        let raw_response = serde_json::json!({
            "id": "",
            "object": "chat.completion",
            "model": model_name,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason_str,
            }],
            "usage": {
                "prompt_tokens": usage.input_tokens,
                "completion_tokens": usage.output_tokens,
            }
        });

        Ok(ChatResponse {
            id: String::new(),
            model: model_name,
            content: if content.is_empty() { None } else { Some(content) },
            reasoning_content: if reasoning_content.is_empty() { None } else { Some(reasoning_content) },
            tool_calls,
            usage,
            finish_reason,
            raw_request,
            raw_response: Some(raw_response),
            provider_id,
        })
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

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 3);
        assert!(messages[0].content.contains("Part one"));
        assert!(messages[0].content.contains("Part two"));
        assert!(!messages[0].content.contains("status info"));
    }

    #[test]
    fn test_turn_error_display() {
        assert!(TurnError::LlmError("timeout".into())
            .to_string()
            .contains("timeout"));
        assert!(TurnError::ToolLoopLimitExceeded
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
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();

        // History should contain at least the user message.
        let last = prepared.history.last().expect("history should not be empty");
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
        };
        let p1 = ChatService::prepare_turn(&container, request).await.unwrap();
        assert_eq!(p1.turn_number, p1.history.len() as u32);

        // Second message in same session.
        let request2 = PrepareTurnRequest {
            session_id: Some(p1.session_id.clone()),
            user_input: "second".into(),
            provider_id: None,
        };
        let p2 = ChatService::prepare_turn(&container, request2).await.unwrap();
        assert_eq!(p2.turn_number, p2.history.len() as u32);
        assert!(p2.turn_number > p1.turn_number);
    }
}
