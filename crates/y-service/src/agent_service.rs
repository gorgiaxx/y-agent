//! Unified Agent Service — single execution path for all agents.
//!
//! Every agent (interactive chat, sub-agents, system agents) runs through
//! the same [`AgentService::execute`] loop. The agent's capabilities (tools,
//! knowledge, iteration limits) are controlled by its [`AgentExecutionConfig`],
//! not by separate code paths.
//!
//! When `max_iterations=1` and `allowed_tools` is empty, the loop naturally
//! degrades to a single LLM call (equivalent to the old `SingleTurnRunner`).

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use y_context::pruning::IntraTurnPruner;
use y_context::{AssembledContext, ContextCategory, ContextRequest};
use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::types::{Message, ProviderId, Role, SessionId, ToolCallRequest, ToolName};
use y_tools::{format_tool_result, parse_tool_calls, strip_tool_call_blocks, ParseResult};

use crate::container::ServiceContainer;
use crate::cost::CostService;

// Re-use progress event types from chat module.
pub use crate::chat::{ToolCallRecord, TurnEvent, TurnEventSender};

// ---------------------------------------------------------------------------
// Execution config & result types
// ---------------------------------------------------------------------------

/// Configuration for a single agent execution.
///
/// Built from an `AgentDefinition` (TOML) plus caller-supplied parameters.
/// This replaces the old `TurnInput` for the internal execution loop.
#[derive(Debug, Clone)]
pub struct AgentExecutionConfig {
    /// Human-readable agent name (for diagnostics/logging).
    pub agent_name: String,
    /// Agent's system prompt (from TOML definition or context pipeline).
    pub system_prompt: String,
    /// Maximum LLM iterations (tool-call loop limit).
    pub max_iterations: usize,
    /// Tool definitions in `OpenAI` function-calling JSON format.
    /// Empty = no tool calling.
    pub tool_definitions: Vec<serde_json::Value>,
    /// Tool calling mode (Native or `PromptBased`).
    pub tool_calling_mode: ToolCallingMode,
    /// Conversation messages (system prompt prepended by caller if needed).
    pub messages: Vec<Message>,
    /// Provider routing preference.
    pub provider_id: Option<String>,
    /// Preferred model identifiers (tried in order via `RouteRequest`).
    pub preferred_models: Vec<String>,
    /// Provider routing tags.
    pub provider_tags: Vec<String>,
    /// Temperature override (None = use provider default).
    pub temperature: Option<f64>,
    /// Max tokens to generate.
    pub max_tokens: Option<u32>,
    /// Session ID for diagnostics tracing.
    pub session_id: Option<SessionId>,
    /// Session UUID for diagnostics tracing.
    pub session_uuid: Uuid,
    /// Knowledge collection names (empty = skip KB retrieval).
    pub knowledge_collections: Vec<String>,
    /// Whether to use the context pipeline for system prompt assembly.
    /// `true` for the root agent (chat), `false` for sub-agents.
    pub use_context_pipeline: bool,
    /// User query text (for context pipeline + knowledge retrieval).
    pub user_query: String,
    /// Pre-created trace ID from the diagnostics delegator.
    ///
    /// When `Some`, `execute()` reuses this trace for per-iteration
    /// observations instead of creating its own trace. The caller is
    /// responsible for calling `on_trace_start` / `on_trace_end`.
    pub external_trace_id: Option<Uuid>,
}

/// Result of agent execution.
#[derive(Debug, Clone)]
pub struct AgentExecutionResult {
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
    /// Input tokens from the **last** LLM iteration -- represents the actual
    /// prompt size sent to the model and thus the current context occupancy.
    pub last_input_tokens: u64,
    /// Context window size of the serving provider.
    pub context_window: usize,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Tool calls executed during this agent run.
    pub tool_calls_executed: Vec<ToolCallRecord>,
    /// Number of LLM iterations (>1 when tool loop occurs).
    pub iterations: usize,
    /// Messages generated during this agent run (assistant + tool messages).
    pub new_messages: Vec<Message>,
    /// Reasoning/thinking content from the final LLM response (if the model
    /// supports chain-of-thought). `None` when the model did not produce
    /// reasoning output.
    pub reasoning_content: Option<String>,
    /// Wall-clock duration of reasoning/thinking in milliseconds.
    /// Measured from the first `StreamReasoningDelta` to the first
    /// `StreamDelta` (content) or end-of-stream, whichever comes first.
    /// `None` when no reasoning was produced or when using non-streaming.
    pub reasoning_duration_ms: Option<u64>,
}

/// Error returned by [`AgentService::execute`].
#[derive(Debug)]
pub enum AgentExecutionError {
    /// LLM request failed.
    LlmError {
        /// Human-readable error message.
        message: String,
        /// Messages accumulated before the failure (assistant + tool messages
        /// from earlier successful iterations). Empty when the error occurs on
        /// the first LLM call.
        partial_messages: Vec<Message>,
    },
    /// Context assembly failed.
    ContextError(String),
    /// Tool-call iteration limit exceeded.
    ToolLoopLimitExceeded {
        /// Maximum allowed iterations.
        max_iterations: usize,
    },
    /// The execution was explicitly cancelled by the caller.
    Cancelled,
}

impl std::fmt::Display for AgentExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentExecutionError::LlmError { message, .. } => write!(f, "LLM error: {message}"),
            AgentExecutionError::ContextError(msg) => write!(f, "Context error: {msg}"),
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations } => {
                write!(f, "Tool call loop limit ({max_iterations}) exceeded")
            }
            AgentExecutionError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl std::error::Error for AgentExecutionError {}

