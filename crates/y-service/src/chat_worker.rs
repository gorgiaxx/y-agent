//! Shared LLM worker -- extracts the duplicated `spawn_llm_worker` logic from
//! y-web and y-gui into a single, transport-agnostic implementation.
//!
//! Presentation layers provide an [`EventSink`] implementation to translate
//! lifecycle events into their own transport (SSE, Tauri emit, etc.).

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::chat::TurnEvent;
use crate::chat_types::TurnMeta;
use crate::event_sink::EventSink;
use crate::{ChatService, PreparedTurn, ServiceContainer};

/// Spawn the LLM worker task with progress forwarding and event emission.
///
/// This is the shared implementation used by both y-web and y-gui. It owns
/// all data needed for the spawned task and communicates results through the
/// provided `EventSink`.
///
/// The lifecycle:
/// 1. Determine whether to generate a title for this session.
/// 2. Set up an mpsc progress channel and spawn a forwarding task.
/// 3. Execute the turn via `ChatService::execute_turn_with_progress`.
/// 4. On success: cache `TurnMeta`, emit complete, optionally generate title.
/// 5. On error: emit error.
/// 6. On panic: emit error.
/// 7. Cleanup: remove from `pending_runs`.
pub fn spawn_llm_worker<S: BuildHasher + Send + 'static>(
    sink: impl EventSink,
    container: Arc<ServiceContainer>,
    prepared: PreparedTurn,
    run_id: String,
    turn_meta_cache: Arc<Mutex<HashMap<String, TurnMeta, S>>>,
    pending_runs: Arc<Mutex<HashMap<String, CancellationToken, S>>>,
    cancel_token: CancellationToken,
    should_generate_title: bool,
) {
    let sid_clone = prepared.session_id.clone();
    let run_id_clone = run_id;
    let panic_run_id = run_id_clone.clone();
    let cancel_clone = cancel_token;

    // Wrap sink in Arc so it can be shared between the progress task and the
    // main task (emit_complete / emit_error / emit_title_updated).
    let sink = Arc::new(sink);

    tokio::spawn(async move {
        let sink_inner = Arc::clone(&sink);
        let result = std::panic::AssertUnwindSafe(async {
            let input = prepared.as_turn_input();

            // Check whether title generation should actually fire for this turn.
            let do_title = if should_generate_title {
                match container.session_manager.get_session(&sid_clone).await {
                    Ok(session) if session.session_type.is_user_facing() => {
                        if session.manual_title.is_some() {
                            false
                        } else {
                            ChatService::should_generate_title(&container, &prepared.history)
                        }
                    }
                    Ok(_) => false,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            session_id = %sid_clone.0,
                            "failed to resolve session type for title generation"
                        );
                        false
                    }
                }
            } else {
                false
            };

            // Set up progress channel -- forward TurnEvents via the EventSink.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let sink_progress = Arc::clone(&sink_inner);
            let run_id_progress = run_id_clone.clone();
            let session_id_progress = sid_clone.0.clone();
            let progress_task = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    // Intercept AskUser events.
                    if let TurnEvent::UserInteractionRequest {
                        ref interaction_id,
                        ref questions,
                    } = event
                    {
                        sink_progress.emit_ask_user(
                            &run_id_progress,
                            &session_id_progress,
                            interaction_id,
                            questions,
                        );
                    }

                    // Intercept PermissionRequest events.
                    if let TurnEvent::PermissionRequest {
                        ref request_id,
                        ref tool_name,
                        ref action_description,
                        ref reason,
                        ref content_preview,
                    } = event
                    {
                        sink_progress.emit_permission_request(
                            &run_id_progress,
                            &session_id_progress,
                            request_id,
                            tool_name,
                            action_description,
                            reason,
                            content_preview.as_deref(),
                        );
                    }

                    // Intercept PlanReviewRequest events.
                    if let TurnEvent::PlanReviewRequest {
                        ref review_id,
                        ref plan_title,
                        ref plan_file,
                        ref estimated_effort,
                        ref overview,
                        ref scope_in,
                        ref scope_out,
                        ref guardrails,
                        ref plan_content,
                        ref tasks,
                    } = event
                    {
                        let plan_payload = serde_json::json!({
                            "plan_title": plan_title,
                            "plan_file": plan_file,
                            "estimated_effort": estimated_effort,
                            "overview": overview,
                            "scope_in": scope_in,
                            "scope_out": scope_out,
                            "guardrails": guardrails,
                            "plan_content": plan_content,
                            "tasks": tasks,
                        });
                        sink_progress.emit_plan_review_request(
                            &run_id_progress,
                            &session_id_progress,
                            review_id,
                            &plan_payload,
                        );
                    }

                    // Forward as generic progress event.
                    sink_progress.emit_progress(&run_id_progress, &event);
                }
            });

            let turn_result =
                ChatService::execute_turn_with_progress(&container, &input, tx, Some(cancel_clone))
                    .await;

            // Flush all remaining progress events before emitting the terminal
            // event so late-arriving stream_delta events do not arrive after
            // the frontend has already processed complete/error.
            let _ = progress_task.await;

            match turn_result {
                Ok(result) => {
                    // Cache last-turn metadata for the presentation layer.
                    let meta = TurnMeta {
                        provider_id: result.provider_id.clone(),
                        model: result.model.clone(),
                        input_tokens: result.input_tokens,
                        output_tokens: result.output_tokens,
                        cost_usd: result.cost_usd,
                        context_window: result.context_window,
                        context_tokens_used: result.last_input_tokens,
                    };
                    if let Ok(mut cache) = turn_meta_cache.lock() {
                        cache.insert(sid_clone.0.clone(), meta);
                    }

                    let payload = serde_json::json!({
                        "run_id": run_id_clone,
                        "session_id": sid_clone.0,
                        "content": result.content,
                        "model": result.model,
                        "provider_id": result.provider_id,
                        "input_tokens": result.input_tokens,
                        "output_tokens": result.output_tokens,
                        "cost_usd": result.cost_usd,
                        "tool_calls": result.tool_calls_executed.iter().map(|tc| {
                            serde_json::json!({
                                "name": tc.name,
                                "success": tc.success,
                                "duration_ms": tc.duration_ms,
                            })
                        }).collect::<Vec<_>>(),
                        "iterations": result.iterations,
                        "context_window": result.context_window,
                        "context_tokens_used": result.last_input_tokens,
                    });
                    sink_inner.emit_complete(&payload);

                    // Title generation.
                    if do_title {
                        match container.session_manager.read_transcript(&sid_clone).await {
                            Ok(transcript) => {
                                match container
                                    .session_manager
                                    .generate_title(
                                        &*container.agent_delegator,
                                        &sid_clone,
                                        &transcript,
                                    )
                                    .await
                                {
                                    Ok(title) => {
                                        sink_inner.emit_title_updated(&sid_clone.0, &title);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "title generation failed"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "failed to read transcript for title generation"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    sink_inner.emit_error(&run_id_clone, &sid_clone.0, &e.to_string());
                }
            }

            run_id_clone
        })
        .catch_unwind()
        .await;

        // Clean up pending_runs regardless of success/panic.
        let final_run_id = if let Ok(rid) = result {
            rid
        } else {
            tracing::error!(
                session_id = %sid_clone.0,
                "LLM worker panicked; emitting error event"
            );
            sink.emit_error(
                &panic_run_id,
                &sid_clone.0,
                "Internal error: LLM worker panicked",
            );
            panic_run_id
        };

        if !final_run_id.is_empty() {
            if let Ok(mut runs) = pending_runs.lock() {
                runs.remove(&final_run_id);
            }
        }
    });
}
