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
use y_tools::{parse_tool_calls, strip_tool_call_blocks};

use crate::container::ServiceContainer;

use super::{
    llm, pruning, result, tool_handling, AgentExecutionConfig, AgentExecutionError,
    AgentExecutionResult, AgentService, FinalResultParams, InjectedSteer, ToolExecContext,
    TurnEventSender,
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

    (assembled, trace_id, owns_trace)
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
        pending_interactions: container.pending_interactions.clone(),
        pending_permissions: container.pending_permissions.clone(),
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
        drain_and_inject_steers(container, progress.as_ref(), &mut ctx).await;

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

        let request = llm::build_chat_request(config, &ctx);
        let routes = llm::build_route_requests(config);
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

        let mut partial_streaming = llm::PartialStreamingContent::default();
        let llm_result = llm::call_llm(
            &diag_pool,
            &request,
            &routes,
            progress.as_ref(),
            cancel.as_ref(),
            &config.agent_name,
            &mut partial_streaming,
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

                let iter_data = llm::build_iteration_data(&response, &fallback, llm_start);

                ctx.cumulative_input_tokens += iter_data.resp_input_tokens;
                ctx.cumulative_output_tokens += iter_data.resp_output_tokens;
                ctx.cumulative_cost += iter_data.cost;
                ctx.last_input_tokens = iter_data.resp_input_tokens;
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
                        // Track per-iteration reasoning before delegating to tool handling.
                        ctx.iteration_reasonings
                            .push(response.reasoning_content.clone());
                        ctx.iteration_reasoning_durations_ms
                            .push(iter_reasoning_duration_ms);
                        tool_handling::handle_prompt_based_tool_calls(
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

                // No tool calls. If steers are queued, fold this response into
                // history as an intermediate turn and keep looping so the model
                // incorporates them; otherwise finalize.
                let pending = crate::ChatService::drain_steers(container, &ctx.session_id).await;
                if !pending.is_empty() {
                    result::emit_llm_response(
                        progress.as_ref(),
                        &response,
                        &iter_data,
                        ctx.iteration,
                        vec![],
                        iter_ctx_window,
                        &config.agent_name,
                    );
                    fold_response_and_inject_steers(
                        progress.as_ref(),
                        &response,
                        iter_reasoning_duration_ms,
                        &mut ctx,
                        pending,
                    );
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
                if !partial_streaming.content.is_empty() {
                    ctx.accumulated_content.push_str(&partial_streaming.content);
                    ctx.iteration_texts
                        .push(std::mem::take(&mut partial_streaming.content));
                }
                if !partial_streaming.reasoning.is_empty() {
                    ctx.iteration_reasonings
                        .push(Some(std::mem::take(&mut partial_streaming.reasoning)));
                    ctx.iteration_reasoning_durations_ms.push(Some(0));
                }
                let elapsed_ms = u64::try_from(llm_start.elapsed().as_millis()).unwrap_or(0);
                let model_name = config.preferred_models.first().cloned().unwrap_or_default();
                return result::handle_llm_error(
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

/// Drain the session's steering queue and inject each entry into the running
/// conversation as a user message. Records each injection in `ctx.injected_steers`
/// (with the iteration boundary) and emits a `SteerInjected` progress event so
/// the GUI can render the bubble live and drop the item from its queue.
///
/// Returns `true` if any steer was injected. Shared by the root turn and every
/// sub-agent (the queue is keyed by session id, so whichever loop is active
/// drains it).
pub(crate) async fn drain_and_inject_steers(
    container: &ServiceContainer,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
) -> bool {
    let steers = crate::ChatService::drain_steers(container, &ctx.session_id).await;
    if steers.is_empty() {
        return false;
    }
    inject_steers(progress, ctx, steers);
    true
}

/// Inject already-drained steers into `ctx.working_history` as user messages, in
/// FIFO order, anchoring each at the current iteration boundary.
fn inject_steers(
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
    steers: Vec<crate::chat::SteerMessage>,
) {
    let after_iteration = ctx.iteration_texts.len();
    for steer in steers {
        let message = y_core::types::Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: steer.text.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
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
}

/// Fold a no-tool-call response into history as an intermediate assistant turn,
/// then inject the given (already-drained) steers. Used at the final boundary so
/// the loop can continue and the model incorporates the steers (mirrors codex's
/// `needs_follow_up`). The caller emits the LLM-response event beforehand.
fn fold_response_and_inject_steers(
    progress: Option<&TurnEventSender>,
    response: &y_core::provider::ChatResponse,
    iter_reasoning_duration_ms: Option<u64>,
    ctx: &mut ToolExecContext,
    steers: Vec<crate::chat::SteerMessage>,
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
    inject_steers(progress, ctx, steers);
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::agent_service::AgentExecutionConfig;
    use crate::config::ServiceConfig;

    async fn make_test_container() -> (ServiceContainer, TempDir) {
        let tmpdir = tempfile::TempDir::new().expect("tempdir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
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

    #[test]
    fn inject_steers_appends_user_messages_and_records_boundary() {
        // Two completed iterations already, so the injection boundary is 2.
        let mut ctx = make_test_ctx(SessionId("s".into()), vec!["a\n".into(), "b\n".into()]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let s1 = crate::chat::SteerMessage::new("steer one".into());
        let s2 = crate::chat::SteerMessage::new("steer two".into());
        let (id1, id2) = (s1.id.clone(), s2.id.clone());

        super::inject_steers(Some(&tx), &mut ctx, vec![s1, s2]);

        assert_eq!(ctx.working_history.len(), 2);
        assert_eq!(ctx.working_history[0].role, Role::User);
        assert_eq!(ctx.working_history[0].content, "steer one");
        assert_eq!(ctx.working_history[1].content, "steer two");

        assert_eq!(ctx.injected_steers.len(), 2);
        assert_eq!(ctx.injected_steers[0].after_iteration, 2);
        assert_eq!(ctx.injected_steers[0].steer_id, id1);
        assert_eq!(ctx.injected_steers[1].steer_id, id2);

        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            crate::chat::TurnEvent::SteerInjected { text, .. } if text == "steer one"
        ));
    }

    #[tokio::test]
    async fn drain_and_inject_steers_drains_then_reports_empty() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("drain-sess".into());
        crate::ChatService::add_steer(&container, &sid, "hello".into()).await;
        crate::ChatService::add_steer(&container, &sid, "world".into()).await;

        let mut ctx = make_test_ctx(sid.clone(), Vec::new());
        let injected = super::drain_and_inject_steers(&container, None, &mut ctx).await;
        assert!(injected);
        assert_eq!(ctx.working_history.len(), 2);
        assert_eq!(ctx.injected_steers.len(), 2);
        assert!(crate::ChatService::list_steers(&container, &sid)
            .await
            .is_empty());

        // Nothing left to drain.
        let again = super::drain_and_inject_steers(&container, None, &mut ctx).await;
        assert!(!again);
        assert_eq!(ctx.working_history.len(), 2);
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
        };

        append_inherited_constraints_context(&mut assembled, &config);

        assert_eq!(assembled.items.len(), 1);
        assert_eq!(assembled.items[0].category, ContextCategory::SystemPrompt);
        assert!(assembled.items[0]
            .content
            .contains("## Inherited Constraints"));
        assert!(assembled.items[0].content.contains("- crates/y-gui/"));
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
        };

        let _ = init_context_and_trace(&container, &config).await;

        let pctx = container.prompt_context.read().await;
        assert_eq!(pctx.working_directory.as_deref(), Some("/repo/workspace"));
    }
}
