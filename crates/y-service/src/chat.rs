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

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use y_context::{AssembledContext, ContextCategory};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::tool::ToolInput;
use y_core::types::{Message, Role, SessionId, ToolCallRequest, ToolName};
use y_tools::{format_tool_result, parse_tool_calls, strip_tool_call_blocks};

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
}

/// Token passed to `execute_turn_with_progress` to support mid-turn cancellation.
pub type TurnCancellationToken = CancellationToken;

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

            // Compute the full serialised prompt sent to the LLM.
            // No truncation -- the frontend code block is scrollable.
            let prompt_preview = serde_json::to_string(&messages).unwrap_or_default();

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

            let route = RouteRequest::default();
            let llm_start = std::time::Instant::now();

            // TODO(middleware): Route this through LlmMiddleware chain before
            // calling provider_pool directly. The middleware chain should handle
            // guardrail validation, rate limiting, caching, and auditing.
            // See hooks-plugin-design.md §Middleware Chains → LlmMiddleware.
            let pool = container.provider_pool().await;
            let llm_future = pool.chat_completion(&request, &route);

            // Race the LLM call against the cancellation token so the user
            // gets immediate feedback instead of waiting for the full HTTP
            // round-trip to complete.
            let llm_result = if let Some(ref tok) = cancel {
                tokio::select! {
                    res = llm_future => res,
                    _ = tok.cancelled() => {
                        return Err(TurnError::Cancelled);
                    }
                }
            } else {
                llm_future.await
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

                    // Diagnostics: record generation observation using raw HTTP payloads.
                    if let Some(tid) = trace_id {
                        let diag_input = response
                            .raw_request
                            .clone()
                            .unwrap_or(serde_json::Value::Null);
                        let diag_output = response
                            .raw_response
                            .clone()
                            .unwrap_or(serde_json::Value::Null);

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
                                response_text: response.content.clone().unwrap_or_default(),
                            });
                        }

                        let assistant_msg = Message {
                            message_id: y_core::types::generate_message_id(),
                            role: Role::Assistant,
                            content: response.content.clone().unwrap_or_default(),
                            tool_call_id: None,
                            tool_calls: response.tool_calls.clone(),
                            timestamp: y_core::types::now(),
                            metadata: serde_json::json!({ "model": response.model }),
                        };
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
                                        response_text: text.clone(),
                                    });
                                }
                                // Record assistant message with original text.
                                let assistant_msg = Message {
                                    message_id: y_core::types::generate_message_id(),
                                    role: Role::Assistant,
                                    content: text.clone(),
                                    tool_call_id: None,
                                    tool_calls: vec![],
                                    timestamp: y_core::types::now(),
                                    metadata: serde_json::json!({ "model": response.model }),
                                };
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
                            response_text: raw_content.clone(),
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

                    let assistant_msg = Message {
                        message_id: y_core::types::generate_message_id(),
                        role: Role::Assistant,
                        content: content.clone(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: serde_json::json!({
                            "model": response.model,
                            "usage": {
                                "input_tokens": response.usage.input_tokens,
                                "output_tokens": response.usage.output_tokens,
                            }
                        }),
                    };

                    let _ = container
                        .session_manager
                        .append_message(&input.session_id, &assistant_msg)
                        .await;

                    new_messages.push(assistant_msg);

                    // Checkpoint.
                    let msg_count_before = input.history.len() as u32;
                    let turn = input.turn_number + 1;
                    let scope_id = format!("turn-{}-{}", input.session_id.0, turn);
                    if let Err(e) = container
                        .chat_checkpoint_manager
                        .create_checkpoint(&input.session_id, turn, msg_count_before, scope_id)
                        .await
                    {
                        tracing::warn!(error = %e, "failed to create chat checkpoint");
                    }

                    let ctx_window = container
                        .provider_pool()
                        .await
                        .list_metadata()
                        .first()
                        .map_or(0, |m| m.context_window);

                    return Ok(TurnResult {
                        content,
                        model: final_model,
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

    /// Execute a single tool call from an LLM response.
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
}
