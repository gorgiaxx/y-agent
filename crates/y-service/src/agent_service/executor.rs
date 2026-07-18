//! Main execution loop for the agent service.
//!
//! Contains `execute_inner()` (the core tool-call loop) and
//! `init_context_and_trace()` (context pipeline + diagnostics setup).

use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_context::pruning::IntraTurnPruner;
use y_context::{AssembledContext, ContextCategory, ContextItem, ContextRequest};
use y_core::provider::{ProviderPool, ToolCallingMode};
use y_core::types::{Role, SessionId};
use y_diagnostics::TraceStore;
use y_tools::{parse_tool_calls, strip_tool_call_blocks};

use crate::container::ServiceContainer;
use crate::context_optimization::{ContextOptimizationService, WorkingHistoryOptimization};

use super::{
    llm, pruning, result, tool_handling, AgentExecutionConfig, AgentExecutionError,
    AgentExecutionResult, AgentService, FinalResultParams, InjectedSteer, LlmIterationData,
    ToolExecContext, TurnEvent, TurnEventSender,
};

pub(crate) struct ParentSubagentObservation {
    parent_trace_id: Uuid,
    observation_id: Uuid,
    child_trace_id: Uuid,
    child_session_id: Uuid,
    agent_name: String,
    started_at: Instant,
}

/// Context assembly + diagnostics trace initialisation.
///
/// Returns `(assembled_context, trace_id, owns_trace)`.
pub(crate) async fn init_context_and_trace(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
) -> (AssembledContext, Option<Uuid>, bool) {
    let mut assembled = if config.use_context_pipeline {
        // Update per-request tool protocol flag so the system prompt
        // includes/excludes XML tool protocol based on this request's mode.
        {
            let mut pctx = container.prompt_context.write().await;
            if config.working_directory.is_some() {
                pctx.working_directory.clone_from(&config.working_directory);
            }
            if config.tool_calling_mode == ToolCallingMode::PromptBased {
                pctx.config_flags
                    .insert("tool_calling.prompt_based".into(), true);
            } else {
                pctx.config_flags.remove("tool_calling.prompt_based");
            }
        }

        let request = ContextRequest {
            session_id: config.session_id.clone(),
            user_query: config.user_query.clone(),
            agent_mode: String::new(),
            tools_enabled: Vec::new(),
            knowledge_collections: config.knowledge_collections.clone(),
        };

        match container
            .context_pipeline
            .assemble_with_request(Some(request))
            .await
        {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, "context pipeline failed, using empty context");
                AssembledContext::default()
            }
        }
    } else {
        AssembledContext::default()
    };
    append_inherited_constraints_context(&mut assembled, config);
    append_reuse_recommendations_context(&mut assembled, config);

    // Diagnostics trace lifecycle.
    // If the caller already created a trace (external_trace_id), we reuse
    // it and do NOT close it ourselves (the caller closes).
    let trace_id = if let Some(eid) = config.external_trace_id {
        Some(eid)
    } else {
        // Start a new trace for this execution.
        let user_input = if config.user_query.trim().is_empty() {
            config
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone())
                .unwrap_or_default()
        } else {
            config.user_query.clone()
        };
        let tid = container
            .diagnostics
            .on_trace_start(config.session_uuid, &config.agent_name, &user_input)
            .await
            .ok();
        tid
    };
    let owns_trace = config.external_trace_id.is_none();

    if let Some(tid) = trace_id {
        merge_trace_metadata(container, tid, &config.trace_metadata).await;
    }

    (assembled, trace_id, owns_trace)
}

async fn merge_trace_metadata(
    container: &ServiceContainer,
    trace_id: Uuid,
    metadata: &serde_json::Value,
) {
    let Some(additions) = metadata.as_object() else {
        return;
    };
    let store = container.diagnostics.store();
    let Ok(mut trace) = store.get_trace(trace_id).await else {
        return;
    };
    if !trace.metadata.is_object() {
        trace.metadata = serde_json::json!({});
    }
    if let Some(existing) = trace.metadata.as_object_mut() {
        for (key, value) in additions {
            existing.insert(key.clone(), value.clone());
        }
    }
    if let Err(error) = store.update_trace(trace).await {
        tracing::warn!(%error, %trace_id, "failed to merge diagnostics trace metadata");
    }
}

fn append_inherited_constraints_context(
    assembled: &mut AssembledContext,
    config: &AgentExecutionConfig,
) {
    let Some(constraints) = config
        .inherited_constraints
        .as_ref()
        .filter(|constraints| !constraints.is_empty())
    else {
        return;
    };
    let content = constraints.to_system_prompt_section();
    assembled.add(ContextItem {
        category: ContextCategory::SystemPrompt,
        token_estimate: y_prompt::estimate_tokens(&content),
        priority: 95,
        content,
    });
}

fn append_reuse_recommendations_context(
    assembled: &mut AssembledContext,
    config: &AgentExecutionConfig,
) {
    let Some(value) = config
        .trace_metadata
        .pointer("/orchestration/reuse")
        .cloned()
    else {
        return;
    };
    let Ok(decision) =
        serde_json::from_value::<crate::capability_reuse::CapabilityReuseDecision>(value)
    else {
        return;
    };
    let Some(content) = decision.prompt_section() else {
        return;
    };
    assembled.add(ContextItem {
        category: ContextCategory::SystemPrompt,
        token_estimate: y_prompt::estimate_tokens(&content),
        priority: 96,
        content,
    });
}

