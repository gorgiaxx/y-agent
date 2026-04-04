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
use y_core::permission_types::PermissionMode;
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::trust::TrustTier;
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
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
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
    /// Trust tier of the executing agent.
    ///
    /// When `Some(TrustTier::BuiltIn)`, tools listed in `agent_allowed_tools`
    /// are auto-allowed without consulting the global permission policy.
    /// `None` for the root chat agent (uses global policy as-is).
    pub trust_tier: Option<TrustTier>,
    /// Tools declared in the agent definition's `allowed_tools` list.
    ///
    /// Used together with `trust_tier` to auto-allow built-in agent tools.
    /// Empty for the root chat agent.
    pub agent_allowed_tools: Vec<String>,
    /// Whether to prune historical tool call pairs from `working_history`.
    ///
    /// When `true`, old assistant+tool message pairs (all except the most
    /// recent batch) are removed between iterations.
    pub prune_tool_history: bool,
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
    /// Tool definitions dynamically activated via `ToolSearch` during this turn.
    /// Merged with `config.tool_definitions` when building each `ChatRequest`.
    dynamic_tool_defs: Vec<serde_json::Value>,
    /// Pending user-interaction answer channels for `AskUser` tool calls.
    pending_interactions: crate::chat::PendingInteractions,
    /// Pending permission-approval channels for HITL permission requests.
    pending_permissions: crate::chat::PendingPermissions,
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

/// Parameters for building the final agent execution result.
///
/// Extracted from a tuple to improve readability at the call site.
struct FinalResultParams {
    final_model: String,
    final_provider_id: Option<String>,
    owns_trace: bool,
    context_window: usize,
    reasoning_duration_ms: Option<u64>,
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

        // Set up DIAGNOSTICS_CTX so gateways can record observations
        // automatically. If no trace_id, we still run without a context.
        let diag_ctx = trace_id.map(|tid| {
            y_diagnostics::DiagnosticsContext::new(
                tid,
                Some(config.session_uuid),
                config.agent_name.clone(),
            )
        });