// ---------------------------------------------------------------------------
// Internal mutable state for the tool-call loop
// ---------------------------------------------------------------------------

/// Carries mutable state across iterations of the agent execution loop.
///
/// Extracted to reduce parameter count on helper methods.
struct ToolExecContext {
    iteration: usize,
    last_gen_id: Option<Uuid>,
    tool_calls_executed: Vec<ToolCallRecord>,
    new_messages: Vec<Message>,
    cumulative_input_tokens: u64,
    cumulative_output_tokens: u64,
    cumulative_cost: f64,
    last_input_tokens: u64,
    trace_id: Option<Uuid>,
    session_id: SessionId,
    working_history: Vec<Message>,
    accumulated_content: String,
}

/// Per-iteration LLM response data bundle.
///
/// Avoids passing 7+ scalar arguments to helpers.
struct LlmIterationData {
    resp_input_tokens: u64,
    resp_output_tokens: u64,
    cost: f64,
    llm_elapsed_ms: u64,
    prompt_preview: String,
    response_text_raw: String,
}

// ---------------------------------------------------------------------------
// AgentService
// ---------------------------------------------------------------------------

/// Unified agent execution service.
///
/// All agents -- interactive chat (root), sub-agents, and system agents --
/// run through [`AgentService::execute`]. The difference between agents is
/// configuration, not code path.
pub struct AgentService;