pub(crate) async fn start_parent_subagent_observation(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    child_trace_id: Option<Uuid>,
) -> Option<ParentSubagentObservation> {
    if config.external_trace_id.is_some() {
        return None;
    }

    let child_trace_id = child_trace_id?;
    let parent_ctx = y_diagnostics::DIAGNOSTICS_CTX.try_with(Clone::clone).ok()?;
    if parent_ctx.trace_id == child_trace_id {
        return None;
    }

    let parent_id = *parent_ctx.last_gen_id.lock().await;
    let input = execution_input_snapshot(config);
    let observation_id = container
        .diagnostics
        .on_subagent_start(y_diagnostics::SubagentStartParams {
            trace_id: parent_ctx.trace_id,
            parent_id,
            session_id: parent_ctx.session_id,
            agent_name: config.agent_name.clone(),
            input,
            child_trace_id: Some(child_trace_id),
            child_session_id: Some(config.session_uuid),
        })
        .await
        .ok()?;

    Some(ParentSubagentObservation {
        parent_trace_id: parent_ctx.trace_id,
        observation_id,
        child_trace_id,
        child_session_id: config.session_uuid,
        agent_name: config.agent_name.clone(),
        started_at: Instant::now(),
    })
}

pub(crate) async fn finish_parent_subagent_observation(
    container: &ServiceContainer,
    observation: Option<ParentSubagentObservation>,
    execution_result: &Result<AgentExecutionResult, AgentExecutionError>,
) {
    let Some(observation) = observation else {
        return;
    };

    let success = execution_result.is_ok();
    let duration_ms = u64::try_from(observation.started_at.elapsed().as_millis()).unwrap_or(0);
    let (output, error_message) = match execution_result {
        Ok(result) => (
            Some(serde_json::json!({
                "content": result.content.clone(),
                "model": result.model.clone(),
                "provider_id": result.provider_id.clone(),
                "input_tokens": result.input_tokens,
                "output_tokens": result.output_tokens,
                "cost_usd": result.cost_usd,
                "iterations": result.iterations,
            })),
            None,
        ),
        Err(error) => (None, Some(error.to_string())),
    };

    let _ = container
        .diagnostics
        .on_subagent_complete(y_diagnostics::SubagentCompleteParams {
            trace_id: observation.parent_trace_id,
            observation_id: observation.observation_id,
            success,
            output,
            error_message,
            duration_ms,
        })
        .await;

    let _ =
        container
            .diagnostics_broadcast
            .send(y_diagnostics::DiagnosticsEvent::SubagentCompleted {
                trace_id: observation.child_trace_id,
                session_id: Some(observation.child_session_id),
                agent_name: observation.agent_name,
                success,
            });
}

fn execution_input_snapshot(config: &AgentExecutionConfig) -> serde_json::Value {
    if !config.user_query.trim().is_empty() {
        return serde_json::Value::String(config.user_query.clone());
    }

    config
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map_or(serde_json::Value::Null, |m| {
            serde_json::Value::String(m.content.clone())
        })
}

/// Materialize partial streaming content captured before an LLM failure into an
/// assistant message so it flows through `handle_llm_error` →
/// `persist_llm_error_partial_state` and survives on the display transcript.
///
/// Without this, text streamed before a mid-stream 504 is silently lost --
/// the display shows only the user message, and a retry wipes the entire turn.
fn materialize_partial_streaming(
    ctx: &mut ToolExecContext,
    partial_streaming: &mut llm::PartialStreamingContent,
) {
    if !partial_streaming.content.is_empty() {
        let partial_text = std::mem::take(&mut partial_streaming.content);
        ctx.accumulated_content.push_str(&partial_text);
        ctx.iteration_texts.push(partial_text.clone());

        let mut partial_meta = serde_json::json!({});
        if !partial_streaming.reasoning.is_empty() {
            let reasoning = std::mem::take(&mut partial_streaming.reasoning);
            partial_meta["reasoning_content"] = serde_json::Value::String(reasoning.clone());
            ctx.iteration_reasonings.push(Some(reasoning));
            ctx.iteration_reasoning_durations_ms.push(Some(0));
        }

        let partial_msg = y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: partial_text,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: partial_meta,
        };
        ctx.working_history.push(partial_msg.clone());
        ctx.new_messages.push(partial_msg);
    } else if !partial_streaming.reasoning.is_empty() {
        // Reasoning without content -- still record per-iteration state so
        // iteration_reasonings stays parallel with iteration_texts.
        ctx.iteration_reasonings
            .push(Some(std::mem::take(&mut partial_streaming.reasoning)));
        ctx.iteration_reasoning_durations_ms.push(Some(0));
    }
}

struct LlmCallAttempt {
    result: Result<(y_core::provider::ChatResponse, Option<u64>), y_core::provider::ProviderError>,
    fallback: String,
    partial_streaming: llm::PartialStreamingContent,
    started_at: Instant,
}

fn should_attempt_context_overflow_recovery(
    error: &y_core::provider::ProviderError,
    partial_streaming: &llm::PartialStreamingContent,
    cancel: Option<&CancellationToken>,
) -> bool {
    y_provider::classify_provider_error(error) == y_provider::StandardError::ContextWindowExceeded
        && partial_streaming.content.is_empty()
        && partial_streaming.reasoning.is_empty()
        && !cancel.is_some_and(CancellationToken::is_cancelled)
}