        // Delegate to the inner execute logic, optionally scoped with
        // the diagnostics context task-local.
        if let Some(ctx) = diag_ctx {
            y_diagnostics::DIAGNOSTICS_CTX
                .scope(
                    ctx,
                    Self::execute_inner(
                        container, config, progress, cancel, assembled, trace_id, owns_trace,
                    ),
                )
                .await
        } else {
            Self::execute_inner(
                container, config, progress, cancel, assembled, trace_id, owns_trace,
            )
            .await
        }
    }

    /// Inner execution loop, optionally running inside a `DIAGNOSTICS_CTX` scope.
    async fn execute_inner(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        progress: Option<TurnEventSender>,
        cancel: Option<CancellationToken>,
        assembled: AssembledContext,
        trace_id: Option<Uuid>,
        owns_trace: bool,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
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
            dynamic_tool_defs: Vec::new(),
            pending_interactions: container.pending_interactions.clone(),
            pending_permissions: container.pending_permissions.clone(),
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

                // Tool history pruning (opt-in per agent): merge old assistant
                // summaries into the latest assistant, then remove old pairs.
                if config.prune_tool_history {
                    let pruned = Self::prune_old_tool_results(&mut ctx.working_history);
                    if pruned > 0 {
                        tracing::debug!(
                            agent = %config.agent_name,
                            iteration = ctx.iteration,
                            messages_removed = pruned,
                            "tool history pruning: merged old summaries and removed old pairs"
                        );
                    }
                }

                // Strip thinking/reasoning content from historical assistant
                // messages (always-on). The LLM does not benefit from
                // re-reading its own prior chain-of-thought.
                Self::strip_historical_thinking(&mut ctx.working_history);
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
            let raw_pool = container.provider_pool().await;

            // Wrap the pool with the diagnostics gateway so non-streaming
            // LLM calls are automatically recorded. Streaming calls pass
            // through (the assembled response is recorded after consumption).
            let diag_pool = crate::diagnostics::DiagnosticsProviderPool::new(
                Arc::clone(&raw_pool) as Arc<dyn ProviderPool>,
                Arc::clone(&container.diagnostics),
                container.diagnostics_broadcast.clone(),
            );

            let llm_result = Self::call_llm(
                &diag_pool,
                &request,
                &route,
                progress.as_ref(),
                cancel.as_ref(),
            )
            .await;

            match llm_result {
                Ok((response, iter_reasoning_duration_ms)) => {
                    // When streaming was used, the pool recorded the request
                    // count at stream start with zero tokens. Now that the
                    // stream is fully consumed, record the actual token usage.
                    if progress.is_some() {
                        if let Some(ref pid) = response.provider_id {
                            raw_pool.record_stream_completion(
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
                        let metadata_list = raw_pool.list_metadata();
                        if let Some(ref pid) = final_provider_id {
                            metadata_list
                                .iter()
                                .find(|m| m.id.to_string() == *pid)
                                .map_or(0, |m| m.context_window)
                        } else {
                            metadata_list.first().map_or(0, |m| m.context_window)
                        }
                    };

                    // Diagnostics recording for non-streaming (non-progress)
                    // calls is handled by DiagnosticsProviderPool. For streaming
                    // calls (progress.is_some()), the gateway cannot intercept
                    // the assembled response, so we record here.
                    if progress.is_some() {
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
                    }

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
                        FinalResultParams {
                            final_model,
                            final_provider_id,
                            owns_trace,
                            context_window: iter_ctx_window,
                            reasoning_duration_ms: iter_reasoning_duration_ms,
                        },
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
            // Update per-request tool protocol flag so the system prompt
            // includes/excludes XML tool protocol based on this request's mode.
            {
                let mut pctx = container.prompt_context.write().await;
                if config.tool_calling_mode == ToolCallingMode::PromptBased {
                    pctx.config_flags
                        .insert("tool_calling.prompt_based".into(), true);
                } else {
                    pctx.config_flags.remove("tool_calling.prompt_based");
                }
            }

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
        // Merge essential (static) + dynamically activated tool definitions.
        let tools = if ctx.dynamic_tool_defs.is_empty() {
            config.tool_definitions.clone()
        } else {
            let mut merged = config.tool_definitions.clone();
            for dyn_def in &ctx.dynamic_tool_defs {
                let dyn_name = dyn_def
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str());
                if let Some(name) = dyn_name {
                    let already_present = merged.iter().any(|t| {
                        t.get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            == Some(name)
                    });
                    if !already_present {
                        merged.push(dyn_def.clone());
                    }
                }
            }
            merged
        };

        ChatRequest {
            messages: ctx.working_history.clone(),
            model: None,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            top_p: None,
            tools,
            tool_calling_mode: config.tool_calling_mode,
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: config.thinking.clone(),
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

    fn resolve_permission_decision_for_session(
        decision: y_guardrails::PermissionDecision,
        session_mode: Option<PermissionMode>,
    ) -> y_guardrails::PermissionDecision {
        match session_mode {
            Some(PermissionMode::BypassPermissions)
                if decision.action != y_guardrails::PermissionAction::Deny =>
            {
                y_guardrails::PermissionDecision {
                    action: y_guardrails::PermissionAction::Allow,
                    reason: format!(
                        "session permission override ({})",
                        PermissionMode::BypassPermissions
                    ),
                }
            }
            _ => decision,
        }
    }

    async fn session_permission_mode(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Option<PermissionMode> {
        let modes = container.session_permission_modes.read().await;
        modes.get(session_id).copied()
    }

    async fn set_session_permission_mode(
        container: &ServiceContainer,
        session_id: &SessionId,
        mode: PermissionMode,
    ) {
        let mut modes = container.session_permission_modes.write().await;
        modes.insert(session_id.clone(), mode);
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

        // ---------------------------------------------------------------
        // Permission gatekeeper: evaluate guardrail permission BEFORE
        // executing the tool. Reads `default_permission`, per-tool overrides,
        // and `dangerous_auto_ask` from the hot-reloadable GuardrailConfig.
        // ---------------------------------------------------------------
        let guardrail_config = container.guardrail_manager.config();
        let is_dangerous = {
            let tool_name_key = ToolName::from_string(&tc.name);
            container
                .tool_registry
                .get_definition(&tool_name_key)
                .await
                .is_some_and(|def| def.is_dangerous)
        };

        let permission_model = y_guardrails::PermissionModel::new(guardrail_config);
        let session_mode = Self::session_permission_mode(container, &ctx.session_id).await;

        // Built-in agents auto-allow their declared tools without consulting
        // global permission policy. This prevents background subagents from
        // being blocked when the user sets a global "ask" mode.
        let builtin_auto_allow = config.trust_tier == Some(TrustTier::BuiltIn)
            && config.agent_allowed_tools.iter().any(|t| t == &tc.name);

        let decision = if builtin_auto_allow {
            tracing::debug!(
                tool = %tc.name,
                agent = %config.agent_name,
                "auto-allowed: built-in agent declared tool"
            );
            y_guardrails::PermissionDecision {
                action: y_guardrails::PermissionAction::Allow,
                reason: format!("built-in agent '{}' declared tool", config.agent_name),
            }
        } else {
            Self::resolve_permission_decision_for_session(
                permission_model.evaluate(&tc.name, is_dangerous),
                session_mode,
            )
        };

        match decision.action {
            y_guardrails::PermissionAction::Deny => {
                // Denied by policy -- do NOT execute the tool.
                tracing::warn!(
                    tool = %tc.name,
                    reason = %decision.reason,
                    "tool execution denied by permission policy"
                );
                let error_content = format!(
                    "[SYSTEM] Tool '{}' is blocked by security policy ({}). \
                     Do NOT ask the user for permission or retry this tool. \
                     Use an alternative approach or skip this action.",
                    tc.name, decision.reason
                );

                ctx.tool_calls_executed.push(ToolCallRecord {
                    name: tc.name.clone(),
                    arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    success: false,
                    duration_ms: 0,
                    result_content: error_content.clone(),
                });

                if let Some(tx) = progress {
                    let _ = tx.send(TurnEvent::ToolResult {
                        name: tc.name.clone(),
                        success: false,
                        duration_ms: 0,
                        input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        result_preview: error_content.clone(),
                        agent_name: config.agent_name.clone(),
                    });
                }

                return (false, error_content);
            }
            y_guardrails::PermissionAction::Ask => {
                // Pause and ask the user for approval via HITL.
                let request_id = uuid::Uuid::new_v4().to_string();

                // Extract content preview (command for ShellExec, path for
                // file tools, etc.) for the permission prompt.
                let content_preview = tc
                    .arguments
                    .get("command")
                    .or_else(|| tc.arguments.get("path"))
                    .or_else(|| tc.arguments.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let action_desc = if let Some(ref preview) = content_preview {
                    format!("{} wants to execute: {}", tc.name, preview)
                } else {
                    format!("{} wants to execute", tc.name)
                };

                tracing::info!(
                    tool = %tc.name,
                    request_id = %request_id,
                    reason = %decision.reason,
                    "permission escalation: asking user for approval"
                );

                // Register a oneshot channel for the response.
                let (resp_tx, resp_rx) =
                    tokio::sync::oneshot::channel::<crate::chat::PermissionPromptResponse>();
                {
                    let mut map = ctx.pending_permissions.lock().await;
                    map.insert(request_id.clone(), resp_tx);
                }

                // Emit the permission request event to the presentation layer.
                if let Some(tx) = progress {
                    let _ = tx.send(TurnEvent::PermissionRequest {
                        request_id: request_id.clone(),
                        tool_name: tc.name.clone(),
                        action_description: action_desc,
                        reason: decision.reason.clone(),
                        content_preview,
                    });
                }

                // Wait for user response (with timeout).
                let timeout_ms = container.guardrail_manager.config().hitl.timeout_ms;
                let response = match tokio::time::timeout(
                    std::time::Duration::from_millis(timeout_ms),
                    resp_rx,
                )
                .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(_)) => {
                        // Channel dropped (UI closed) -- deny.
                        tracing::warn!(
                            tool = %tc.name,
                            "permission channel dropped -- denying"
                        );
                        crate::chat::PermissionPromptResponse::Deny
                    }
                    Err(_) => {
                        // Timeout -- deny.
                        tracing::warn!(
                            tool = %tc.name,
                            timeout_ms,
                            "permission timeout -- denying"
                        );
                        // Clean up the pending entry.
                        let mut map = ctx.pending_permissions.lock().await;
                        map.remove(&request_id);
                        crate::chat::PermissionPromptResponse::Deny
                    }
                };

                let approved = match response {
                    crate::chat::PermissionPromptResponse::Approve => true,
                    crate::chat::PermissionPromptResponse::Deny => false,
                    crate::chat::PermissionPromptResponse::AllowAllForSession => {
                        Self::set_session_permission_mode(
                            container,
                            &ctx.session_id,
                            PermissionMode::BypassPermissions,
                        )
                        .await;
                        tracing::info!(
                            tool = %tc.name,
                            session_id = %ctx.session_id,
                            "permission approved and bypass enabled for session"
                        );
                        true
                    }
                };

                if !approved {
                    let error_content = format!(
                        "[SYSTEM] Tool '{}' was denied by the user via the permission dialog. \
                         Do NOT ask the user for permission or retry this tool. \
                         Use an alternative approach or skip this action.",
                        tc.name
                    );

                    ctx.tool_calls_executed.push(ToolCallRecord {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        success: false,
                        duration_ms: 0,
                        result_content: error_content.clone(),
                    });

                    if let Some(tx) = progress {
                        let _ = tx.send(TurnEvent::ToolResult {
                            name: tc.name.clone(),
                            success: false,
                            duration_ms: 0,
                            input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                            result_preview: error_content.clone(),
                            agent_name: config.agent_name.clone(),
                        });
                    }

                    return (false, error_content);
                }

                tracing::info!(
                    tool = %tc.name,
                    "permission approved by user"
                );
            }
            y_guardrails::PermissionAction::Notify => {
                // Execute but log for auditing.
                tracing::info!(
                    tool = %tc.name,
                    reason = %decision.reason,
                    "tool execution permitted with notification"
                );
            }
            y_guardrails::PermissionAction::Allow => {
                // No action needed -- proceed silently.
            }
        }

        // ---------------------------------------------------------------
        // Permission check passed -- execute the tool.
        // ---------------------------------------------------------------

        // Intercept AskUser calls -- route through the user interaction
        // orchestrator which emits a TurnEvent and awaits the user's answer.
        let tool_result = if tc.name == "AskUser" {
            crate::user_interaction_orchestrator::UserInteractionOrchestrator::handle(
                &tc.arguments,
                &ctx.pending_interactions,
                progress,
            )
            .await
        } else {
            Self::execute_tool_call(container, tc, &ctx.session_id).await
        };

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

        // Record via the tool gateway (reads DIAGNOSTICS_CTX automatically).
        container
            .tool_gateway
            .record_from_str(
                &tc.name,
                &tc.arguments,
                &result_content,
                tool_elapsed_ms,
                tool_success,
            )
            .await;

        // Auto-register agent definitions when FileWrite creates a .toml
        // in an agents/ directory. This lets agent-architect's output take
        // effect immediately without a restart or manual reload.
        if tool_success && tc.name == "FileWrite" {
            Self::maybe_auto_register_agent(container, &tc.arguments).await;
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

        // Even with Native tool calling, some providers/models embed XML tool
        // call blocks in the text content alongside structured tool_calls.
        // Strip them so raw protocol XML never leaks into the conversation.
        let iter_content = {
            let raw = response.content.clone().unwrap_or_default();
            let stripped = strip_tool_call_blocks(&raw);
            if stripped.is_empty() {
                raw
            } else {
                stripped
            }
        };

        // Accumulate this iteration's text so the final persisted message
        // includes all iterations' content. If the response doesn't already
        // contain <think> tags, wrap it so the frontend can properly interleave
        // and group it inside the ActionCard's collapsible section.
        let out_content = if iter_content.trim().is_empty() {
            String::new()
        } else if iter_content.trim().starts_with("<think>") {
            iter_content.clone()
        } else {
            format!("<think>\n{}\n</think>\n", iter_content.trim())
        };
        ctx.accumulated_content.push_str(&out_content);

        let assistant_msg =
            Self::build_assistant_msg(response, out_content, response.tool_calls.clone());

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

        // If ToolSearch was called this iteration, sync newly activated
        // tool definitions so they appear in the next ChatRequest.tools.
        if response.tool_calls.iter().any(|tc| tc.name == "ToolSearch") {
            Self::sync_dynamic_tool_defs(container, ctx).await;
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

        // Accumulate this iteration's text so the final persisted message
        // includes all iterations' content. If the response doesn't already
        // contain <think> tags, wrap it so the frontend can properly interleave
        // and group it inside the ActionCard's collapsible section.
        let out_content = if text.trim().is_empty() {
            String::new()
        } else if text.trim().starts_with("<think>") {
            text.to_string()
        } else {
            format!("<think>\n{}\n</think>\n", text.trim())
        };

        ctx.accumulated_content.push_str(&out_content);

        let assistant_msg = Self::build_assistant_msg(response, out_content, vec![]);

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

        // If ToolSearch was called this iteration, sync newly activated
        // tool definitions so they appear in the next ChatRequest.tools.
        if parse_result
            .tool_calls
            .iter()
            .any(|ptc| ptc.name == "ToolSearch")
        {
            Self::sync_dynamic_tool_defs(container, ctx).await;
        }

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
        params: FinalResultParams,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        let FinalResultParams {
            final_model,
            final_provider_id,
            owns_trace,
            context_window: ctx_window,
            reasoning_duration_ms,
        } = params;
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

        // Always strip XML tool call blocks regardless of tool calling mode.
        // Even Native-mode providers may embed provider-specific XML tags in
        // the text content (e.g. MiniMax, DeepSeek, GLM), and these must
        // never leak into the user-visible output.
        let content = {
            let stripped = strip_tool_call_blocks(&raw_content);
            if stripped.is_empty() {
                raw_content
            } else {
                stripped
            }
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

    /// Sync dynamically activated tool definitions from the `ToolActivationSet`
    /// into `ctx.dynamic_tool_defs` so they appear in subsequent `ChatRequest.tools`.
    ///
    /// Called after a `ToolSearch` call activates new tools. Also sets the
    /// `orchestration.enabled` prompt flag when workflow/schedule tools are active.
    async fn sync_dynamic_tool_defs(container: &ServiceContainer, ctx: &mut ToolExecContext) {
        use crate::container::ESSENTIAL_TOOL_NAMES;

        let essential: std::collections::HashSet<&str> =
            ESSENTIAL_TOOL_NAMES.iter().copied().collect();

        let set = container.tool_activation_set.read().await;
        let active = set.active_definitions();

        ctx.dynamic_tool_defs = active
            .iter()
            .filter(|def| !essential.contains(def.name.as_str()))
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
            .collect();

        // If workflow/schedule tools were activated, set the orchestration
        // flag so the system prompt includes orchestration instructions on
        // subsequent turns.
        let has_orchestration = active.iter().any(|d| {
            let n = d.name.as_str();
            n.starts_with("workflow_") || n.starts_with("schedule_")
        });
        if has_orchestration {
            let mut pctx = container.prompt_context.write().await;
            pctx.config_flags
                .insert("orchestration.enabled".into(), true);
        }

        tracing::debug!(
            dynamic_count = ctx.dynamic_tool_defs.len(),
            "synced dynamic tool definitions from activation set"
        );
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
    // Working history pruning helpers
    // -----------------------------------------------------------------------

    /// Merge-and-prune historical tool call pairs from `working_history`.
    ///
    /// For agents that build incremental summaries (e.g. `knowledge-summarizer`),
    /// each assistant response contains a summary of the tool result it just
    /// processed. This function:
    ///
    /// 1. **Collects** text content (stripped of `<think>` tags) from all
    ///    assistant messages with `tool_calls` that appear *before* the latest
    ///    assistant message.
    /// 2. **Merges** those summaries into the latest assistant message by
    ///    prepending them, so the accumulated context is preserved in a single
    ///    assistant message.
    /// 3. **Removes** the old assistant+tool message pairs.
    ///
    /// The net effect: the LLM request always contains at most **one**
    /// assistant message (with the accumulated rolling summary) and **one**
    /// tool result (the most recent chunk). System and User messages are
    /// never removed.
    fn prune_old_tool_results(working_history: &mut Vec<Message>) -> usize {
        let last_assistant_idx = working_history
            .iter()
            .rposition(|m| m.role == Role::Assistant);

        let Some(last_idx) = last_assistant_idx else {
            return 0;
        };

        // Pass 1: collect old summaries and mark indices for removal.
        let mut old_summaries: Vec<String> = Vec::new();
        let mut indices_to_remove: Vec<usize> = Vec::new();

        for (i, msg) in working_history.iter().enumerate() {
            if i >= last_idx {
                break;
            }
            match msg.role {
                Role::Assistant if !msg.tool_calls.is_empty() => {
                    let stripped = strip_think_tags(&msg.content);
                    let trimmed = stripped.trim();
                    if !trimmed.is_empty() {
                        old_summaries.push(trimmed.to_string());
                    }
                    indices_to_remove.push(i);
                }
                Role::Tool => {
                    indices_to_remove.push(i);
                }
                _ => {}
            }
        }

        if indices_to_remove.is_empty() {
            return 0;
        }

        // Pass 2: merge old summaries into the latest assistant message.
        if !old_summaries.is_empty() {
            let current_content = &working_history[last_idx].content;
            let merged = format!("{}\n\n{}", old_summaries.join("\n\n"), current_content);
            working_history[last_idx].content = merged;
        }

        // Pass 3: remove old messages (reverse order to preserve indices).
        let removed = indices_to_remove.len();
        for &idx in indices_to_remove.iter().rev() {
            working_history.remove(idx);
        }

        removed
    }

    /// Strip thinking/reasoning content from historical assistant messages.
    ///
    /// Two forms are handled:
    /// 1. `<think>...</think>` tags in `message.content` -- stripped
    /// 2. `metadata.reasoning_content` field -- removed from metadata JSON
    ///
    /// Only processes assistant messages that are NOT the most recent one.
    /// The latest assistant message's thinking is preserved because the
    /// current iteration result should not be altered.
    fn strip_historical_thinking(working_history: &mut [Message]) {
        let last_assistant_idx = working_history
            .iter()
            .rposition(|m| m.role == Role::Assistant);

        for (i, msg) in working_history.iter_mut().enumerate() {
            if msg.role != Role::Assistant {
                continue;
            }
            // Protect the most recent assistant message.
            if Some(i) == last_assistant_idx {
                continue;
            }

            // 1. Strip <think>...</think> from content.
            if msg.content.contains("<think>") {
                msg.content = strip_think_tags(&msg.content);
            }

            // 2. Remove reasoning_content from metadata.
            if let Some(obj) = msg.metadata.as_object_mut() {
                obj.remove("reasoning_content");
            }
        }
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

    /// Filter tool definitions by an agent's allowed/denied tool lists.
    ///
    /// Returns the raw [`ToolDefinition`]s so callers can both build JSON
    /// tool schemas and generate a tools summary for prompt injection.
    ///
    /// - `"*"` in `allowed` means all tools in the registry.
    /// - Empty `allowed` means no tools (returns empty vec).
    /// - `denied` overrides `allowed`.
    async fn filter_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<y_core::tool::ToolDefinition> {
        if allowed.is_empty() {
            return vec![];
        }

        let defs = container.tool_registry.get_all_definitions().await;
        let allow_all = allowed.iter().any(|a| a == "*");

        defs.into_iter()
            .filter(|def| {
                let name = def.name.as_str();
                let is_allowed = allow_all || allowed.iter().any(|a| a == name);
                let is_denied = denied.iter().any(|d| d == name);
                is_allowed && !is_denied
            })
            .collect()
    }

    /// Build tool definitions filtered by an agent's allowed/denied tool lists.
    ///
    /// Returns `OpenAI` function-calling JSON format. Delegates filtering to
    /// `filter_tool_definitions`.
    pub async fn build_filtered_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<serde_json::Value> {
        Self::filter_tool_definitions(container, allowed, denied)
            .await
            .iter()
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

    /// Check if a successful `FileWrite` just created an agent TOML and, if
    /// so, auto-register it so it takes effect immediately.
    ///
    /// Detection heuristic: the `path` argument ends with `.toml` and contains
    /// an `agents/` directory segment. Errors are logged but never propagated
    /// (auto-registration is best-effort).
    async fn maybe_auto_register_agent(
        container: &ServiceContainer,
        arguments: &serde_json::Value,
    ) {
        let path_str = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");

        if path_str.is_empty() {
            return;
        }

        let path = std::path::Path::new(path_str);

        // Only consider .toml files in an agents/ directory.
        let is_toml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
        let in_agents_dir = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .is_some_and(|name| name == "agents");

        if !is_toml || !in_agents_dir {
            return;
        }

        // Read the file from disk and attempt registration.
        match std::fs::read_to_string(path) {
            Ok(content) => match container.register_agent_from_toml(&content).await {
                Ok(id) => {
                    tracing::info!(
                        agent_id = %id,
                        path = %path_str,
                        "Auto-registered new agent definition from FileWrite"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path_str,
                        error = %e,
                        "Failed to auto-register agent from written file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path_str,
                    error = %e,
                    "Failed to read agent file for auto-registration"
                );
            }
        }
    }

    /// Execute a tool call -- delegates to the tool registry.
    ///
    /// Special handling for `ToolSearch` and `task`: these meta-tools are
    /// intercepted and routed to their respective orchestrators which have
    /// access to the full `ServiceContainer`.
    async fn execute_tool_call(
        container: &ServiceContainer,
        tc: &ToolCallRequest,
        session_id: &SessionId,
    ) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
        // Intercept ToolSearch calls — unified search across tools, skills, and agents.
        if tc.name == "ToolSearch" {
            let sources = crate::tool_search_orchestrator::CapabilitySearchSources {
                skill_search: Some(&container.skill_search),
                agent_registry: Some(&*container.agent_registry),
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

        // Intercept task calls — delegate to a sub-agent via AgentDelegator.
        if tc.name == "Task" {
            let session_uuid =
                uuid::Uuid::parse_str(session_id.as_str()).unwrap_or_else(|_| uuid::Uuid::new_v4());
            return crate::task_delegation_orchestrator::TaskDelegationOrchestrator::handle(
                &tc.arguments,
                container.agent_delegator.as_ref(),
                Some(session_uuid),
            )
            .await;
        }

        // Intercept workflow/schedule meta-tools -- route through orchestrator.
        {
            use crate::workflow_orchestrator::WorkflowOrchestrator as WO;
            let args = &tc.arguments;
            match tc.name.as_str() {
                "WorkflowCreate" => return WO::handle_create(args, container).await,
                "WorkflowList" => return WO::handle_list(args, container).await,
                "WorkflowGet" => return WO::handle_get(args, container).await,
                "WorkflowUpdate" => return WO::handle_update(args, container).await,
                "WorkflowDelete" => return WO::handle_delete(args, container).await,
                "WorkflowValidate" => return WO::handle_validate(args, container),
                "ScheduleCreate" => return WO::handle_schedule_create(args, container).await,
                "ScheduleList" => return WO::handle_schedule_list(args, container).await,
                "SchedulePause" => return WO::handle_schedule_pause(args, container).await,
                "ScheduleResume" => return WO::handle_schedule_resume(args, container).await,
                "ScheduleDelete" => return WO::handle_schedule_delete(args, container).await,
                _ => {} // fall through to normal tool dispatch
            }
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
// Think tag stripping
// ---------------------------------------------------------------------------

/// Remove all `<think>...</think>` blocks from a string.
///
/// Handles multiple consecutive blocks and unclosed tags (drops from the
/// opening tag to the end of the string).
fn strip_think_tags(content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end_offset) = result[start..].find("</think>") {
            // Remove <think>...</think> including the tags.
            let end = start + end_offset + "</think>".len();
            result = format!("{}{}", &result[..start], result[end..].trim_start());
        } else {
            // Unclosed <think> -- drop from tag to end.
            result.truncate(start);
            break;
        }
    }
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Sub-agent prompt augmentation
// ---------------------------------------------------------------------------

/// Build the effective system prompt for a sub-agent.
///
/// When `filtered_defs` is empty the base prompt is returned unchanged.
///
/// In [`ToolCallingMode::Native`] the base prompt is returned unchanged
/// because tools are sent via the API `tools` field -- no prompt injection
/// needed.
///
/// In [`ToolCallingMode::PromptBased`] the XML tool protocol and an
/// available-tools summary table are appended to the base prompt.
fn build_subagent_system_prompt(
    base_prompt: &str,
    filtered_defs: &[y_core::tool::ToolDefinition],
    tool_calling_mode: ToolCallingMode,
) -> String {
    if filtered_defs.is_empty() {
        return base_prompt.to_string();
    }

    let tool_protocol = y_prompt::PROMPT_TOOL_PROTOCOL;

    match tool_calling_mode {
        ToolCallingMode::Native => {
            // Native mode: tools are sent via the API `tools` field.
            // Still provide universal tool protocol rules, but no XML syntax.
            format!("{base_prompt}\n\n{tool_protocol}")
        }
        ToolCallingMode::PromptBased => {
            let tools_summary = crate::container::build_agent_tools_summary(filtered_defs);
            let syntax = y_tools::parser::PROMPT_TOOL_CALL_SYNTAX;
            format!("{base_prompt}\n\n{tool_protocol}\n\n{syntax}\n\n{tools_summary}")
        }
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

        // Filter tool definitions from allowed_tools/denied_tools.
        // When allowed_tools is non-empty, agents can make tool calls across
        // multiple iterations (e.g. skill-ingestion reading companion files).
        let filtered_defs = AgentService::filter_tool_definitions(
            &self.container,
            &config.allowed_tools,
            &config.denied_tools,
        )
        .await;

        // Convert filtered definitions to OpenAI function-calling JSON.
        let tool_definitions: Vec<serde_json::Value> = filtered_defs
            .iter()
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
            .collect();

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

        // Augment the system prompt with tool protocol and available-tools
        // summary when the agent has tools. In Native mode the XML tool
        // protocol is omitted (~800 tokens saved).
        let system_prompt =
            build_subagent_system_prompt(&config.system_prompt, &filtered_defs, tool_calling_mode);

        // Build messages: system_prompt + input as user message.
        let mut messages = Vec::with_capacity(2);
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: system_prompt.clone(),
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

        // Pick up a pre-created trace_id from the diagnostics context
        // (set via DIAGNOSTICS_CTX task-local by DiagnosticsAgentDelegator).
        let external_trace_id = y_diagnostics::DIAGNOSTICS_CTX
            .try_with(|ctx| ctx.trace_id)
            .ok();

        let exec_config = AgentExecutionConfig {
            agent_name: config.agent_name.clone(),
            system_prompt,
            max_iterations,
            tool_definitions,
            tool_calling_mode,
            messages,
            provider_id: None,
            preferred_models: config.preferred_models.clone(),
            provider_tags: config.provider_tags.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            thinking: None,
            session_id: None,
            session_uuid: Uuid::nil(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: user_content,
            external_trace_id,
            trust_tier: config.trust_tier,
            agent_allowed_tools: config.allowed_tools.clone(),
            prune_tool_history: config.prune_tool_history,
        };

        let result = AgentService::execute(&self.container, &exec_config, None, None)
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
    use y_core::permission_types::PermissionMode;
    use y_guardrails::{PermissionAction, PermissionDecision};

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

    // -- build_subagent_system_prompt tests --

    fn make_test_tool_def(name: &str) -> y_core::tool::ToolDefinition {
        y_core::tool::ToolDefinition {
            name: y_core::types::ToolName::from_string(name),
            description: format!("{name} description. Extra detail."),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "arg1": {"type": "string", "description": "First argument"}
                },
                "required": ["arg1"]
            }),
            result_schema: None,
            category: y_core::tool::ToolCategory::Shell,
            tool_type: y_core::tool::ToolType::BuiltIn,
            capabilities: Default::default(),
            is_dangerous: false,
        }
    }

    #[test]
    fn test_subagent_prompt_unchanged_without_tools() {
        let base = "You are a test agent.";
        let result = super::build_subagent_system_prompt(base, &[], ToolCallingMode::PromptBased);
        assert_eq!(result, base);
    }

    #[test]
    fn test_subagent_prompt_includes_protocol_and_summary() {
        let base = "You are a test agent.";
        let defs = vec![make_test_tool_def("ShellExec")];
        let result = super::build_subagent_system_prompt(base, &defs, ToolCallingMode::PromptBased);

        assert!(result.starts_with(base));
        assert!(result.contains("Tool Usage Protocol"));
        assert!(result.contains("## Available Tools"));
        assert!(result.contains("| ShellExec |"));
    }

    #[test]
    fn test_subagent_prompt_native_mode_returns_base_and_rules() {
        let base = "You are a test agent.";
        let defs = vec![make_test_tool_def("ShellExec")];
        let result = super::build_subagent_system_prompt(base, &defs, ToolCallingMode::Native);

        // Native mode: tools are sent via API field, prompt includes rules but no XML/summary.
        assert!(result.starts_with(base));
        assert!(result.contains("Tool Usage Protocol"));
        assert!(!result.contains("Available Tools"));
        assert!(!result.contains("<tool_call>"));
    }

    #[test]
    fn test_subagent_prompt_preserves_base() {
        let base = "Custom system prompt with specific instructions.";
        let defs = vec![make_test_tool_def("FileRead")];
        let result = super::build_subagent_system_prompt(base, &defs, ToolCallingMode::PromptBased);

        assert!(result.starts_with(base));
        assert!(result.contains("FileRead"));
    }

    #[test]
    fn test_session_allow_all_converts_ask_to_allow() {
        let decision = PermissionDecision {
            action: PermissionAction::Ask,
            reason: "global default policy".to_string(),
        };

        let resolved = AgentService::resolve_permission_decision_for_session(
            decision,
            Some(PermissionMode::BypassPermissions),
        );

        assert_eq!(resolved.action, PermissionAction::Allow);
        assert!(resolved.reason.contains("session"));
    }

    #[test]
    fn test_session_allow_all_does_not_override_deny() {
        let decision = PermissionDecision {
            action: PermissionAction::Deny,
            reason: "per-tool override for `ShellExec`".to_string(),
        };

        let resolved = AgentService::resolve_permission_decision_for_session(
            decision.clone(),
            Some(PermissionMode::BypassPermissions),
        );

        assert_eq!(resolved.action, PermissionAction::Deny);
        assert_eq!(resolved.reason, decision.reason);
    }

    // -----------------------------------------------------------------------
    // strip_think_tags tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_think_tags_basic() {
        let input = "<think>reasoning here</think>Final answer";
        assert_eq!(super::strip_think_tags(input), "Final answer");
    }

    #[test]
    fn test_strip_think_tags_multiple() {
        let input = "<think>first</think>Part A <think>second</think>Part B";
        assert_eq!(super::strip_think_tags(input), "Part A Part B");
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "Some text <think>never closed";
        assert_eq!(super::strip_think_tags(input), "Some text");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "No thinking here";
        assert_eq!(super::strip_think_tags(input), "No thinking here");
    }

    #[test]
    fn test_strip_think_tags_empty_think() {
        let input = "<think></think>Content";
        assert_eq!(super::strip_think_tags(input), "Content");
    }

    // -----------------------------------------------------------------------
    // prune_old_tool_results tests
    // -----------------------------------------------------------------------

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_assistant_with_tool_calls(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![y_core::types::ToolCallRequest {
                id: "tc_1".to_string(),
                name: "FileRead".to_string(),
                arguments: serde_json::json!({}),
            }],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_tool_result(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: content.to_string(),
            tool_call_id: Some("tc_1".to_string()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_prune_old_tool_results_merges_and_removes() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "user question"),
            // Old pair -- should be merged + removed
            make_assistant_with_tool_calls("<think>reasoning</think>Summary of chunk 1"),
            make_tool_result("raw chunk 1 contents"),
            // Current pair -- kept, with old summary prepended
            make_assistant_with_tool_calls("<think>more reasoning</think>Summary of chunk 2"),
            make_tool_result("raw chunk 2 contents"),
        ];

        let removed = AgentService::prune_old_tool_results(&mut history);
        assert_eq!(removed, 2); // old assistant + old tool removed
        assert_eq!(history.len(), 4); // system + user + merged assistant + current tool
        assert_eq!(history[0].role, Role::System);
        assert_eq!(history[1].role, Role::User);
        assert_eq!(history[2].role, Role::Assistant);
        assert_eq!(history[3].role, Role::Tool);

        // The merged assistant should contain old summary prepended to current.
        let merged = &history[2].content;
        assert!(
            merged.starts_with("Summary of chunk 1"),
            "old summary should be prepended"
        );
        assert!(
            merged.contains("<think>more reasoning</think>Summary of chunk 2"),
            "current content (including think tags) should be preserved"
        );
        // Old thinking tags should be stripped from the merged portion.
        assert!(
            !merged.contains("<think>reasoning</think>"),
            "old thinking should be stripped"
        );
    }

    #[test]
    fn test_prune_old_tool_results_three_iterations() {
        // Simulates three iterations of progressive summarization.
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "summarize document"),
            // Iteration 0
            make_assistant_with_tool_calls("chunk 1 summary"),
            make_tool_result("raw chunk 1"),
            // Iteration 1
            make_assistant_with_tool_calls("chunk 2 summary"),
            make_tool_result("raw chunk 2"),
            // Iteration 2 (latest)
            make_assistant_with_tool_calls("chunk 3 summary"),
            make_tool_result("raw chunk 3"),
        ];

        let removed = AgentService::prune_old_tool_results(&mut history);
        assert_eq!(removed, 4); // 2 old assistants + 2 old tools
        assert_eq!(history.len(), 4); // system + user + merged + latest tool

        let merged = &history[2].content;
        // All old summaries should be present in order.
        assert!(merged.contains("chunk 1 summary"));
        assert!(merged.contains("chunk 2 summary"));
        assert!(merged.contains("chunk 3 summary"));
        // Only the latest tool result should remain.
        assert_eq!(history[3].content, "raw chunk 3");
    }

    #[test]
    fn test_prune_old_tool_results_preserves_user_messages() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "question 1"),
            make_assistant_with_tool_calls("old summary"),
            make_tool_result("old result"),
            make_msg(Role::User, "question 2"),
            make_assistant_with_tool_calls("new summary"),
            make_tool_result("new result"),
        ];

        let removed = AgentService::prune_old_tool_results(&mut history);
        assert_eq!(removed, 2); // old assistant + old tool
        assert_eq!(history.len(), 5); // system + user1 + user2 + merged + tool
        assert!(history.iter().filter(|m| m.role == Role::User).count() == 2);
        // Merged assistant should have "old summary" prepended.
        let asst = history.iter().find(|m| m.role == Role::Assistant).unwrap();
        assert!(asst.content.contains("old summary"));
        assert!(asst.content.contains("new summary"));
    }

    #[test]
    fn test_prune_old_tool_results_no_assistant() {
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
        ];
        let removed = AgentService::prune_old_tool_results(&mut history);
        assert_eq!(removed, 0);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_prune_old_tool_results_single_pair() {
        // Only one assistant+tool pair -- nothing to prune.
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
            make_assistant_with_tool_calls("call tool"),
            make_tool_result("result"),
        ];
        let removed = AgentService::prune_old_tool_results(&mut history);
        assert_eq!(removed, 0);
        assert_eq!(history.len(), 4);
    }

    // -----------------------------------------------------------------------
    // strip_historical_thinking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_historical_thinking_removes_think_tags() {
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
            // Historical assistant -- should have <think> stripped
            {
                let mut m = make_msg(Role::Assistant, "<think>reasoning</think>Answer 1");
                m.metadata = serde_json::json!({"reasoning_content": "deep thought"});
                m
            },
            // Current (latest) assistant -- should be preserved
            {
                let mut m = make_msg(Role::Assistant, "<think>current reasoning</think>Answer 2");
                m.metadata = serde_json::json!({"reasoning_content": "current thought"});
                m
            },
        ];

        AgentService::strip_historical_thinking(&mut history);

        // Historical assistant: think tags and reasoning_content removed
        assert_eq!(history[2].content, "Answer 1");
        assert!(history[2].metadata.get("reasoning_content").is_none());

        // Current assistant: preserved intact
        assert!(history[3].content.contains("<think>"));
        assert!(history[3].metadata.get("reasoning_content").is_some());
    }

    #[test]
    fn test_strip_historical_thinking_skips_non_assistant() {
        let mut history = vec![
            make_msg(Role::User, "<think>user text</think>question"),
            make_msg(Role::Assistant, "answer"),
        ];
        AgentService::strip_historical_thinking(&mut history);
        // User message content should not be modified
        assert!(history[0].content.contains("<think>"));
    }
}