impl AgentService {
    /// Execute an agent with full capabilities.
    ///
    /// The execution loop:
    /// 1. (Optional) Assemble context pipeline for system prompt
    /// 2. Build messages with system prompt
    /// 3. LLM call via `ProviderPool`
    /// 4. If tool calls: execute tools, append results, loop (up to `max_iterations`)
    /// 5. Return final text + diagnostics
    pub async fn execute(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        progress: Option<TurnEventSender>,
        cancel: Option<CancellationToken>,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        // 1. Context assembly + diagnostics trace (extracted to keep execute() under 200 lines).
        let (assembled, trace_id, owns_trace) =
            Self::init_context_and_trace(container, config).await;

        // 3. Build initial working history.
        let working_history = if config.use_context_pipeline {
            Self::build_chat_messages(&assembled, &config.messages)
        } else {
            config.messages.clone()
        };

        let session_id = config
            .session_id
            .clone()
            .unwrap_or_else(|| SessionId("agent".into()));

        // Mutable state for the tool-call loop.
        let mut ctx = ToolExecContext {
            iteration: 0,
            last_gen_id: None,
            tool_calls_executed: Vec::new(),
            new_messages: Vec::new(),
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cost: 0.0,
            last_input_tokens: 0,
            trace_id,
            session_id,
            working_history,
            accumulated_content: String::new(),
        };
        #[allow(unused_assignments)]
        let mut final_model = String::new();
        #[allow(unused_assignments)]
        let mut final_provider_id: Option<String> = None;

        let max_iterations = config.max_iterations;

        // Intra-turn pruner: removes failed tool call branches from
        // working_history between iterations to reduce LLM noise.
        let intra_turn_pruner = IntraTurnPruner::from_config_with_patterns(
            &container.pruning_engine.config().intra_turn,
            container
                .pruning_engine
                .config()
                .retry
                .heuristic_patterns
                .clone(),
        );

        loop {
            if let Some(ref tok) = cancel {
                if tok.is_cancelled() {
                    return Err(AgentExecutionError::Cancelled);
                }
            }

            // Intra-turn pruning: remove failed tool call branches from
            // working_history before building the next LLM request.
            if ctx.iteration > 0 {
                let prune_report = intra_turn_pruner
                    .prune_working_history(&mut ctx.working_history, ctx.iteration);
                if !prune_report.skipped && prune_report.messages_removed > 0 {
                    tracing::debug!(
                        agent = %config.agent_name,
                        iteration = ctx.iteration,
                        messages_removed = prune_report.messages_removed,
                        tokens_saved = prune_report.tokens_saved,
                        "intra-turn pruning applied to working history"
                    );
                }
            }

            ctx.iteration += 1;
            if ctx.iteration > max_iterations {
                Self::emit_loop_limit(
                    progress.as_ref(),
                    &ctx,
                    max_iterations,
                    container,
                    owns_trace,
                )
                .await;
                return Err(AgentExecutionError::ToolLoopLimitExceeded { max_iterations });
            }

            let request = Self::build_chat_request(config, &ctx);
            let route = Self::build_route_request(config);
            let fallback = serde_json::to_string(&request.messages).unwrap_or_default();

            let llm_start = std::time::Instant::now();
            let pool = container.provider_pool().await;

            let llm_result =
                Self::call_llm(&*pool, &request, &route, progress.as_ref(), cancel.as_ref()).await;

            match llm_result {
                Ok((response, iter_reasoning_duration_ms)) => {
                    // When streaming was used, the pool recorded the request
                    // count at stream start with zero tokens. Now that the
                    // stream is fully consumed, record the actual token usage.
                    if progress.is_some() {
                        if let Some(ref pid) = response.provider_id {
                            pool.record_stream_completion(
                                pid,
                                response.usage.input_tokens,
                                response.usage.output_tokens,
                            );
                        }
                    }

                    let iter_data = Self::build_iteration_data(&response, &fallback, llm_start);

                    ctx.cumulative_input_tokens += iter_data.resp_input_tokens;
                    ctx.cumulative_output_tokens += iter_data.resp_output_tokens;
                    ctx.cumulative_cost += iter_data.cost;
                    ctx.last_input_tokens = iter_data.resp_input_tokens;
                    final_model.clone_from(&response.model);
                    final_provider_id = response
                        .provider_id
                        .as_ref()
                        .map(std::string::ToString::to_string);

                    // Resolve context_window from the provider pool so
                    // real-time progress events carry it (status bar).
                    let iter_ctx_window = {
                        let metadata_list = pool.list_metadata();
                        if let Some(ref pid) = final_provider_id {
                            metadata_list
                                .iter()
                                .find(|m| m.id.to_string() == *pid)
                                .map_or(0, |m| m.context_window)
                        } else {
                            metadata_list.first().map_or(0, |m| m.context_window)
                        }
                    };

                    Self::record_generation_diagnostics(
                        container,
                        config,
                        &response,
                        &fallback,
                        &iter_data,
                        &mut ctx.last_gen_id,
                        ctx.trace_id,
                    )
                    .await;

                    if !response.tool_calls.is_empty() {
                        Self::handle_native_tool_calls(
                            container,
                            config,
                            &response,
                            progress.as_ref(),
                            &iter_data,
                            &mut ctx,
                            iter_ctx_window,
                        )
                        .await;
                        continue;
                    }

                    // Fallback: even when tool_calling_mode is Native, some
                    // models/providers may embed tool calls in text output
                    // instead of using the native API (e.g. model doesn't
                    // support function calling, or the provider strips tool
                    // definitions). Always attempt prompt-based parsing as
                    // a safety net.
                    if let Some(ref text) = response.content {
                        tracing::debug!(
                            agent = %config.agent_name,
                            content_len = text.len(),
                            has_tool_call_tag = text.contains("<tool_call>"),
                            "fallback: attempting prompt-based tool call parsing"
                        );
                        let parse_result = parse_tool_calls(text);
                        tracing::debug!(
                            agent = %config.agent_name,
                            parsed_tool_calls = parse_result.tool_calls.len(),
                            warnings = ?parse_result.warnings,
                            "fallback: parse_tool_calls result"
                        );
                        if !parse_result.tool_calls.is_empty() {
                            Self::handle_prompt_based_tool_calls(
                                container,
                                config,
                                &response,
                                &parse_result,
                                text,
                                progress.as_ref(),
                                &iter_data,
                                &mut ctx,
                                iter_ctx_window,
                            )
                            .await;
                            continue;
                        }
                    }

                    // No tool calls -- final text response.
                    return Self::build_final_result(
                        container,
                        config,
                        &response,
                        progress.as_ref(),
                        &iter_data,
                        ctx,
                        final_model,
                        final_provider_id,
                        owns_trace,
                        iter_ctx_window,
                        iter_reasoning_duration_ms,
                    )
                    .await;
                }
                Err(e) => {
                    let elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);
                    let model_name = config.preferred_models.first().cloned().unwrap_or_default();
                    return Self::handle_llm_error(
                        e,
                        elapsed_ms,
                        &model_name,
                        &fallback,
                        0, // context_window unknown -- LLM call failed
                        progress.as_ref(),
                        container,
                        owns_trace,
                        &mut ctx,
                        &config.agent_name,
                    )
                    .await;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Execute-loop helpers
    // -----------------------------------------------------------------------

    /// Context assembly + diagnostics trace initialisation.
    ///
    /// Extracted from `execute()` to keep it under the clippy line limit.
    /// Returns `(assembled_context, trace_id, owns_trace)`.
    async fn init_context_and_trace(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
    ) -> (AssembledContext, Option<Uuid>, bool) {
        let assembled = if config.use_context_pipeline {
            let context_request = ContextRequest {
                user_query: config.user_query.clone(),
                session_id: config.session_id.clone(),
                knowledge_collections: config.knowledge_collections.clone(),
                ..Default::default()
            };
            container
                .context_pipeline
                .assemble_with_request(Some(context_request))
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "context pipeline assembly failed; using empty context");
                    AssembledContext::default()
                })
        } else {
            AssembledContext::default()
        };

        // When an external_trace_id is supplied (subagent delegation),
        // reuse it so per-iteration observations land on the same trace
        // that the DiagnosticsAgentDelegator created.
        let trace_id = if let Some(ext_id) = config.external_trace_id {
            Some(ext_id)
        } else {
            container
                .diagnostics
                .on_trace_start(config.session_uuid, &config.agent_name, &config.user_query)
                .await
                .ok()
        };
        let owns_trace = config.external_trace_id.is_none();

        (assembled, trace_id, owns_trace)
    }

    fn build_chat_request(config: &AgentExecutionConfig, ctx: &ToolExecContext) -> ChatRequest {
        ChatRequest {
            messages: ctx.working_history.clone(),
            model: None,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: None,
            tools: config.tool_definitions.clone(),
            tool_calling_mode: config.tool_calling_mode,
            stop: vec![],
            extra: serde_json::Value::Null,
        }
    }

    fn build_route_request(config: &AgentExecutionConfig) -> RouteRequest {
        RouteRequest {
            preferred_provider_id: config.provider_id.as_ref().map(ProviderId::from_string),
            preferred_model: config.preferred_models.first().cloned(),
            required_tags: config.provider_tags.clone(),
            ..RouteRequest::default()
        }
    }

    fn build_iteration_data(
        response: &y_core::provider::ChatResponse,
        fallback: &str,
        llm_start: std::time::Instant,
    ) -> LlmIterationData {
        let resp_input_tokens = u64::from(response.usage.input_tokens);
        let resp_output_tokens = u64::from(response.usage.output_tokens);
        let cost = CostService::compute_cost(resp_input_tokens, resp_output_tokens);
        let llm_elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);

