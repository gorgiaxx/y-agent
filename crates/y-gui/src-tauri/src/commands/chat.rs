//! Chat command handlers — send messages and stream LLM responses.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use y_core::types::SessionId;
use y_service::{ChatService, PrepareTurnRequest, TurnEvent};

use crate::state::{AppState, TurnMeta};

// ---------------------------------------------------------------------------
// Response / event types
// ---------------------------------------------------------------------------

/// Returned immediately when a chat turn is started.
#[derive(Debug, Serialize, Clone)]
pub struct ChatStarted {
    /// Session ID (may have been auto-created).
    pub session_id: String,
    /// Unique run identifier for event correlation.
    pub run_id: String,
}

/// Payload emitted on `chat:started` for run_id -> session_id mapping.
#[derive(Debug, Serialize, Clone)]
pub struct ChatStartedPayload {
    pub run_id: String,
    pub session_id: String,
}

/// Payload emitted on `chat:complete`.
#[derive(Debug, Serialize, Clone)]
pub struct ChatCompletePayload {
    pub run_id: String,
    pub content: String,
    pub model: String,
    pub provider_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallInfo>,
    pub iterations: usize,
    /// Context window size of the serving provider (tokens).
    pub context_window: usize,
}

/// Tool call summary in the completion payload.
#[derive(Debug, Serialize, Clone)]
pub struct ToolCallInfo {
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Payload emitted on `chat:error`.
#[derive(Debug, Serialize, Clone)]
pub struct ChatErrorPayload {
    pub run_id: String,
    pub error: String,
}

/// Payload emitted on `chat:progress` for real-time turn diagnostics.
#[derive(Debug, Serialize, Clone)]
pub struct ProgressPayload {
    pub run_id: String,
    /// Forwarded event from the service layer.
    pub event: TurnEvent,
}

/// Payload emitted on `session:title_updated` after auto title generation.
#[derive(Debug, Serialize, Clone)]
pub struct TitleUpdatedPayload {
    pub session_id: String,
    pub title: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Send a chat message and receive streaming response via Tauri events.
///
/// Events emitted:
/// - `chat:complete` — full response on success
/// - `chat:error` — error details on failure
/// - `session:title_updated` — when a title is generated for the session
#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    message: String,
    session_id: Option<String>,
    provider_id: Option<String>,
) -> Result<ChatStarted, String> {
    if message.trim().is_empty() {
        return Err("Message must not be empty".into());
    }

    let run_id = uuid::Uuid::new_v4().to_string();

    // Prepare turn: resolve/create session, persist user message, read transcript.
    let prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: session_id.map(|s| SessionId(s)),
            user_input: message.clone(),
            provider_id: provider_id.clone(),
        },
    )
    .await
    .map_err(|e| format!("{e}"))?;

    let sid = prepared.session_id.clone();
    let result_sid = sid.0.clone();
    let result_run_id = run_id.clone();

    // Spawn async LLM turn -- results streamed via events.
    let container = state.container.clone();
    let sid_clone = sid.clone();
    let run_id_clone = run_id.clone();
    // Clone the Arc to the turn-meta cache so the spawned task can write to it
    // after a successful turn without holding a reference to `AppState`.
    let turn_meta_cache = Arc::clone(&state.turn_meta_cache);

    // Emit chat:started so the frontend can map run_id -> session_id
    // before any chat:progress events arrive.
    let _ = app.emit("chat:started", ChatStartedPayload {
        run_id: run_id.clone(),
        session_id: sid.0.clone(),
    });

    // Create a cancellation token for this run and register it so chat_cancel
    // can trigger it for immediate mid-LLM-call termination.
    let cancel_token = CancellationToken::new();
    let run_id_key = run_id_clone.clone();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id_key, cancel_token.clone());
    }

    // Spawn the LLM worker task.
    let cancel_clone = cancel_token.clone();
    tokio::spawn(async move {
        let input = prepared.as_turn_input();

        // Determine whether to generate a title after this turn.
        let title_interval = container.session_manager.config().title_summarize_interval;
        let user_msg_count = ((prepared.turn_number as usize) + 1) / 2 + 1;
        let should_generate_title = title_interval > 0
            && (user_msg_count == 1 || user_msg_count % title_interval as usize == 0);

        // Set up progress channel -- forward TurnEvents as Tauri events.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let app_progress = app.clone();
        let run_id_progress = run_id_clone.clone();
        let progress_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = app_progress.emit("chat:progress", ProgressPayload {
                    run_id: run_id_progress.clone(),
                    event,
                });
            }
        });

        match ChatService::execute_turn_with_progress(
            &container,
            &input,
            tx,
            Some(cancel_clone),
        )
        .await
        {
            Ok(result) => {
                // Cache last-turn metadata so the frontend can restore the
                // status bar when switching back to this session.
                let meta = TurnMeta {
                    provider_id: result.provider_id.clone(),
                    model: result.model.clone(),
                    input_tokens: result.input_tokens,
                    output_tokens: result.output_tokens,
                    cost_usd: result.cost_usd,
                    context_window: result.context_window,
                };
                if let Ok(mut cache) = turn_meta_cache.lock() {
                    cache.insert(sid_clone.0.clone(), meta);
                }

                let _ = app.emit(
                    "chat:complete",
                    ChatCompletePayload {
                        run_id: run_id_clone,
                        content: result.content,
                        model: result.model,
                        provider_id: result.provider_id,
                        input_tokens: result.input_tokens,
                        output_tokens: result.output_tokens,
                        cost_usd: result.cost_usd,
                        tool_calls: result
                            .tool_calls_executed
                            .iter()
                            .map(|tc| ToolCallInfo {
                                name: tc.name.clone(),
                                success: tc.success,
                                duration_ms: tc.duration_ms,
                            })
                            .collect(),
                        iterations: result.iterations,
                        context_window: result.context_window,
                    },
                );

                // Trigger title generation if the interval is reached.
                if should_generate_title {
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
                                    let _ = app.emit(
                                        "session:title_updated",
                                        TitleUpdatedPayload {
                                            session_id: sid_clone.0.clone(),
                                            title,
                                        },
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "title generation failed");
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
                let _ = app.emit(
                    "chat:error",
                    ChatErrorPayload {
                        run_id: run_id_clone,
                        error: e.to_string(),
                    },
                );
            }
        }

        // Ensure all progress events are forwarded before returning.
        let _ = progress_task.await;
    });

    Ok(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    })
}