async fn call_llm_with_context_recovery(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    ctx: &mut ToolExecContext,
    request_prefix_len: usize,
    pool: &dyn ProviderPool,
    routes: &[y_core::provider::RouteRequest],
    context_window: usize,
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
) -> LlmCallAttempt {
    let initial_request = llm::build_chat_request(config, ctx);
    if let Err(error) = ContextOptimizationService::optimize_working_history_before_sampling(
        container,
        &ctx.session_id,
        &mut ctx.working_history,
        request_prefix_len,
        &initial_request,
        context_window,
        false,
    )
    .await
    {
        tracing::warn!(
            agent = %config.agent_name,
            %error,
            "sampling preflight compaction failed; sending unchanged history"
        );
    }

    let mut request = llm::build_chat_request(config, ctx);
    let mut fallback = serde_json::to_string(&request.messages).unwrap_or_default();
    let mut partial_streaming = llm::PartialStreamingContent::default();
    let mut started_at = Instant::now();
    let mut result = llm::call_llm(
        pool,
        &request,
        routes,
        progress,
        cancel,
        &config.agent_name,
        &mut partial_streaming,
    )
    .await;

    let overflow_without_output = result.as_ref().is_err_and(|error| {
        should_attempt_context_overflow_recovery(error, &partial_streaming, cancel)
    });

    if overflow_without_output {
        match ContextOptimizationService::optimize_working_history_before_sampling(
            container,
            &ctx.session_id,
            &mut ctx.working_history,
            request_prefix_len,
            &request,
            context_window,
            true,
        )
        .await
        {
            Ok(WorkingHistoryOptimization::Applied) => {
                tracing::info!(
                    agent = %config.agent_name,
                    "context overflow recovered by emergency in-memory compaction; retrying once"
                );
                request = llm::build_chat_request(config, ctx);
                fallback = serde_json::to_string(&request.messages).unwrap_or_default();
                partial_streaming = llm::PartialStreamingContent::default();
                started_at = Instant::now();
                result = llm::call_llm(
                    pool,
                    &request,
                    routes,
                    progress,
                    cancel,
                    &config.agent_name,
                    &mut partial_streaming,
                )
                .await;
            }
            Ok(WorkingHistoryOptimization::NotNeeded) => {
                tracing::warn!(
                    agent = %config.agent_name,
                    "context overflow recovery skipped because no safe compaction was available"
                );
            }
            #[cfg(feature = "compaction_prefire")]
            Ok(WorkingHistoryOptimization::Suppressed) => {
                tracing::warn!(
                    agent = %config.agent_name,
                    "context overflow recovery suppressed for unchanged compaction input"
                );
            }
            Err(error) => {
                tracing::warn!(
                    agent = %config.agent_name,
                    %error,
                    "context overflow recovery failed"
                );
            }
        }
    }

    LlmCallAttempt {
        result,
        fallback,
        partial_streaming,
        started_at,
    }
}