        let prompt_preview = response.raw_request.as_ref().map_or_else(
            || fallback.to_string(),
            |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
        );

        let response_text_raw = response.raw_response.as_ref().map_or_else(
            || {
                serde_json::json!({
                    "content": response.content.clone().unwrap_or_default(),
                    "model": response.model,
                    "usage": {
                        "input_tokens": resp_input_tokens,
                        "output_tokens": resp_output_tokens,
                    }
                })
                .to_string()
            },
            std::string::ToString::to_string,
        );

        LlmIterationData {
            resp_input_tokens,
            resp_output_tokens,
            cost,
            llm_elapsed_ms,
            prompt_preview,
            response_text_raw,
        }
    }

    /// Dispatch to streaming or non-streaming LLM call.
    ///
    /// Returns `(ChatResponse, Option<reasoning_duration_ms>)`. The duration
    /// is only available when streaming is active and the model produced
    /// reasoning content.
    async fn call_llm(
        pool: &dyn ProviderPool,
        request: &ChatRequest,
        route: &RouteRequest,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError>
    {
        if progress.is_some() {
            Self::call_llm_streaming(pool, request, route, progress, cancel).await
        } else {
            let llm_future = pool.chat_completion(request, route);
            let response = if let Some(tok) = cancel {
                tokio::select! {
                    res = llm_future => res?,
                    () = tok.cancelled() => {
                        return Err(y_core::provider::ProviderError::Cancelled);
                    }
                }
            } else {
                llm_future.await?
            };
            // Non-streaming: no reasoning duration tracking.
            Ok((response, None))
        }
    }

    async fn emit_loop_limit(
        progress: Option<&TurnEventSender>,
        ctx: &ToolExecContext,
        max_iterations: usize,
        container: &ServiceContainer,
        owns_trace: bool,
    ) {
        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::LoopLimitHit {
                iterations: ctx.iteration - 1,
                max_iterations,
            });
        }
        if owns_trace {
            if let Some(tid) = ctx.trace_id {
                let _ = container
                    .diagnostics
                    .on_trace_end(tid, false, Some("tool loop limit exceeded"))
                    .await;
            }
        }
    }

    /// Handle an LLM call error: emit progress event, close trace, return error.
    ///
    /// Extracted from `execute()` to keep that function within the clippy line limit.
    async fn handle_llm_error(
        error: y_core::provider::ProviderError,
        elapsed_ms: u64,
        model: &str,
        prompt_preview: &str,
        context_window: usize,
        progress: Option<&TurnEventSender>,
        container: &ServiceContainer,
        owns_trace: bool,
        ctx: &mut ToolExecContext,
        agent_name: &str,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        if matches!(error, y_core::provider::ProviderError::Cancelled) {
            return Err(AgentExecutionError::Cancelled);
        }

        // Emit LlmError progress event so the diagnostics panel
        // records the failed call before the progress channel closes.
        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::LlmError {
                iteration: ctx.iteration,
                error: format!("{error}"),
                duration_ms: elapsed_ms,
                model: model.to_string(),
                prompt_preview: prompt_preview.to_string(),
                context_window,
                agent_name: agent_name.to_string(),
            });
        }

        if owns_trace {
            if let Some(tid) = ctx.trace_id {
                let _ = container
                    .diagnostics
                    .on_trace_end(tid, false, Some(&error.to_string()))
                    .await;
            }
        }
        let partial = std::mem::take(&mut ctx.new_messages);
        Err(AgentExecutionError::LlmError {
            message: format!("{error}"),
            partial_messages: partial,
        })
    }

    /// Record a generation observation in the diagnostics subsystem.
    async fn record_generation_diagnostics(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        response: &y_core::provider::ChatResponse,
        prompt_preview_fallback: &str,
        data: &LlmIterationData,
        last_gen_id: &mut Option<Uuid>,
        trace_id: Option<Uuid>,
    ) {
        let Some(tid) = trace_id else { return };

        let diag_input = response.raw_request.clone().unwrap_or_else(|| {
            serde_json::from_str(prompt_preview_fallback).unwrap_or(serde_json::Value::Null)
        });
        let diag_output = response.raw_response.clone().unwrap_or_else(|| {
            serde_json::json!({
                "content": response.content.clone().unwrap_or_default(),
                "model": response.model,
                "usage": {
                    "input_tokens": data.resp_input_tokens,
                    "output_tokens": data.resp_output_tokens,
                }
            })
        });

        let gen_id = container
            .diagnostics
            .on_generation(y_diagnostics::GenerationParams {
                trace_id: tid,
                parent_id: None,
                session_id: Some(config.session_uuid),
                model: response.model.clone(),
                input_tokens: data.resp_input_tokens,
                output_tokens: data.resp_output_tokens,
                cost_usd: data.cost,
                input: diag_input,
                output: diag_output,
                duration_ms: data.llm_elapsed_ms,
            })
            .await
            .ok();
        *last_gen_id = gen_id;

        tracing::debug!(
            trace_id = %tid,
            agent = %config.agent_name,
            model = %response.model,
            input_tokens = data.resp_input_tokens,
            output_tokens = data.resp_output_tokens,
            llm_ms = data.llm_elapsed_ms,
            "Diagnostics: agent LLM call recorded"
        );
    }

    /// Execute a single tool call, record it, and emit progress events.
    ///
    /// Returns `(success, result_content)`.
    async fn execute_and_record_tool(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        tc: &ToolCallRequest,
        progress: Option<&TurnEventSender>,
        ctx: &mut ToolExecContext,
    ) -> (bool, String) {
        let tool_start = std::time::Instant::now();
        let tool_result = Self::execute_tool_call(container, tc, &ctx.session_id).await;
        let tool_elapsed_ms = u64::try_from(tool_start.elapsed().as_millis()).unwrap_or(0);

        let (tool_success, result_content) = match &tool_result {
            Ok(output) => {
                let content =
                    serde_json::to_string(&output.content).unwrap_or_else(|_| "{}".to_string());
                (output.success, content)
            }
            Err(e) => {
                let content = serde_json::json!({ "error": e.to_string() }).to_string();
                (false, content)
            }
        };

        ctx.tool_calls_executed.push(ToolCallRecord {
            name: tc.name.clone(),
            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            success: tool_success,
            duration_ms: tool_elapsed_ms,
            result_content: result_content.clone(),
        });

        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::ToolResult {
                name: tc.name.clone(),
                success: tool_success,
                duration_ms: tool_elapsed_ms,
                input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                result_preview: result_content.clone(),
                agent_name: config.agent_name.clone(),
            });
        }

        if let Some(tid) = ctx.trace_id {
            let tool_output_json: serde_json::Value = serde_json::from_str(&result_content)
                .unwrap_or(serde_json::Value::String(result_content.clone()));
            let _ = container
                .diagnostics
                .on_tool_call(
                    tid,
                    ctx.last_gen_id,
                    Some(config.session_uuid),
                    &tc.name,
                    tc.arguments.clone(),
                    tool_output_json,
                    tool_elapsed_ms,
                    tool_success,
                )
                .await;
        }

        (tool_success, result_content)
    }

    /// Emit `LlmResponse` progress event with the given tool call names.
    fn emit_llm_response(
        progress: Option<&TurnEventSender>,
        response: &y_core::provider::ChatResponse,
        data: &LlmIterationData,
        iteration: usize,
        tool_call_names: Vec<String>,
        context_window: usize,
        agent_name: &str,
    ) {
        if let Some(tx) = progress {
            let _ = tx.send(TurnEvent::LlmResponse {
                iteration,
                model: response.model.clone(),
                input_tokens: data.resp_input_tokens,
                output_tokens: data.resp_output_tokens,
                duration_ms: data.llm_elapsed_ms,
                cost_usd: data.cost,
                tool_calls_requested: tool_call_names,
                prompt_preview: data.prompt_preview.clone(),
                response_text: data.response_text_raw.clone(),
                context_window,
                agent_name: agent_name.to_string(),
            });
        }
    }

    /// Build an assistant `Message` with reasoning metadata.
    fn build_assistant_msg(
        response: &y_core::provider::ChatResponse,
        content: String,
        tool_calls: Vec<ToolCallRequest>,
    ) -> Message {
        let mut meta = serde_json::json!({ "model": response.model });
        if let Some(ref rc) = response.reasoning_content {
            meta["reasoning_content"] = serde_json::Value::String(rc.clone());
        }
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content,
            tool_call_id: None,
            tool_calls,
            timestamp: y_core::types::now(),
            metadata: meta,
        }
    }

    /// Handle native (function-calling) tool calls from an LLM response.
    async fn handle_native_tool_calls(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        response: &y_core::provider::ChatResponse,
        progress: Option<&TurnEventSender>,
        data: &LlmIterationData,
        ctx: &mut ToolExecContext,
        context_window: usize,
    ) {
        let tc_names: Vec<String> = response
            .tool_calls
            .iter()
            .map(|tc| tc.name.clone())
            .collect();

        Self::emit_llm_response(
            progress,
            response,
            data,
            ctx.iteration,
            tc_names,
            context_window,
            &config.agent_name,
        );

        // Track new messages added in this iteration for mid-loop persistence.
        let msgs_before = ctx.new_messages.len();

        let assistant_msg = Self::build_assistant_msg(
            response,
            response.content.clone().unwrap_or_default(),
            response.tool_calls.clone(),
        );

        ctx.working_history.push(assistant_msg.clone());
        ctx.new_messages.push(assistant_msg);

        for tc in &response.tool_calls {
            let (_success, result_content) =
                Self::execute_and_record_tool(container, config, tc, progress, ctx).await;

            let tool_msg = Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Tool,
                content: result_content,
                tool_call_id: Some(tc.id.clone()),
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            };
            ctx.working_history.push(tool_msg.clone());
            ctx.new_messages.push(tool_msg);
        }

        // Mid-loop pruning: truncate large tool results from previous
        // iterations so context is managed at tool-call granularity.
        if config.use_context_pipeline {
            Self::prune_working_history_mid_loop(container, ctx, msgs_before);
        }
    }

    /// Handle prompt-based tool calls parsed from LLM response text.
    async fn handle_prompt_based_tool_calls(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        response: &y_core::provider::ChatResponse,
        parse_result: &ParseResult,
        text: &str,
        progress: Option<&TurnEventSender>,
        data: &LlmIterationData,
        ctx: &mut ToolExecContext,
        context_window: usize,
    ) {
        let tc_names: Vec<String> = parse_result
            .tool_calls
            .iter()
            .map(|ptc| ptc.name.clone())
            .collect();

        Self::emit_llm_response(
            progress,
            response,
            data,
            ctx.iteration,
            tc_names,
            context_window,
            &config.agent_name,
        );

        // Track new messages added in this iteration for mid-loop persistence.
        let msgs_before = ctx.new_messages.len();

        let assistant_msg = Self::build_assistant_msg(response, text.to_string(), vec![]);

        ctx.working_history.push(assistant_msg.clone());
        ctx.new_messages.push(assistant_msg);

        let mut result_blocks = Vec::new();
        for ptc in &parse_result.tool_calls {
            let tc = ToolCallRequest {
                id: format!("prompt_{}", uuid::Uuid::new_v4()),
                name: ptc.name.clone(),
                arguments: ptc.arguments.clone(),
            };

            let (tool_success, result_content) =
                Self::execute_and_record_tool(container, config, &tc, progress, ctx).await;

            let result_value: serde_json::Value = serde_json::from_str(&result_content)
                .unwrap_or(serde_json::Value::String(result_content));
            result_blocks.push(format_tool_result(&tc.name, tool_success, &result_value));
        }

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
        ctx.working_history.push(user_msg.clone());
        ctx.new_messages.push(user_msg);

        // Mid-loop pruning: truncate large tool results from previous
        // iterations so context is managed at tool-call granularity.
        if config.use_context_pipeline {
            Self::prune_working_history_mid_loop(container, ctx, msgs_before);
        }
    }

    /// Build the final result when no tool calls are requested.
    async fn build_final_result(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        response: &y_core::provider::ChatResponse,
        progress: Option<&TurnEventSender>,
        data: &LlmIterationData,
        ctx: ToolExecContext,
        final_model: String,
        final_provider_id: Option<String>,
        owns_trace: bool,
        ctx_window: usize,
        reasoning_duration_ms: Option<u64>,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        let raw_content = response
            .content
            .clone()
            .unwrap_or_else(|| "(no content)".to_string());

        Self::emit_llm_response(
            progress,
            response,
            data,
            ctx.iteration,
            vec![],
            ctx_window,
            &config.agent_name,
        );

        let content = if config.tool_calling_mode == ToolCallingMode::PromptBased {
            let stripped = strip_tool_call_blocks(&raw_content);
            if stripped.is_empty() {
                raw_content
            } else {
                stripped
            }
        } else {
            raw_content
        };

        if owns_trace {
            if let Some(tid) = ctx.trace_id {
                let _ = container
                    .diagnostics
                    .on_trace_end(tid, true, Some(&content))
                    .await;
            }
        }

        let final_content = if ctx.accumulated_content.is_empty() {
            content.clone()
        } else {
            format!("{}{content}", ctx.accumulated_content)
        };

        Ok(AgentExecutionResult {
            content: final_content,
            model: final_model,
            provider_id: final_provider_id,
            input_tokens: ctx.cumulative_input_tokens,
            output_tokens: ctx.cumulative_output_tokens,
            last_input_tokens: ctx.last_input_tokens,
            context_window: ctx_window,
            cost_usd: ctx.cumulative_cost,
            tool_calls_executed: ctx.tool_calls_executed,
            iterations: ctx.iteration,
            new_messages: ctx.new_messages,
            reasoning_content: response.reasoning_content.clone(),
            reasoning_duration_ms,
        })
    }

    /// Mid-loop context pruning: truncates large tool result messages from
    /// previous iterations when total `working_history` tokens exceed the
    /// configured pruning threshold.
    ///
    /// Operates entirely in-memory on `working_history` -- no
    /// `ChatMessageStore` dependency. This is correct because the agentic
    /// loop builds LLM requests from `working_history`, not from persistent
    /// storage.
    ///
    /// Only called for the root chat agent (`use_context_pipeline == true`).
    ///
    /// Strategy:
    /// 1. Estimate total tokens in `working_history`
    /// 2. If total exceeds threshold, find tool/user result messages from
    ///    *previous* iterations (protect current iteration's messages)
    /// 3. Sort candidates by token size descending
    /// 4. Truncate the largest messages until total is under the threshold
    fn prune_working_history_mid_loop(
        container: &ServiceContainer,
        ctx: &mut ToolExecContext,
        msgs_before: usize,
    ) {
        let config = container.pruning_engine.config();
        if !config.enabled {
            return;
        }

        // Per-message token limit: individual tool results larger than this
        // are truncated immediately. Uses the pruning token_threshold as the
        // per-message cap (default 2000 tokens = ~8K chars).
        let per_message_limit = config.token_threshold;

        // Overall context budget: when total working_history exceeds this,
        // the largest old tool results are truncated greedily.
        // Default: 10x the per-message limit = 20K tokens.
        let context_budget = per_message_limit.saturating_mul(10);

        // Estimate total tokens in working_history.
        let total_tokens: u32 = ctx
            .working_history
            .iter()
            .map(Self::estimate_msg_tokens)
            .sum();

        if total_tokens < context_budget {
            // Total is under budget; skip the overall truncation pass but still
            // check individual large messages below.
        }

        // Collect IDs of messages added in the current iteration -- protected.
        let current_iteration_ids: std::collections::HashSet<String> = ctx.new_messages
            [msgs_before..]
            .iter()
            .map(|m| m.message_id.clone())
            .collect();

        // Build candidate list: any non-system, non-assistant message from
        // previous iterations. This captures:
        // - Role::Tool messages (native tool calling)
        // - Role::User messages with <tool_result> content (prompt-based mode)
        // - Role::User messages from prior turns loaded from transcript
        //   (these lack metadata.type=="tool_result" so a content-only check
        //    would miss them; using role-based filtering is simpler and correct)
        let mut candidates: Vec<(usize, u32)> = ctx
            .working_history
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                !current_iteration_ids.contains(&m.message_id)
                    && m.role != Role::System
                    && m.role != Role::Assistant
            })
            .map(|(idx, m)| (idx, Self::estimate_msg_tokens(m)))
            .filter(|(_, tokens)| *tokens > 200) // Only truncate messages worth truncating
            .collect();

        // Sort by token count descending so we truncate the largest first.
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        let mut truncated_count = 0u32;
        let mut tokens_saved = 0u32;
        let over_budget = total_tokens > context_budget;

        for (idx, original_tokens) in &candidates {
            // Two conditions to truncate:
            // 1. Per-message: message exceeds per_message_limit (always truncate)
            // 2. Budget: total working_history exceeds context_budget
            if *original_tokens <= per_message_limit && !over_budget {
                continue;
            }
            // If we're in budget mode only (not per-message), stop once we've
            // reclaimed enough.
            if *original_tokens <= per_message_limit
                && tokens_saved >= total_tokens.saturating_sub(context_budget)
            {
                break;
            }

            let msg = &ctx.working_history[*idx];
            let content = &msg.content;

            // Keep first 200 and last 100 chars, replace the rest with a marker.
            let keep_head = 200.min(content.len());
            let keep_tail = 100.min(content.len().saturating_sub(keep_head));

            if content.len() <= keep_head + keep_tail + 50 {
                continue;
            }

            let head = &content[..content.floor_char_boundary(keep_head)];
            let tail_start = content.ceil_char_boundary(content.len() - keep_tail);
            let tail = &content[tail_start..];
            let truncated = format!(
                "{head}\n\n[... content truncated ({original_tokens} tokens -> ~100 tokens) ...]\n\n{tail}"
            );

            let new_tokens = Self::estimate_msg_tokens_from_str(&truncated);
            let saved = original_tokens.saturating_sub(new_tokens);

            ctx.working_history[*idx].content = truncated;
            tokens_saved += saved;
            truncated_count += 1;
        }

        if truncated_count > 0 {
            tracing::info!(
                session_id = %ctx.session_id,
                total_tokens_before = total_tokens,
                per_message_limit,
                context_budget,
                messages_truncated = truncated_count,
                tokens_saved,
                "mid-loop pruning: truncated large tool results in working_history"
            );
        }
    }

    /// Estimate token count for a message (content + role overhead).
    fn estimate_msg_tokens(msg: &Message) -> u32 {
        Self::estimate_msg_tokens_from_str(&msg.content) + 4 // role/separator overhead
    }

    /// Estimate token count from text (4 chars per token heuristic).
    fn estimate_msg_tokens_from_str(text: &str) -> u32 {
        u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

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
                        | ContextCategory::Tools
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

    /// Build tool definitions filtered by an agent's allowed/denied tool lists.
    ///
    /// - `"*"` in `allowed` means all tools in the registry.
    /// - Empty `allowed` means no tools.
    /// - `denied` overrides `allowed`.
    pub async fn build_filtered_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<serde_json::Value> {
        if allowed.is_empty() {
            return vec![];
        }

        let defs = container.tool_registry.get_all_definitions().await;
        let allow_all = allowed.iter().any(|a| a == "*");

        defs.iter()
            .filter(|def| {
                let name = def.name.as_str();
                let is_allowed = allow_all || allowed.iter().any(|a| a == name);
                let is_denied = denied.iter().any(|d| d == name);
                is_allowed && !is_denied
            })
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

    /// Execute a tool call — delegates to the tool registry.
    ///
    /// Special handling for `tool_search`: delegates to [`ToolSearchOrchestrator`].
    async fn execute_tool_call(
        container: &ServiceContainer,
        tc: &ToolCallRequest,
        session_id: &SessionId,
    ) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
        // Intercept tool_search calls — unified search across tools, skills, and agents.
        if tc.name == "tool_search" {
            let sources = crate::tool_search_orchestrator::CapabilitySearchSources {
                skill_search: Some(&container.skill_search),
                agent_registry: Some(&container.agent_registry),
            };
            let result =
                crate::tool_search_orchestrator::ToolSearchOrchestrator::handle_with_sources(
                    &tc.arguments,
                    &container.tool_registry,
                    &container.tool_taxonomy,
                    &container.tool_activation_set,
                    &sources,
                )
                .await;

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

        tool.execute(input).await
    }

    // -----------------------------------------------------------------------
    // Streaming LLM call helper
    // -----------------------------------------------------------------------

    /// Call the LLM via streaming and emit `TurnEvent::StreamDelta` events.
    ///
    /// Returns `(ChatResponse, Option<reasoning_duration_ms>)` -- the assembled
    /// response plus the wall-clock reasoning duration if thinking content was
    /// produced. Supports mid-stream cancellation via `CancellationToken`.
    async fn call_llm_streaming(
        pool: &dyn ProviderPool,
        request: &ChatRequest,
        route: &RouteRequest,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError>
    {
        use y_core::provider::{ChatResponse, FinishReason, ProviderError};
        use y_core::types::TokenUsage;

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
            ..Default::default()
        };
        let mut finish_reason = FinishReason::Stop;

        // Track reasoning timing: first reasoning delta -> first content delta.
        let mut reasoning_start: Option<std::time::Instant> = None;
        let mut reasoning_duration_ms: Option<u64> = None;

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
                    () = tok.cancelled() => {
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
                            // Mark end of reasoning on first content delta.
                            if let Some(start) = reasoning_start.take() {
                                reasoning_duration_ms =
                                    Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0));
                            }
                            content.push_str(delta);
                            if let Some(tx) = progress {
                                let _ = tx.send(TurnEvent::StreamDelta {
                                    content: delta.clone(),
                                });
                            }
                        }
                    }

                    // Emit reasoning/thinking delta.
                    if let Some(ref reasoning) = chunk.delta_reasoning_content {
                        if !reasoning.is_empty() {
                            // Mark start of reasoning on first delta.
                            if reasoning_start.is_none() {
                                reasoning_start = Some(std::time::Instant::now());
                            }
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
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }

        // Build synthetic raw response for diagnostics.
        let finish_reason_str = match finish_reason {
            FinishReason::Length => "length",
            FinishReason::ToolUse => "tool_calls",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::Unknown | FinishReason::Stop => "stop",
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

        // If reasoning ended without any content delta (e.g. model produced
        // only reasoning), finalize the duration now.
        if let Some(start) = reasoning_start.take() {
            reasoning_duration_ms = Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(0));
        }

        let response = ChatResponse {
            id: String::new(),
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            reasoning_content: if reasoning_content.is_empty() {
                None
            } else {
                Some(reasoning_content)
            },
            model: model_name,
            tool_calls,
            finish_reason,
            usage,
            raw_request,
            raw_response: Some(raw_response),
            provider_id,
        };
        Ok((response, reasoning_duration_ms))
    }
}

