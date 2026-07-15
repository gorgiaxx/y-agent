//! Shared LLM worker -- extracts the duplicated `spawn_llm_worker` logic from
//! y-web and y-gui into a single, transport-agnostic implementation.
//!
//! Presentation layers provide an [`EventSink`] implementation to translate
//! lifecycle events into their own transport (SSE, Tauri emit, etc.).

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex, Once};

use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use y_core::types::{Message, Role};

use crate::chat::TurnEvent;
use crate::chat_types::TurnMeta;
use crate::event_sink::EventSink;
use crate::{ChatService, PreparedTurn, ServiceContainer};

/// Extract a human-readable message from a caught panic payload.
///
/// `catch_unwind` yields the payload as `Box<dyn Any + Send>`. The common panic
/// sources (`panic!("..")`, `unwrap`, `expect`, `unreachable!`) carry their
/// message as a `&'static str` or `String`; anything else falls back to a
/// placeholder so the surfaced error is never silently empty.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload (non-string)".to_string()
    }
}

static PANIC_LOGGER: Once = Once::new();

thread_local! {
    /// Panic `file:line` captured by the panic hook on the unwinding thread.
    ///
    /// `catch_unwind` recovers the payload but not the panic `Location`, so the
    /// hook stashes the location here for the recovery path to read. The hook
    /// runs synchronously on the same thread that unwinds, and `catch_unwind`
    /// catches on that same thread, so a thread-local round-trips the location
    /// correctly even under tokio's multi-threaded runtime.
    static LAST_PANIC_LOCATION: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Install a process-wide panic hook (once) that records the panic location
/// (`file:line`) for the recovery path and logs it via `tracing` before
/// delegating to the previous hook.
///
/// `catch_unwind` only recovers the payload, not the originating `Location`, so
/// without this hook a worker panic surfaces a message with no source position.
/// The hook runs synchronously on the unwinding thread, so it captures the real
/// panic site even though the worker recovers the payload afterwards. The
/// previous hook is preserved so default stderr/backtrace output is unchanged.
fn install_panic_logger() {
    PANIC_LOGGER.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info
                .location()
                .map_or_else(|| "<unknown>".to_string(), ToString::to_string);
            LAST_PANIC_LOCATION.with(|cell| {
                *cell.borrow_mut() = Some(location.clone());
            });
            tracing::error!(
                panic.location = %location,
                panic.message = %panic_message(info.payload()),
                "panic captured by worker panic logger"
            );
            prev(info);
        }));
    });
}

/// Take the panic location stashed by the hook on this thread, if any.
fn take_panic_location() -> Option<String> {
    LAST_PANIC_LOCATION.with(|cell| cell.borrow_mut().take())
}