// ---------------------------------------------------------------------------
// Cancel command
// ---------------------------------------------------------------------------

/// Abort an in-flight LLM run by run ID.
///
/// Emits `chat:error` with the error string "Cancelled" so the frontend
/// correctly finalises its streaming state for this run.
#[tauri::command]
pub async fn chat_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
) -> Result<(), String> {
    tracing::info!(run_id = %run_id, "chat_cancel: received");

    let token = {
        let mut runs = state
            .pending_runs
            .lock()
            .map_err(|_| "lock poisoned".to_string())?;
        let map_len = runs.len();
        let token = runs.remove(&run_id);
        tracing::info!(
            run_id = %run_id,
            found = token.is_some(),
            pending_count = map_len,
            "chat_cancel: token lookup"
        );
        token
    };
    if let Some(tok) = token {
        tracing::info!(run_id = %run_id, "chat_cancel: cancelling token");
        tok.cancel();
    } else {
        tracing::warn!(run_id = %run_id, "chat_cancel: no token found -- run may have already completed");
    }
    // Notify frontend regardless of whether a token was found so the UI
    // streaming state is always cleared.
    let _ = app.emit(
        "chat:error",
        ChatErrorPayload {
            run_id,
            error: "Cancelled".to_string(),
        },
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Session last-turn metadata command
// ---------------------------------------------------------------------------

/// Return the metadata of the last completed LLM turn for a session.
///
/// Two-tier lookup:
///   1. In-memory cache (populated by `chat_send` during this runtime).
///   2. Diagnostics database (survives restarts) -- fetches the most recent
///      Trace for the session, sums its token/cost totals, and resolves the
///      model name from the last Generation observation.  Context window is
///      looked up from the provider pool by matching the model name.
///
/// Returns `null` if neither source has data for this session.
#[tauri::command]
pub async fn session_last_turn_meta(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<TurnMeta>, String> {
    // --- Tier 1: in-memory cache ---
    {
        let cache = state
            .turn_meta_cache
            .lock()
            .map_err(|_| "lock poisoned".to_string())?;
        if let Some(meta) = cache.get(&session_id) {
            return Ok(Some(meta.clone()));
        }
    }

    // --- Tier 2: diagnostics database via service layer ---
    let summary = ChatService::get_last_turn_meta(&state.container, &session_id)
        .await
        .map_err(|e| format!("{e}"))?;

    let meta = match summary {
        Some(s) => TurnMeta {
            provider_id: s.provider_id,
            model: s.model,
            input_tokens: s.input_tokens,
            output_tokens: s.output_tokens,
            cost_usd: s.cost_usd,
            context_window: s.context_window,
        },
        None => return Ok(None),
    };

    // Warm the in-memory cache so subsequent switches are instant.
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.insert(session_id, meta.clone());
    }

    Ok(Some(meta))
}