// ---------------------------------------------------------------------------
// ServiceAgentRunner — bridges AgentPool.delegate() → AgentService.execute()
// ---------------------------------------------------------------------------

/// `AgentRunner` implementation that uses `AgentService::execute()`.
///
/// Replaces `SingleTurnRunner` — sub-agents now get the same execution loop
/// as the root chat agent (with capabilities controlled by `AgentRunConfig`).
pub struct ServiceAgentRunner {
    container: Arc<ServiceContainer>,
}

impl ServiceAgentRunner {
    /// Create a new `ServiceAgentRunner` backed by the given `ServiceContainer`.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait::async_trait]
impl AgentRunner for ServiceAgentRunner {
    async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
        let start = std::time::Instant::now();

        // Build messages: system_prompt + input as user message.
        let mut messages = Vec::with_capacity(2);
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: config.system_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        let user_content = match &config.input {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        };
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        // Build tool definitions from allowed_tools/denied_tools.
        // When allowed_tools is non-empty, agents can make tool calls across
        // multiple iterations (e.g. skill-ingestion reading companion files).
        let tool_definitions = AgentService::build_filtered_tool_definitions(
            &self.container,
            &config.allowed_tools,
            &config.denied_tools,
        )
        .await;

        // Determine max_iterations: if tools are available, use the agent
        // definition's max_iterations; otherwise single-turn.
        let max_iterations = if tool_definitions.is_empty() {
            1
        } else {
            config.max_iterations
        };

