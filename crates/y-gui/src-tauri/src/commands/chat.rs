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
    pub session_id: String,
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
    pub session_id: String,
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

/// Payload emitted on `diagnostics:subagent_completed` so the frontend
/// can refresh subagent history in the Global diagnostics view.
#[derive(Debug, Serialize, Clone)]
pub struct SubagentCompletedPayload {
    pub agent_name: String,
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
    skills: Option<Vec<String>>,
    knowledge_collections: Option<Vec<String>>,
    context_start_index: Option<usize>,
) -> Result<ChatStarted, String> {
    if message.trim().is_empty() {
        return Err("Message must not be empty".into());
    }

    let run_id = uuid::Uuid::new_v4().to_string();

    // Prepare turn: resolve/create session, persist user message, read transcript.
    let mut prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: session_id.map(|s| SessionId(s)),
            user_input: message.clone(),
            provider_id: provider_id.clone(),
            skills: skills.clone(),
            knowledge_collections: knowledge_collections.clone(),
        },
    )
    .await
    .map_err(|e| format!("{e}"))?;

    // If context reset is active, trim history so only messages after the
    // reset point are sent to the LLM (fresh context).
    if let Some(start_idx) = context_start_index {
        if start_idx < prepared.history.len() {
            prepared.history.drain(..start_idx);
        }
    }

    let sid = prepared.session_id.clone();
    let result_sid = sid.0.clone();
    let result_run_id = run_id.clone();

    // Resolve workspace path for this session and update prompt context.
    // Also set active_skills if the user attached skills to this message.
    {
        let workspace_path = super::workspace::resolve_workspace_path(
            &state.config_dir,
            &sid.0,
        );
        tracing::info!(
            session_id = %sid.0,
            workspace_path = ?workspace_path,
            skills = ?skills,
            knowledge_collections = ?knowledge_collections,
            "chat_send: resolved workspace path for session"
        );
        let mut ctx = state.container.prompt_context.write().await;
        ctx.working_directory = workspace_path;
        if let Some(ref skill_names) = skills {
            ctx.active_skills = skill_names.clone();
        } else {
            ctx.active_skills.clear();
        }
    }

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
        let user_msg_count = prepared
            .history
            .iter()
            .filter(|m| m.role == y_core::types::Role::User)
            .count();
        let should_generate_title = title_interval > 0
            && user_msg_count > 0
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
                        session_id: sid_clone.0.clone(),
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
                            // Notify frontend that a subagent call finished so the
                            // Global diagnostics view can refresh.
                            let _ = app.emit(
                                "diagnostics:subagent_completed",
                                SubagentCompletedPayload {
                                    agent_name: "title-generator".to_string(),
                                },
                            );
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
                        session_id: sid_clone.0.clone(),
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
    // For cancel, the session_id is not easily recoverable from the backend
    // side (the run_id→session mapping lives in the frontend's ChatBus).
    // We send an empty string; the frontend falls back to its own mapping.
    let _ = app.emit(
        "chat:error",
        ChatErrorPayload {
            run_id,
            session_id: String::new(),
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

// ---------------------------------------------------------------------------
// Chat undo (rollback) command
// ---------------------------------------------------------------------------

/// Result of an undo (rollback) operation.
#[derive(Debug, Serialize, Clone)]
pub struct UndoResult {
    /// Messages remaining in the transcript after truncation.
    pub remaining_message_count: usize,
    /// Turn number the session was rolled back to.
    pub restored_turn_number: u32,
    /// Number of file journal scopes that need rollback (for info).
    pub files_restored: u32,
}

/// Roll the conversation back to a specific checkpoint.
///
/// Truncates the JSONL transcript to the checkpoint's `message_count_before`,
/// invalidates all checkpoints from that turn onward, and returns summary info.
#[tauri::command]
pub async fn chat_undo(
    state: State<'_, AppState>,
    session_id: String,
    checkpoint_id: String,
) -> Result<UndoResult, String> {
    let sid = SessionId(session_id.clone());
    let result = state
        .container
        .chat_checkpoint_manager
        .rollback_to(&sid, &checkpoint_id)
        .await
        .map_err(|e| format!("{e}"))?;

    // Clear stale turn-meta cache so the status bar refreshes.
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&session_id);
    }

    Ok(UndoResult {
        remaining_message_count: result.messages_removed, // messages_removed is what was cut
        restored_turn_number: result.rolled_back_to_turn,
        files_restored: result.scopes_rolled_back.len() as u32,
    })
}

// ---------------------------------------------------------------------------
// Chat resend command
// ---------------------------------------------------------------------------