/// Inner execution loop, optionally running inside a `DIAGNOSTICS_CTX` scope.
pub(crate) async fn execute_inner(
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
        AgentService::build_chat_messages(&assembled, &config.messages)
    } else {
        config.messages.clone()
    };
    let request_prefix_len = working_history.len().saturating_sub(config.messages.len());

    let session_id = config
        .session_id
        .clone()
        .unwrap_or_else(|| SessionId("agent".into()));
    let working_directory = if let Some(path) = config.working_directory.clone() {
        Some(path)
    } else {
        let prompt_context = container.prompt_context.read().await;
        prompt_context.working_directory.clone()
    };

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
        last_cache_read_tokens: 0,
        last_cache_write_tokens: 0,
        trace_id,
        session_id,
        working_directory,
        additional_read_dirs: config.additional_read_dirs.clone(),
        working_history,
        accumulated_content: String::new(),
        iteration_texts: Vec::new(),
        iteration_reasonings: Vec::new(),
        iteration_reasoning_durations_ms: Vec::new(),
        iteration_tool_counts: Vec::new(),
        dynamic_tool_defs: Vec::new(),
        pending_interactions: container.session_state.pending_interactions.clone(),
        pending_permissions: container.session_state.pending_permissions.clone(),
        cancel_token: cancel.clone(),
        injected_steers: Vec::new(),
    };
    let mut final_model: Option<String> = None;
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
                return Err(AgentExecutionError::Cancelled {
                    partial_messages: std::mem::take(&mut ctx.new_messages),
                    accumulated_content: std::mem::take(&mut ctx.accumulated_content),
                    iteration_texts: std::mem::take(&mut ctx.iteration_texts),
                    iteration_reasonings: std::mem::take(&mut ctx.iteration_reasonings),
                    iteration_reasoning_durations_ms: std::mem::take(
                        &mut ctx.iteration_reasoning_durations_ms,
                    ),
                    iteration_tool_counts: std::mem::take(&mut ctx.iteration_tool_counts),
                    tool_calls_executed: std::mem::take(&mut ctx.tool_calls_executed),
                    iterations: ctx.iteration.saturating_sub(1),
                    input_tokens: ctx.cumulative_input_tokens,
                    output_tokens: ctx.cumulative_output_tokens,
                    cost_usd: ctx.cumulative_cost,
                    model: final_model.clone().unwrap_or_default(),
                    generated_images: Vec::new(),
                });
            }
        }

        // Steering: drain any queued user messages and inject them as user
        // turns before building the next LLM request. This is the LLM-call
        // boundary -- protocol-valid because the prior iteration appended all
        // its tool results before looping back here.
        take_and_inject_steer(container, progress.as_ref(), &mut ctx).await;

        // Intra-turn pruning: remove failed tool call branches from
        // working_history before building the next LLM request.
        if ctx.iteration > 0 {
            let prune_report =
                intra_turn_pruner.prune_working_history(&mut ctx.working_history, ctx.iteration);
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
                let pruned = pruning::prune_old_tool_results(&mut ctx.working_history);
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
            pruning::strip_historical_thinking(&mut ctx.working_history);
        }

        ctx.iteration += 1;
        if ctx.iteration > max_iterations {
            result::emit_loop_limit(
                progress.as_ref(),
                &ctx,
                max_iterations,
                container,
                owns_trace,
            )
            .await;
            return Err(AgentExecutionError::ToolLoopLimitExceeded { max_iterations });
        }

        let routes = llm::build_route_requests(config);
        let raw_pool = container.provider_pool().await;
        let preflight_context_window =
            llm::resolve_preflight_context_window(&raw_pool.list_metadata(), &routes);

        // Wrap the pool with the diagnostics gateway so non-streaming
        // LLM calls are automatically recorded. Streaming calls pass
        // through (the assembled response is recorded after consumption).
        let diag_pool = crate::diagnostics::DiagnosticsProviderPool::new(
            Arc::clone(&raw_pool) as Arc<dyn ProviderPool>,
            Arc::clone(&container.diagnostics),
            container.diagnostics_broadcast.clone(),
        );

        let attempt = call_llm_with_context_recovery(
            container,
            config,
            &mut ctx,
            request_prefix_len,
            &diag_pool,
            &routes,
            preflight_context_window,
            progress.as_ref(),
            cancel.as_ref(),
        )
        .await;
        let LlmCallAttempt {
            result: llm_result,
            fallback,
            mut partial_streaming,
            started_at: llm_start,
        } = attempt;

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

                let iter_data = llm::build_iteration_data(&response, &fallback, llm_start);

                ctx.cumulative_input_tokens += iter_data.resp_input_tokens;
                ctx.cumulative_output_tokens += iter_data.resp_output_tokens;
                ctx.cumulative_cost += iter_data.cost;
                ctx.last_input_tokens = iter_data.context_input_tokens;
                ctx.last_cache_read_tokens = iter_data.resp_cache_read_tokens;
                ctx.last_cache_write_tokens = iter_data.resp_cache_write_tokens;
                final_model = Some(response.model.clone());
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
                    result::record_generation_diagnostics(
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
                    // Track per-iteration reasoning before delegating to tool handling.
                    ctx.iteration_reasonings
                        .push(response.reasoning_content.clone());
                    ctx.iteration_reasoning_durations_ms
                        .push(iter_reasoning_duration_ms);
                    tool_handling::handle_native_tool_calls(
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
                // instead of using the native API. Always attempt
                // prompt-based parsing as a safety net.
                if try_fallback_prompt_based_tool_calls(
                    container,
                    config,
                    &response,
                    progress.as_ref(),
                    &iter_data,
                    &mut ctx,
                    iter_ctx_window,
                    iter_reasoning_duration_ms,
                )
                .await
                {
                    continue;
                }

                // At a natural stop, atomically select a pending steer first,
                // then the oldest TODO. If neither exists, close input
                // acceptance and finalize the run.
                if inject_next_run_input(
                    container,
                    progress.as_ref(),
                    &response,
                    &iter_data,
                    iter_ctx_window,
                    iter_reasoning_duration_ms,
                    &mut ctx,
                    &config.agent_name,
                )
                .await
                {
                    continue;
                }

                // No tool calls -- final text response.
                return result::build_final_result(
                    container,
                    config,
                    &response,
                    progress.as_ref(),
                    &iter_data,
                    ctx,
                    FinalResultParams {
                        final_model: final_model.unwrap_or_default(),
                        final_provider_id,
                        owns_trace,
                        context_window: iter_ctx_window,
                        reasoning_duration_ms: iter_reasoning_duration_ms,
                    },
                )
                .await;
            }
            Err(e) => {
                materialize_partial_streaming(&mut ctx, &mut partial_streaming);
                let elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);
                let model_name = config.preferred_models.first().cloned().unwrap_or_default();
                return result::handle_llm_error(
                    e,
                    elapsed_ms,
                    &model_name,
                    &fallback,
                    preflight_context_window,
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

/// Take the session's single pending steer and inject it into the running
/// conversation as a user message. Records the injection in
/// `ctx.injected_steers` and emits a `SteerInjected` progress event.
///
/// Returns `true` if a steer was injected. Shared by the root turn and every
/// sub-agent; whichever loop owns the session takes the pending slot.
pub(crate) async fn take_and_inject_steer(
    container: &ServiceContainer,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
) -> bool {
    let Some(steer) = crate::ChatService::take_pending_steer(container, &ctx.session_id).await
    else {
        return false;
    };
    inject_steer(progress, ctx, steer);
    true
}

/// Inject the pending steer into `ctx.working_history`, anchoring it at the
/// current iteration boundary.
fn inject_steer(
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
    steer: crate::chat::SteerMessage,
) {
    let after_iteration = ctx.iteration_texts.len();
    let message = y_core::types::Message {
        message_id: y_core::types::generate_message_id(),
        role: Role::User,
        content: steer.text.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        // Tag so the GUI can render this persisted user message as an inline
        // steer chip instead of a normal user bubble. Display-only: providers
        // serialize role/content/tool_calls, so this never reaches the LLM.
        metadata: serde_json::json!({ "kind": "steer", "steer_id": steer.id }),
    };
    ctx.working_history.push(message.clone());
    ctx.injected_steers.push(InjectedSteer {
        steer_id: steer.id.clone(),
        message,
        after_iteration,
    });
    if let Some(tx) = progress {
        let _ = tx.send(crate::chat::TurnEvent::SteerInjected {
            steer_id: steer.id,
            text: steer.text,
        });
    }
}

/// Try to parse prompt-based tool calls from the LLM response text as a
/// fallback when native tool calls are absent. Returns `true` if tool calls
/// were found and handled (the loop should continue), `false` otherwise.
///
/// Even in `Native` mode, some models/providers embed tool calls in text
/// output. This function attempts `parse_tool_calls` on the response content
/// and, if tool calls are found, delegates to `handle_prompt_based_tool_calls`.
async fn try_fallback_prompt_based_tool_calls(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    response: &y_core::provider::ChatResponse,
    progress: Option<&TurnEventSender>,
    data: &LlmIterationData,
    ctx: &mut ToolExecContext,
    context_window: usize,
    reasoning_duration_ms: Option<u64>,
) -> bool {
    let Some(text) = &response.content else {
        return false;
    };
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
    if parse_result.tool_calls.is_empty() {
        return false;
    }
    ctx.iteration_reasonings
        .push(response.reasoning_content.clone());
    ctx.iteration_reasoning_durations_ms
        .push(reasoning_duration_ms);
    tool_handling::handle_prompt_based_tool_calls(
        container,
        config,
        response,
        &parse_result,
        text,
        progress,
        data,
        ctx,
        context_window,
    )
    .await;
    true
}

/// Fold a no-tool-call response into history as an intermediate assistant turn,
/// then inject the pending steer. Used at the final boundary so the loop can
/// continue and the model incorporates the steer (mirrors codex's
/// `needs_follow_up`). The caller emits the LLM-response event beforehand.
fn fold_response_and_inject_steer(
    progress: Option<&TurnEventSender>,
    response: &y_core::provider::ChatResponse,
    iter_reasoning_duration_ms: Option<u64>,
    ctx: &mut ToolExecContext,
    steer: crate::chat::SteerMessage,
) {
    let iter_content = {
        let raw = response.content.clone().unwrap_or_default();
        let stripped = strip_tool_call_blocks(&raw);
        if stripped.is_empty() {
            raw
        } else {
            stripped
        }
    };
    let out_content = if iter_content.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", iter_content.trim())
    };
    ctx.accumulated_content.push_str(&out_content);
    ctx.iteration_texts.push(out_content.clone());
    ctx.iteration_tool_counts.push(0);
    ctx.iteration_reasonings
        .push(response.reasoning_content.clone());
    ctx.iteration_reasoning_durations_ms
        .push(iter_reasoning_duration_ms);
    let assistant_msg = result::build_assistant_msg(response, out_content, vec![]);
    ctx.working_history.push(assistant_msg.clone());
    ctx.new_messages.push(assistant_msg);
    inject_steer(progress, ctx, steer);
}

/// Drain the session's follow-up queue and inject each entry into the
/// running conversation as a user message after the agent's natural stop.
///
/// Returns `true` if follow-ups were injected (the loop should continue),
/// `false` if no input remained (the loop should finalize).
///
/// The current LLM response is folded into history as an intermediate
/// assistant turn before injecting the follow-up user messages.
async fn inject_next_run_input(
    container: &ServiceContainer,
    progress: Option<&TurnEventSender>,
    response: &y_core::provider::ChatResponse,
    data: &LlmIterationData,
    context_window: usize,
    reasoning_duration_ms: Option<u64>,
    ctx: &mut ToolExecContext,
    agent_name: &str,
) -> bool {
    let Some(next_input) =
        crate::ChatService::take_next_run_input_or_close(container, &ctx.session_id).await
    else {
        return false;
    };

    let follow_up = match next_input {
        crate::chat::PendingRunInput::Steer(steer) => {
            result::emit_llm_response(
                progress,
                response,
                data,
                ctx.iteration,
                vec![],
                context_window,
                agent_name,
            );
            fold_response_and_inject_steer(progress, response, reasoning_duration_ms, ctx, steer);
            return true;
        }
        crate::chat::PendingRunInput::FollowUp(follow_up) => follow_up,
    };

    // Emit the current response as an intermediate turn.
    result::emit_llm_response(
        progress,
        response,
        data,
        ctx.iteration,
        vec![],
        context_window,
        agent_name,
    );

    // Fold the response into history as an intermediate turn.
    let out_content = response.content.as_deref().unwrap_or("").trim();
    let out_content = if out_content.is_empty() {
        String::new()
    } else {
        format!("{out_content}\n")
    };
    ctx.accumulated_content.push_str(&out_content);
    ctx.iteration_texts.push(out_content.clone());
    ctx.iteration_tool_counts.push(0);
    ctx.iteration_reasonings
        .push(response.reasoning_content.clone());
    ctx.iteration_reasoning_durations_ms
        .push(reasoning_duration_ms);

    let assistant_msg = result::build_assistant_msg(response, out_content, vec![]);
    ctx.working_history.push(assistant_msg.clone());
    ctx.new_messages.push(assistant_msg);

    // Inject exactly one follow-up so each FIFO item receives its own response.
    let msg = y_core::types::Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: follow_up.text.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::json!({
            "kind": "follow_up",
            "follow_up_id": follow_up.id,
        }),
    };
    ctx.working_history.push(msg.clone());
    ctx.new_messages.push(msg);

    // Remove the injected item from the client's pending queue projection.
    if let Some(progress) = progress {
        let _ = progress.send(TurnEvent::FollowUpInjected {
            follow_up_id: follow_up.id,
            text: follow_up.text,
        });
    }

    true
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use tempfile::TempDir;
    use y_context::{CompactionConfig, CompactionEngine, CompactionLlm, CompactionLlmError};
    use y_core::provider::{
        ChatRequest, ChatResponse, ChatStreamResponse, FinishReason, ProviderError, ProviderPool,
        ProviderStatus, RouteRequest,
    };
    use y_core::types::ProviderId;

    use super::*;
    use crate::agent_service::AgentExecutionConfig;
    use crate::config::ServiceConfig;

    struct SuccessfulCompactionLlm;

    #[async_trait]
    impl CompactionLlm for SuccessfulCompactionLlm {
        async fn summarize(&self, _prompt: &str) -> Result<String, CompactionLlmError> {
            Ok("emergency summary".to_string())
        }
    }

    struct OverflowThenSuccessPool {
        calls: AtomicUsize,
        request_message_counts: StdMutex<Vec<usize>>,
        succeed_on_retry: bool,
    }

    impl OverflowThenSuccessPool {
        fn new(succeed_on_retry: bool) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                request_message_counts: StdMutex::new(Vec::new()),
                succeed_on_retry,
            }
        }
    }

    #[async_trait]
    impl ProviderPool for OverflowThenSuccessPool {
        async fn chat_completion(
            &self,
            request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatResponse, ProviderError> {
            self.request_message_counts
                .lock()
                .expect("request counts lock")
                .push(request.messages.len());
            let call_index = self.calls.fetch_add(1, Ordering::SeqCst);
            if call_index == 0 || !self.succeed_on_retry {
                return Err(ProviderError::Other {
                    message: "maximum context length exceeded".to_string(),
                });
            }
            Ok(ChatResponse {
                id: "response-1".to_string(),
                model: "test-model".to_string(),
                content: Some("recovered".to_string()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                usage: y_core::types::TokenUsage::default(),
                finish_reason: FinishReason::Stop,
                raw_request: None,
                raw_response: None,
                provider_id: None,
                generated_images: Vec::new(),
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
            _route: &RouteRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            Err(ProviderError::Other {
                message: "streaming is not used by this test".to_string(),
            })
        }

        fn report_error(&self, _provider_id: &ProviderId, _error: &ProviderError) {}

        async fn provider_statuses(&self) -> Vec<ProviderStatus> {
            Vec::new()
        }

        async fn freeze(&self, _provider_id: &ProviderId, _reason: String) {}

        async fn thaw(&self, _provider_id: &ProviderId) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    async fn make_test_container() -> (ServiceContainer, TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("tempdir");
        let config = ServiceConfig {
            storage: y_storage::StorageConfig {
                db_path: ":memory:".to_string(),
                pool_size: 1,
                wal_enabled: false,
                transcript_dir: tmpdir.path().join("transcripts"),
                ..y_storage::StorageConfig::default()
            },
            ..ServiceConfig::default()
        };
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    fn make_test_ctx(session_id: SessionId, iteration_texts: Vec<String>) -> ToolExecContext {
        ToolExecContext {
            iteration: 0,
            last_gen_id: None,
            tool_calls_executed: Vec::new(),
            new_messages: Vec::new(),
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cost: 0.0,
            last_input_tokens: 0,
            last_cache_read_tokens: 0,
            last_cache_write_tokens: 0,
            trace_id: None,
            session_id,
            working_directory: None,
            additional_read_dirs: Vec::new(),
            working_history: Vec::new(),
            accumulated_content: String::new(),
            iteration_texts,
            iteration_reasonings: Vec::new(),
            iteration_reasoning_durations_ms: Vec::new(),
            iteration_tool_counts: Vec::new(),
            dynamic_tool_defs: Vec::new(),
            pending_interactions: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_permissions: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            cancel_token: None,
            injected_steers: Vec::new(),
        }
    }

    fn message(role: Role, content: &str) -> y_core::types::Message {
        y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn test_execution_config(messages: Vec<y_core::types::Message>) -> AgentExecutionConfig {
        AgentExecutionConfig {
            agent_name: "test-agent".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: usize::MAX,
            tool_definitions: Vec::new(),
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages,
            provider_id: None,
            preferred_models: Vec::new(),
            provider_tags: Vec::new(),
            fallback_provider_tags: Vec::new(),
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: Vec::new(),
            temperature: None,
            max_tokens: None,
            thinking: None,
            session_id: Some(SessionId("sampling-recovery".to_string())),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: Vec::new(),
            use_context_pipeline: false,
            user_query: "recover from overflow".to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: Vec::new(),
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn context_overflow_compacts_and_retries_exactly_once() {
        let (mut container, _tmpdir) = make_test_container().await;
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(SuccessfulCompactionLlm),
        );
        let history = vec![
            message(Role::User, "old request one"),
            message(Role::Assistant, "old response one"),
            message(Role::User, "old request two"),
            message(Role::Assistant, "old response two"),
            message(Role::User, "recent request"),
            message(Role::Assistant, "recent response"),
        ];
        let config = test_execution_config(history.clone());
        let mut ctx = make_test_ctx(SessionId("sampling-recovery".to_string()), Vec::new());
        ctx.working_history = history;
        let pool = Arc::new(OverflowThenSuccessPool::new(true));
        let routes = llm::build_route_requests(&config);

        let attempt = call_llm_with_context_recovery(
            &container,
            &config,
            &mut ctx,
            0,
            pool.as_ref(),
            &routes,
            128_000,
            None,
            None,
        )
        .await;

        assert!(attempt.result.is_ok());
        assert_eq!(pool.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            *pool
                .request_message_counts
                .lock()
                .expect("request counts lock"),
            vec![6, 3]
        );
        assert_eq!(ctx.working_history.len(), 3);
        assert_eq!(ctx.working_history[0].role, Role::System);
        assert_eq!(ctx.working_history[0].content, "emergency summary");
    }

    #[tokio::test]
    async fn second_context_overflow_is_returned_without_a_third_attempt() {
        let (mut container, _tmpdir) = make_test_container().await;
        container.compaction_engine = CompactionEngine::with_llm(
            CompactionConfig::default(),
            Box::new(SuccessfulCompactionLlm),
        );
        let history = vec![
            message(Role::User, "old request one"),
            message(Role::Assistant, "old response one"),
            message(Role::User, "old request two"),
            message(Role::Assistant, "old response two"),
            message(Role::User, "recent request"),
            message(Role::Assistant, "recent response"),
        ];
        let config = test_execution_config(history.clone());
        let mut ctx = make_test_ctx(SessionId("sampling-recovery".to_string()), Vec::new());
        ctx.working_history = history;
        let pool = Arc::new(OverflowThenSuccessPool::new(false));
        let routes = llm::build_route_requests(&config);

        let attempt = call_llm_with_context_recovery(
            &container,
            &config,
            &mut ctx,
            0,
            pool.as_ref(),
            &routes,
            128_000,
            None,
            None,
        )
        .await;

        assert!(attempt.result.is_err());
        assert_eq!(pool.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn context_overflow_with_partial_output_is_not_recoverable() {
        let error = ProviderError::Other {
            message: "maximum context length exceeded".to_string(),
        };
        let partial = llm::PartialStreamingContent {
            content: "already streamed".to_string(),
            reasoning: String::new(),
        };

        assert!(!should_attempt_context_overflow_recovery(
            &error, &partial, None
        ));
    }

    #[test]
    fn inject_steer_appends_user_message_and_records_boundary() {
        // Two completed iterations already, so the injection boundary is 2.
        let mut ctx = make_test_ctx(SessionId("s".into()), vec!["a\n".into(), "b\n".into()]);
        let (tx, mut rx) = crate::chat::TurnEventSender::channel();
        let steer = crate::chat::SteerMessage::new("steer one".into());
        let steer_id = steer.id.clone();

        super::inject_steer(Some(&tx), &mut ctx, steer);

        assert_eq!(ctx.working_history.len(), 1);
        assert_eq!(ctx.working_history[0].role, Role::User);
        assert_eq!(ctx.working_history[0].content, "steer one");

        // The injected user message is tagged so the GUI renders it as an
        // inline steer chip rather than a normal user bubble.
        assert_eq!(ctx.working_history[0].metadata["kind"], "steer");
        assert_eq!(
            ctx.working_history[0].metadata["steer_id"],
            steer_id.as_str()
        );

        assert_eq!(ctx.injected_steers.len(), 1);
        assert_eq!(ctx.injected_steers[0].after_iteration, 2);
        assert_eq!(ctx.injected_steers[0].steer_id, steer_id);

        let mut events = Vec::new();
        while let Ok((ev, _session_id)) = rx.try_recv() {
            events.push(ev);
        }
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            crate::chat::TurnEvent::SteerInjected { text, .. } if text == "steer one"
        ));
    }

    #[tokio::test]
    async fn take_and_inject_steer_consumes_the_single_pending_slot() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("drain-sess".into());
        crate::ChatService::begin_follow_up_run(&container, &sid);
        let todo = crate::ChatService::add_follow_up(&container, &sid, "hello".into())
            .expect("active run should accept TODO");
        crate::ChatService::steer_follow_up(&container, &sid, &todo.id)
            .await
            .expect("TODO should become the pending steer");

        let mut ctx = make_test_ctx(sid.clone(), Vec::new());
        let injected = super::take_and_inject_steer(&container, None, &mut ctx).await;
        assert!(injected);
        assert_eq!(ctx.working_history.len(), 1);
        assert_eq!(ctx.injected_steers.len(), 1);

        // Nothing remains in the single pending slot.
        let again = super::take_and_inject_steer(&container, None, &mut ctx).await;
        assert!(!again);
        assert_eq!(ctx.working_history.len(), 1);
    }

    #[tokio::test]
    async fn follow_up_boundary_injects_only_oldest_todo() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("todo-fifo-session".into());
        crate::ChatService::begin_follow_up_run(&container, &sid);
        let first = crate::ChatService::add_follow_up(&container, &sid, "first".into())
            .expect("active run should accept first TODO");
        let second = crate::ChatService::add_follow_up(&container, &sid, "second".into())
            .expect("active run should accept second TODO");

        let response = y_core::provider::ChatResponse {
            id: "response-1".into(),
            model: "test-model".into(),
            content: Some("current task done".into()),
            reasoning_content: None,
            tool_calls: vec![],
            usage: y_core::types::TokenUsage::default(),
            finish_reason: y_core::provider::FinishReason::Stop,
            raw_request: None,
            raw_response: None,
            provider_id: None,
            generated_images: vec![],
        };
        let data = LlmIterationData {
            resp_input_tokens: 1,
            resp_output_tokens: 1,
            resp_cache_read_tokens: 0,
            resp_cache_write_tokens: 0,
            context_input_tokens: 1,
            cost: 0.0,
            llm_elapsed_ms: 1,
            prompt_preview: String::new(),
            response_text_raw: "current task done".into(),
        };
        let mut ctx = make_test_ctx(sid.clone(), Vec::new());
        let (tx, mut rx) = crate::chat::TurnEventSender::channel();

        assert!(
            super::inject_next_run_input(
                &container,
                Some(&tx),
                &response,
                &data,
                128,
                None,
                &mut ctx,
                "test-agent",
            )
            .await
        );

        assert_eq!(ctx.working_history.len(), 2);
        assert_eq!(ctx.working_history[1].content, "first");
        assert_eq!(ctx.working_history[1].metadata["follow_up_id"], first.id);
        assert_eq!(
            crate::ChatService::list_follow_ups(&container, &sid),
            vec![second]
        );
        let mut saw_injected = false;
        while let Ok((event, _)) = rx.try_recv() {
            if matches!(
                event,
                crate::chat::TurnEvent::FollowUpInjected { text, .. } if text == "first"
            ) {
                saw_injected = true;
            }
        }
        assert!(saw_injected);
    }

    #[tokio::test]
    async fn init_context_and_trace_records_actual_user_query_in_trace() {
        let (container, _tmpdir) = make_test_container().await;
        let config = AgentExecutionConfig {
            agent_name: "chat-turn".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: usize::MAX,
            tool_definitions: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: vec![],
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            fallback_provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: None,
            max_tokens: None,
            thinking: None,
            session_id: Some(SessionId("trace-session".into())),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: vec![],
            use_context_pipeline: true,
            user_query: "real user question".to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::json!({
                "orchestration": {
                    "requested_mode": "auto",
                    "selected_mode": "plan"
                }
            }),
        };

        let (_assembled, trace_id, owns_trace) = init_context_and_trace(&container, &config).await;

        assert!(owns_trace);
        let trace_id = trace_id.expect("trace should be created");
        let trace = container
            .diagnostics
            .store()
            .get_trace(trace_id)
            .await
            .expect("trace should be readable");

        assert_eq!(trace.user_input.as_deref(), Some("real user question"));
        assert_eq!(trace.metadata["orchestration"]["selected_mode"], "plan");
    }

    #[test]
    fn append_inherited_constraints_context_adds_system_prompt_item() {
        let mut assembled = AssembledContext::default();
        let config = AgentExecutionConfig {
            agent_name: "phase-agent".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: 1,
            tool_definitions: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: vec![],
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            fallback_provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: None,
            max_tokens: None,
            thinking: None,
            session_id: Some(SessionId("phase-session".into())),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: "phase task".to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: Some(y_core::agent::InheritedConstraints {
                scope_boundaries: vec!["crates/y-gui/".to_string()],
                guardrails: vec!["Use the parent report format".to_string()],
                output_format: None,
            }),
            trace_metadata: serde_json::Value::Null,
        };

        append_inherited_constraints_context(&mut assembled, &config);

        assert_eq!(assembled.items.len(), 1);
        assert_eq!(assembled.items[0].category, ContextCategory::SystemPrompt);
        assert!(assembled.items[0]
            .content
            .contains("## Inherited Constraints"));
        assert!(assembled.items[0].content.contains("- crates/y-gui/"));
    }

    #[test]
    fn append_reuse_recommendations_context_requires_reuse_before_creation() {
        let mut assembled = AssembledContext::default();
        let mut config = AgentExecutionConfig {
            agent_name: "chat-turn".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: 1,
            tool_definitions: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: vec![],
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            fallback_provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: None,
            max_tokens: None,
            thinking: None,
            session_id: Some(SessionId("reuse-session".into())),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: "Review these Rust files".to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::Value::Null,
        };
        config.trace_metadata = serde_json::json!({
            "orchestration": {
                "reuse": {
                    "reuse_before_create": true,
                    "recommendations": [{
                        "asset_type": "agent",
                        "id": "rust-reviewer",
                        "name": "rust-reviewer",
                        "score": 20,
                        "reason": "strong existing match",
                        "usage": "Delegate with Task"
                    }]
                }
            }
        });

        append_reuse_recommendations_context(&mut assembled, &config);

        assert_eq!(assembled.items.len(), 1);
        assert!(assembled.items[0]
            .content
            .contains("Reuse these existing assets before creating"));
        assert!(assembled.items[0].content.contains("rust-reviewer"));
    }

    #[tokio::test]
    async fn init_context_and_trace_syncs_explicit_working_directory_to_prompt_context() {
        let (container, _tmpdir) = make_test_container().await;
        let config = AgentExecutionConfig {
            agent_name: "phase-agent".to_string(),
            system_prompt: String::new(),
            max_iterations: 1,
            max_tool_calls: 1,
            tool_definitions: vec![],
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: vec![],
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            fallback_provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: Some("/repo/workspace".to_string()),
            additional_read_dirs: vec![],
            temperature: None,
            max_tokens: None,
            thinking: None,
            session_id: Some(SessionId("phase-session".into())),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: vec![],
            use_context_pipeline: true,
            user_query: "phase task".to_string(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::Value::Null,
        };

        let _ = init_context_and_trace(&container, &config).await;

        let pctx = container.prompt_context.read().await;
        assert_eq!(pctx.working_directory.as_deref(), Some("/repo/workspace"));
    }
}