        // Determine tool calling mode: use Native when tools are available.
        let tool_calling_mode = if tool_definitions.is_empty() {
            ToolCallingMode::default()
        } else {
            ToolCallingMode::Native
        };

        // Pick up a pre-created trace_id from the diagnostics delegator
        // (set via SUBAGENT_TRACE_ID task-local).
        let external_trace_id = crate::diagnostics::SUBAGENT_TRACE_ID
            .try_with(|id| *id)
            .ok();

        let exec_config = AgentExecutionConfig {
            agent_name: config.agent_name.clone(),
            system_prompt: config.system_prompt.clone(),
            max_iterations,
            tool_definitions,
            tool_calling_mode,
            messages,
            provider_id: None,
            preferred_models: config.preferred_models.clone(),
            provider_tags: config.provider_tags.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            session_id: None,
            session_uuid: Uuid::nil(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: user_content,
            external_trace_id,
        };

        // Pick up a progress sender from the task-local (if the caller injected
        // one via SUBAGENT_PROGRESS) so that real-time LLM/tool events are
        // forwarded during subagent execution.
        let progress = crate::diagnostics::SUBAGENT_PROGRESS
            .try_with(std::clone::Clone::clone)
            .ok();

        let result = AgentService::execute(&self.container, &exec_config, progress, None)
            .await
            .map_err(|e| DelegationError::DelegationFailed {
                message: format!(
                    "AgentService execution failed for agent '{}': {e}",
                    config.agent_name
                ),
            })?;

        if result.content.is_empty() {
            return Err(DelegationError::DelegationFailed {
                message: format!("agent '{}' returned empty response", config.agent_name),
            });
        }

        let tokens_used = u32::try_from(result.input_tokens).unwrap_or(0)
            + u32::try_from(result.output_tokens).unwrap_or(0);

        Ok(AgentRunOutput {
            text: result.content,
            tokens_used,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            model_used: result.model,
            duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(0),
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

    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent, a helpful AI assistant.".to_string(),
            token_estimate: 10,
            priority: 100,
        });

        let history = vec![Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: "Hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];

        let messages = AgentService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("y-agent"));
        assert_eq!(messages[1].role, Role::User);
    }

    #[test]
    fn test_build_chat_messages_no_system_when_empty() {
        let assembled = AssembledContext::default();
        let history = vec![Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: "Hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];
        let messages = AgentService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_agent_execution_error_display() {
        assert!(AgentExecutionError::LlmError {
            message: "timeout".into(),
            partial_messages: vec![],
        }
        .to_string()
        .contains("timeout"));
        assert!(
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations: 10 }
                .to_string()
                .contains("10")
        );
        assert!(AgentExecutionError::Cancelled
            .to_string()
            .contains("Cancelled"));
    }
}