/// Resend a user message by keeping it in the transcript and only removing
/// the assistant reply + subsequent tool messages. Then re-run the LLM.
///
/// Unlike undo + send, this preserves the original user message (same ID,
/// same timestamp) so the conversation history remains consistent.
///
/// Events emitted: same as `chat_send` (`chat:started`, `chat:progress`,
/// `chat:complete`, `chat:error`).
#[tauri::command]
pub async fn chat_resend(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    checkpoint_id: String,
    provider_id: Option<String>,
) -> Result<ChatStarted, String> {
    let sid = SessionId(session_id.clone());

    // 1. Load the checkpoint to find message_count_before.
    let checkpoint = state
        .container
        .chat_checkpoint_manager
        .checkpoint_store()
        .load(&checkpoint_id)
        .await
        .map_err(|e| format!("{e}"))?;

    // 2. Partial truncation: keep user message (message_count_before + 1),
    //    remove assistant reply and any tool messages after it.
    //    Truncate both display and context stores.
    let keep_count = checkpoint.message_count_before as usize + 1;
    state
        .container
        .session_manager
        .display_transcript_store()
        .truncate(&sid, keep_count)
        .await
        .map_err(|e| format!("{e}"))?;
    state
        .container
        .session_manager
        .transcript_store()
        .truncate(&sid, keep_count)
        .await
        .map_err(|e| format!("{e}"))?;

    // 3. Invalidate this checkpoint and all newer ones.
    state
        .container
        .chat_checkpoint_manager
        .checkpoint_store()
        .invalidate_after(&sid, checkpoint.turn_number.saturating_sub(1))
        .await
        .map_err(|e| format!("{e}"))?;

    // 4. Read display transcript (now ends with the original user message).
    let history = state
        .container
        .session_manager
        .read_display_transcript(&sid)
        .await
        .map_err(|e| format!("{e}"))?;

    if history.is_empty() {
        return Err("transcript is empty after truncation".into());
    }

    let user_input = history.last().unwrap().content.clone();
    let turn_number = history.len() as u32;
    let session_uuid = uuid::Uuid::parse_str(&session_id)
        .unwrap_or_else(|_| uuid::Uuid::new_v4());

    // 5. Build run_id and emit chat:started.
    let run_id = uuid::Uuid::new_v4().to_string();
    let result_sid = session_id.clone();
    let result_run_id = run_id.clone();

    let _ = app.emit("chat:started", ChatStartedPayload {
        run_id: run_id.clone(),
        session_id: session_id.clone(),
    });

    // Register cancellation token.
    let cancel_token = CancellationToken::new();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id.clone(), cancel_token.clone());
    }

    // Clear stale turn-meta cache.
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&session_id);
    }

    // 6. Spawn LLM worker (same pattern as chat_send).
    let container = state.container.clone();
    let sid_clone = sid.clone();
    let run_id_clone = run_id.clone();
    let turn_meta_cache = Arc::clone(&state.turn_meta_cache);
    let cancel_clone = cancel_token.clone();

    tokio::spawn(async move {
        let input = y_service::TurnInput {
            user_input: &user_input,
            session_id: sid_clone.clone(),
            session_uuid,
            history: &history,
            turn_number,
            provider_id: provider_id.clone(),
            knowledge_collections: vec![],
        };

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
                        session_id: sid_clone.0.clone(),
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
            }
            Err(e) => {
                let _ = app.emit(
                    "chat:error",
                    ChatErrorPayload {
                        run_id: run_id_clone,
                        session_id: sid_clone.0.clone(),
                        error: e.to_string(),
                    },
                );
            }
        }

        let _ = progress_task.await;
    });

    Ok(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    })
}

// ---------------------------------------------------------------------------
// Chat checkpoint list command
// ---------------------------------------------------------------------------

/// Info about a single chat checkpoint (for the frontend).
#[derive(Debug, Serialize, Clone)]
pub struct ChatCheckpointInfo {
    pub checkpoint_id: String,
    pub session_id: String,
    pub turn_number: u32,
    pub message_count_before: u32,
    pub created_at: String,
}