/// Spawn the LLM worker task with progress forwarding and event emission.
///
/// This is the shared implementation used by both y-web and y-gui. It owns
/// all data needed for the spawned task and communicates results through the
/// provided `EventSink`.
///
/// The lifecycle:
/// 1. Determine whether to generate a title, and if so fire it concurrently
///    (it depends only on user messages, so it does not wait for the turn).
/// 2. Set up an mpsc progress channel and spawn a forwarding task.
/// 3. Execute the turn via `ChatService::execute_turn_with_progress`.
/// 4. On success: cache `TurnMeta`, emit complete.
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
    install_panic_logger();

    let sid_clone = prepared.session_id.clone();
    let run_id_clone = run_id;
    let panic_run_id = run_id_clone.clone();
    let cancel_clone = cancel_token;

    // Wrap sink in Arc so it can be shared between the progress task and the
    // main task (emit_complete / emit_error / emit_title_updated).
    let sink = Arc::new(sink);

    // Open TODO acceptance before clients observe the run as streaming.
    ChatService::begin_follow_up_run(&container, &sid_clone);
    sink.emit_started(&run_id_clone, &sid_clone.0);

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

            // Fire title generation concurrently with the turn. The title only
            // consumes the user messages already present in `prepared.history`,
            // so it does not need the assistant reply and must not block the
            // turn. Steering messages never reach this path (they are injected
            // mid-turn without spawning a worker), so they never trigger a
            // title regeneration.
            if do_title {
                let user_messages: Vec<Message> = prepared
                    .history
                    .iter()
                    .filter(|m| m.role == Role::User)
                    .cloned()
                    .collect();
                let title_container = Arc::clone(&container);
                let title_sink = Arc::clone(&sink_inner);
                let title_sid = sid_clone.clone();
                tokio::spawn(async move {
                    match title_container
                        .session_manager
                        .generate_title(
                            &*title_container.agent_delegator,
                            &title_sid,
                            &user_messages,
                        )
                        .await
                    {
                        Ok(title) => title_sink.emit_title_updated(&title_sid.0, &title),
                        Err(e) => tracing::warn!(error = %e, "title generation failed"),
                    }
                });
            }

            // Set up progress channel -- forward TurnEvents via the EventSink.
            let (tx, mut rx) = crate::chat::TurnEventSender::channel();
            let sink_progress = Arc::clone(&sink_inner);
            let run_id_progress = run_id_clone.clone();
            let session_id_progress = sid_clone.0.clone();
            let progress_task = tokio::spawn(async move {
                while let Some((event, child_session_id)) = rx.recv().await {
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

                    // Forward as generic progress event, attributed to the
                    // originating sub-agent session when present.
                    sink_progress.emit_progress(
                        &run_id_progress,
                        &event,
                        child_session_id
                            .as_ref()
                            .map(y_core::types::SessionId::as_str),
                    );
                }
            });

            let turn_result =
                ChatService::execute_turn_with_progress(&container, &input, tx, Some(cancel_clone))
                    .await;

            // Flush all remaining progress events before emitting the terminal
            // event so late-arriving stream_delta events do not arrive after
            // the frontend has already processed complete/error.
            let _ = progress_task.await;

            // Cancellation and provider errors can bypass the natural
            // empty-queue boundary. Close acceptance before the terminal event.
            ChatService::finish_follow_up_run(&container, &sid_clone).await;

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
                        cache_read_tokens: result.last_cache_read_tokens,
                        cache_write_tokens: result.last_cache_write_tokens,
                    };
                    if let Ok(mut cache) = turn_meta_cache.lock() {
                        cache.insert(sid_clone.0.clone(), meta);
                    }

                    let payload = serde_json::json!({
                        "run_id": run_id_clone,
                        "session_id": sid_clone.0,
                        "trace_id": result.trace_id,
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
                        "cache_read_tokens": result.last_cache_read_tokens,
                        "cache_write_tokens": result.last_cache_write_tokens,
                    });
                    sink_inner.emit_complete(&payload);
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
        let final_run_id = match result {
            Ok(rid) => rid,
            Err(payload) => {
                ChatService::finish_follow_up_run(&container, &sid_clone).await;
                let detail = panic_message(payload.as_ref());
                let location = take_panic_location();
                let location_suffix = location
                    .as_deref()
                    .map(|loc| format!(" (at {loc})"))
                    .unwrap_or_default();
                tracing::error!(
                    session_id = %sid_clone.0,
                    panic = %detail,
                    panic.location = location.as_deref().unwrap_or("<unknown>"),
                    "LLM worker panicked; emitting error event"
                );
                sink.emit_error(
                    &panic_run_id,
                    &sid_clone.0,
                    &format!("Internal error: LLM worker panicked: {detail}{location_suffix}"),
                );
                panic_run_id
            }
        };

        if !final_run_id.is_empty() {
            if let Ok(mut runs) = pending_runs.lock() {
                runs.remove(&final_run_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{install_panic_logger, panic_message, take_panic_location};

    #[test]
    fn test_panic_message_extracts_static_str() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(payload.as_ref()), "boom");
    }

    #[test]
    fn test_panic_message_extracts_owned_string() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("kaboom"));
        assert_eq!(panic_message(payload.as_ref()), "kaboom");
    }

    #[test]
    fn test_panic_message_falls_back_for_non_string_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42_u32);
        assert_eq!(
            panic_message(payload.as_ref()),
            "unknown panic payload (non-string)"
        );
    }

    #[test]
    fn test_panic_hook_records_location_for_recovery_path() {
        install_panic_logger();
        // Drain any location left by an earlier panic on this thread.
        let _ = take_panic_location();

        let result = std::panic::catch_unwind(|| panic!("intentional test panic"));
        assert!(result.is_err());

        let location = take_panic_location().expect("hook should record panic location");
        assert!(
            location.contains("chat_worker.rs"),
            "expected this file in location, got: {location}"
        );
        // Taking again yields None -- the location is consumed once.
        assert!(take_panic_location().is_none());
    }
}
