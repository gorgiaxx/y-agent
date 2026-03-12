//! Chat command handlers — send messages and stream LLM responses.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::SessionId;
use y_service::{ChatService, TurnEvent, TurnInput};

use crate::state::AppState;

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
) -> Result<ChatStarted, String> {
    if message.trim().is_empty() {
        return Err("Message must not be empty".into());
    }

    let run_id = Uuid::new_v4().to_string();

    // Resolve or create session.
    let sid = match session_id {
        Some(ref id) => {
            let id = SessionId(id.clone());
            state
                .container
                .session_manager
                .get_session(&id)
                .await
                .map_err(|e| format!("Session not found: {e}"))?;
            id
        }
        None => {
            let session = state
                .container
                .session_manager
                .create_session(CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: None,
                })
                .await
                .map_err(|e| format!("Failed to create session: {e}"))?;
            session.id
        }
    };

    let result_sid = sid.0.clone();
    let result_run_id = run_id.clone();

    // Persist user message.
    let user_msg = y_core::types::Message {
        message_id: y_core::types::generate_message_id(),
        role: y_core::types::Role::User,
        content: message.clone(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    };
    let _ = state
        .container
        .session_manager
        .append_message(&sid, &user_msg)
        .await;

    // Spawn async LLM turn -- results streamed via events.
    let container = state.container.clone();
    let sid_clone = sid.clone();
    let run_id_clone = run_id.clone();

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
        let history = container
            .session_manager
            .read_transcript(&sid_clone)
            .await
            .unwrap_or_default();

        let turn_number = u32::try_from(history.len()).unwrap_or(u32::MAX);
        // Parse the session ID string as a UUID for diagnostics tracing so that
        // list_traces_by_session can correctly look up traces by session.
        // If the session ID is not a valid UUID (unlikely but possible with legacy data),
        // fall back to a deterministic v5 UUID derived from the session ID string.
        let session_uuid = Uuid::parse_str(sid_clone.as_str())
            .unwrap_or_else(|_| Uuid::new_v4());

        let input = TurnInput {
            user_input: &message,
            session_id: sid_clone.clone(),
            session_uuid,
            history: &history,
            turn_number,
        };

        // Determine whether to generate a title after this turn.
        // Mirror the TUI interval logic: generate on turn 1, then every N turns.
        let title_interval = container.session_manager.config().title_summarize_interval;
        // turn_number here == (history.len() before appending assistant reply).
        // After the user message is appended, history.len() reflects user messages + prior
        // assistant messages. user_msg_count is approximated as ceiling(history.len() / 2).
        let user_msg_count = ((turn_number as usize) + 1) / 2 + 1;
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
                let _ = app.emit(
                    "chat:complete",
                    ChatCompletePayload {
                        run_id: run_id_clone,
                        content: result.content,
                        model: result.model,
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