/// List all non-invalidated checkpoints for a session, ordered by turn number DESC.
#[tauri::command]
pub async fn chat_checkpoint_list(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<ChatCheckpointInfo>, String> {
    let sid = SessionId(session_id);
    let checkpoints = state
        .container
        .chat_checkpoint_manager
        .list_checkpoints(&sid)
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(checkpoints
        .into_iter()
        .map(|cp| ChatCheckpointInfo {
            checkpoint_id: cp.checkpoint_id,
            session_id: cp.session_id.0,
            turn_number: cp.turn_number,
            message_count_before: cp.message_count_before,
            created_at: cp.created_at.to_rfc3339(),
        })
        .collect())
}

/// Find the correct checkpoint for resending a user message.
///
/// Tries to find the user message by `message_id` first, then falls back to
/// content matching. Returns the checkpoint whose `message_count_before`
/// matches that message's index in the transcript.
///
/// This consolidates the multi-step checkpoint lookup that the frontend
/// previously did (session_get_messages + chat_checkpoint_list + index
/// matching) into a single atomic backend call.
#[tauri::command]
pub async fn chat_find_checkpoint_for_resend(
    state: State<'_, AppState>,
    session_id: String,
    user_message_content: String,
    message_id: Option<String>,
) -> Result<Option<ChatCheckpointInfo>, String> {
    let sid = SessionId(session_id);

    // 1. Read display transcript to find the user message's index.
    let messages = state
        .container
        .session_manager
        .read_display_transcript(&sid)
        .await
        .map_err(|e| format!("{e}"))?;

    // Try message_id first (precise), then fall back to content match (last occurrence).
    let message_index = message_id
        .as_ref()
        .and_then(|mid| {
            messages
                .iter()
                .enumerate()
                .find(|(_, m)| &m.message_id == mid)
                .map(|(idx, _)| idx)
        })
        .or_else(|| {
            messages
                .iter()
                .enumerate()
                .rev()
                .find(|(_, m)| {
                    m.role == y_core::types::Role::User && m.content == user_message_content
                })
                .map(|(idx, _)| idx)
        });

    let message_index = match message_index {
        Some(idx) => idx,
        None => {
            tracing::warn!(
                "chat_find_checkpoint_for_resend: user message not found in transcript"
            );
            return Ok(None);
        }
    };

    tracing::info!(
        message_index,
        message_id = message_id.as_deref().unwrap_or("none"),
        "chat_find_checkpoint_for_resend: found user message"
    );

    // 2. List non-invalidated checkpoints.
    let checkpoints = state
        .container
        .chat_checkpoint_manager
        .list_checkpoints(&sid)
        .await
        .map_err(|e| format!("{e}"))?;

    // 3. Find checkpoint whose message_count_before matches this message's index.
    //    Do NOT fallback to an arbitrary checkpoint -- returning the wrong one
    //    would cause the resend to truncate to the wrong point.
    let matched = checkpoints
        .iter()
        .find(|cp| cp.message_count_before as usize == message_index);

    if matched.is_none() {
        tracing::warn!(
            message_index,
            checkpoint_count = checkpoints.len(),
            "chat_find_checkpoint_for_resend: no checkpoint matched message_count_before"
        );
    }

    Ok(matched.map(|cp| ChatCheckpointInfo {
        checkpoint_id: cp.checkpoint_id.clone(),
        session_id: cp.session_id.0.clone(),
        turn_number: cp.turn_number,
        message_count_before: cp.message_count_before,
        created_at: cp.created_at.to_rfc3339(),
    }))
}

// ---------------------------------------------------------------------------
// Chat messages with status (Phase 2 — session history tree)
// ---------------------------------------------------------------------------

/// A message with its active/tombstone status for the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct MessageWithStatus {
    pub id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub checkpoint_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub context_window: Option<i64>,
    pub created_at: String,
}

/// Get all messages for a session, including tombstoned ones, with status.
#[tauri::command]
pub async fn chat_get_messages_with_status(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<MessageWithStatus>, String> {
    use y_core::session::ChatMessageStore;

    let sid = SessionId(session_id);
    let records = state
        .container
        .chat_message_store
        .list_by_session(&sid)
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(records
        .into_iter()
        .map(|r| {
            let status = match r.status {
                y_core::session::ChatMessageStatus::Active => "active".to_string(),
                y_core::session::ChatMessageStatus::Tombstone => "tombstone".to_string(),
            };
            MessageWithStatus {
                id: r.id,
                role: r.role,
                content: r.content,
                status,
                checkpoint_id: r.checkpoint_id,
                model: r.model,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: r.cost_usd,
                context_window: r.context_window,
                created_at: r.created_at.to_rfc3339(),
            }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Chat restore branch (Phase 2)
// ---------------------------------------------------------------------------

/// Result of a branch restoration.
#[derive(Debug, Serialize, Clone)]
pub struct RestoreResult {
    pub tombstoned_count: u32,
    pub restored_count: u32,
}

/// Swap the active and tombstoned branches at a checkpoint boundary.
#[tauri::command]
pub async fn chat_restore_branch(
    state: State<'_, AppState>,
    session_id: String,
    checkpoint_id: String,
) -> Result<RestoreResult, String> {
    use y_core::session::ChatMessageStore;

    let sid = SessionId(session_id.clone());
    let (tombstoned_count, restored_count) = state
        .container
        .chat_message_store
        .swap_branches(&sid, &checkpoint_id)
        .await
        .map_err(|e| format!("{e}"))?;

    // Clear stale turn-meta cache.
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&session_id);
    }

    Ok(RestoreResult {
        tombstoned_count,
        restored_count,
    })
}
