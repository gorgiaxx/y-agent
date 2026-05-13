//! Chat command handlers -- send messages and stream LLM responses.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use y_core::provider::{ImageGenerationOptions, RequestMode};
use y_core::types::SessionId;
use y_service::chat_types::OperationMode;
use y_service::chat_types::PlanReviewDecision;
use y_service::chat_types::{
    ChatCheckpointInfo, CompactResult, MessageWithStatus, RestoreResult, TurnMeta, UndoResult,
};
use y_service::decode_session_prompt_config;
use y_service::event_sink::EventSink;
use y_service::{
    ChatService, PermissionPromptResponse, PrepareTurnRequest, PreparedTurn, ResendTurnRequest,
    TurnEvent,
};

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

/// Payload emitted on `chat:started` for `run_id` -> `session_id` mapping.
#[derive(Debug, Serialize, Clone)]
pub struct ChatStartedPayload {
    pub run_id: String,
    pub session_id: String,
}

/// Payload emitted on `chat:complete`.
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    /// Tokens actually occupying the context window (last LLM call's prompt
    /// size). Use this for the context-usage progress bar.
    pub context_tokens_used: u64,
}

/// Tool call summary in the completion payload.
/// Tool call summary in the completion payload.
#[derive(Debug, Serialize, Deserialize, Clone)]
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
    request_mode: Option<RequestMode>,
    skills: Option<Vec<String>>,
    knowledge_collections: Option<Vec<String>>,
    context_start_index: Option<usize>,
    thinking_effort: Option<String>,
    attachments: Option<Vec<serde_json::Value>>,
    plan_mode: Option<String>,
    operation_mode: Option<OperationMode>,
    mcp_mode: Option<String>,
    mcp_servers: Option<Vec<String>>,
    image_generation_options: Option<ImageGenerationOptions>,
) -> Result<ChatStarted, String> {
    tracing::info!(plan_mode = ?plan_mode, "chat_send: plan_mode received from frontend");
    if message.trim().is_empty() {
        return Err("Message must not be empty".into());
    }

    let thinking = thinking_effort.and_then(|e| {
        use y_core::provider::{ThinkingConfig, ThinkingEffort};
        let effort = match e.as_str() {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            _ => return None,
        };
        Some(ThinkingConfig { effort })
    });

    let run_id = uuid::Uuid::new_v4().to_string();

    // Build user message metadata from attachments.
    let user_message_metadata = attachments.map(|atts| serde_json::json!({ "attachments": atts }));

    // Prepare turn: resolve/create session, persist user message, read transcript.
    let mut prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: session_id.map(SessionId),
            user_input: message.clone(),
            provider_id: provider_id.clone(),
            request_mode,
            skills: skills.clone(),
            knowledge_collections: knowledge_collections.clone(),
            thinking,
            user_message_metadata,
            plan_mode,
            operation_mode,
            mcp_mode,
            mcp_servers,
            image_generation_options,
        },
    )
    .await
    .map_err(|e| format!("{e}"))?;

    // If context reset is active, trim history so only messages after the
    // reset point are sent to the LLM (fresh context).
    // Resolve from the frontend parameter first, falling back to the
    // DB-persisted value so context resets survive app restarts even if the
    // frontend hasn't loaded the value yet.
    let effective_start_idx = match context_start_index {
        Some(idx) => Some(idx),
        None => {
            // Fallback: read persisted context_reset_index from database.
            state
                .container
                .session_manager
                .get_context_reset_index(&prepared.session_id)
                .await
                .ok()
                .flatten()
                .map(|v| v as usize)
        }
    };
    if let Some(start_idx) = effective_start_idx {
        tracing::info!(
            session_id = %prepared.session_id.0,
            context_start_index = start_idx,
            history_len = prepared.history.len(),
            from_frontend = context_start_index.is_some(),
            "applying context reset: trimming history"
        );
        if start_idx < prepared.history.len() {
            prepared.history.drain(..start_idx);
        }
    }

    let sid = prepared.session_id.clone();
    let result_sid = sid.0.clone();
    let result_run_id = run_id.clone();

    apply_prepared_prompt_context(&state, &sid, &mut prepared).await;

    // Emit chat:started so the frontend can map run_id -> session_id
    // before any chat:progress events arrive.
    let _ = app.emit(
        "chat:started",
        ChatStartedPayload {
            run_id: run_id.clone(),
            session_id: sid.0.clone(),
        },
    );

    // Create a cancellation token for this run and register it so chat_cancel
    // can trigger it for immediate mid-LLM-call termination.
    let cancel_token = CancellationToken::new();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id.clone(), cancel_token.clone());
    }

    y_service::chat_worker::spawn_llm_worker(
        TauriEventSink(app),
        state.container.clone(),
        prepared,
        run_id.clone(),
        Arc::clone(&state.turn_meta_cache),
        Arc::clone(&state.pending_runs),
        cancel_token,
        true, // may generate title
    );

    Ok(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    })
}

// ---------------------------------------------------------------------------
// Tauri EventSink implementation
// ---------------------------------------------------------------------------

/// Tauri-based [`EventSink`] implementation that translates chat lifecycle
/// events into Tauri `emit()` calls with typed payloads.
struct TauriEventSink(AppHandle);

impl EventSink for TauriEventSink {
    fn emit_started(&self, run_id: &str, session_id: &str) {
        let _ = self.0.emit(
            "chat:started",
            ChatStartedPayload {
                run_id: run_id.to_owned(),
                session_id: session_id.to_owned(),
            },
        );
    }

    fn emit_progress(&self, run_id: &str, event: &TurnEvent) {
        let _ = self.0.emit(
            "chat:progress",
            ProgressPayload {
                run_id: run_id.to_owned(),
                event: event.clone(),
            },
        );
    }

    fn emit_ask_user(
        &self,
        run_id: &str,
        session_id: &str,
        interaction_id: &str,
        questions: &serde_json::Value,
    ) {
        let _ = self.0.emit(
            "chat:AskUser",
            AskUserPayload {
                run_id: run_id.to_owned(),
                session_id: session_id.to_owned(),
                interaction_id: interaction_id.to_owned(),
                questions: questions.clone(),
            },
        );
    }

    fn emit_permission_request(
        &self,
        run_id: &str,
        session_id: &str,
        request_id: &str,
        tool_name: &str,
        action_description: &str,
        reason: &str,
        content_preview: Option<&str>,
    ) {
        let _ = self.0.emit(
            "chat:PermissionRequest",
            PermissionRequestPayload {
                run_id: run_id.to_owned(),
                session_id: session_id.to_owned(),
                request_id: request_id.to_owned(),
                tool_name: tool_name.to_owned(),
                action_description: action_description.to_owned(),
                reason: reason.to_owned(),
                content_preview: content_preview.map(String::from),
            },
        );
    }

    fn emit_plan_review_request(
        &self,
        run_id: &str,
        session_id: &str,
        review_id: &str,
        plan: &serde_json::Value,
    ) {
        let _ = self.0.emit(
            "chat:PlanReview",
            PlanReviewRequestPayload {
                run_id: run_id.to_owned(),
                session_id: session_id.to_owned(),
                review_id: review_id.to_owned(),
                plan: plan.clone(),
            },
        );
    }

    fn emit_complete(&self, payload: &serde_json::Value) {
        // Deserialize the generic JSON payload into the typed
        // ChatCompletePayload that the Tauri frontend expects.
        if let Ok(typed) = serde_json::from_value::<ChatCompletePayload>(payload.clone()) {
            let _ = self.0.emit("chat:complete", typed);
        }
    }

    fn emit_error(&self, run_id: &str, session_id: &str, error: &str) {
        let _ = self.0.emit(
            "chat:error",
            ChatErrorPayload {
                run_id: run_id.to_owned(),
                session_id: session_id.to_owned(),
                error: error.to_owned(),
            },
        );
    }

    fn emit_title_updated(&self, session_id: &str, title: &str) {
        let _ = self.0.emit(
            "session:title_updated",
            TitleUpdatedPayload {
                session_id: session_id.to_owned(),
                title: title.to_owned(),
            },
        );
    }
}

async fn apply_prepared_prompt_context(
    state: &AppState,
    session_id: &SessionId,
    prepared: &mut PreparedTurn,
) {
    let workspace_path = super::workspace::resolve_workspace_path(&state.config_dir, &session_id.0);
    let session_prompt_config = decode_session_prompt_config(
        state
            .container
            .session_manager
            .get_custom_system_prompt(session_id)
            .await
            .unwrap_or(None),
    );

    let (agent_mode, working_directory, available_tools, agent_prompt, prompt_sections, permission) =
        prepared.agent_config.as_ref().map_or_else(
            || {
                (
                    String::new(),
                    resolve_turn_working_directory(None, workspace_path.clone(), &state.state_dir),
                    Vec::new(),
                    None,
                    None,
                    None,
                )
            },
            |config| {
                (
                    config.agent_mode.clone(),
                    resolve_turn_working_directory(
                        config.working_directory.clone(),
                        workspace_path.clone(),
                        &state.state_dir,
                    ),
                    if !config.features.toolcall || config.allowed_tools.is_empty() {
                        Vec::new()
                    } else {
                        config.allowed_tools.clone()
                    },
                    config.system_prompt.clone(),
                    (!config.prompt_section_ids.is_empty())
                        .then(|| config.prompt_section_ids.clone()),
                    config.permission_mode,
                )
            },
        );

    tracing::info!(
        session_id = %session_id.0,
        working_directory = ?working_directory,
        skills = ?prepared.skills,
        knowledge_collections = ?prepared.knowledge_collections,
        has_custom_prompt = session_prompt_config.system_prompt.is_some()
            || !session_prompt_config.prompt_section_ids.is_empty()
            || agent_prompt.is_some(),
        agent_mode = %agent_mode,
        "applied prompt context for prepared turn"
    );

    prepared.working_directory.clone_from(&working_directory);

    {
        let mut ctx = state.container.prompt_context.write().await;
        ctx.agent_mode = agent_mode;
        ctx.working_directory = working_directory;
        ctx.custom_system_prompt = session_prompt_config.system_prompt.or(agent_prompt);
        ctx.active_skills.clone_from(&prepared.skills);
        ctx.available_tools = available_tools;
        ctx.selected_prompt_sections = (!session_prompt_config.prompt_section_ids.is_empty())
            .then_some(session_prompt_config.prompt_section_ids)
            .or(prompt_sections);
    }

    if let Some(default_permission) = permission {
        let mut modes = state.container.session_permission_modes.write().await;
        modes
            .entry(session_id.clone())
            .or_insert(default_permission);
    }
}

fn resolve_turn_working_directory(
    agent_working_directory: Option<String>,
    workspace_path: Option<String>,
    state_dir: &Path,
) -> Option<String> {
    normalize_directory(agent_working_directory)
        .or_else(|| normalize_directory(workspace_path))
        .or_else(|| Some(state_dir.join("tmp").to_string_lossy().to_string()))
}

fn normalize_directory(path: Option<String>) -> Option<String> {
    path.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
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
    let summary = ChatService::get_last_turn_meta(&state.container, &session_id).await?;

    let meta = match summary {
        Some(s) => TurnMeta {
            provider_id: s.provider_id,
            model: s.model,
            input_tokens: s.input_tokens,
            output_tokens: s.output_tokens,
            cost_usd: s.cost_usd,
            context_window: s.context_window,
            context_tokens_used: s.context_tokens_used,
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
        messages_removed: result.messages_removed,
        restored_turn_number: result.rolled_back_to_turn,
        files_restored: u32::try_from(result.scopes_rolled_back.len()).unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// Chat resend command
// ---------------------------------------------------------------------------

/// Resend a user message by keeping it in the transcript and only removing
/// the assistant reply + subsequent tool messages. Then re-run the LLM.
///
/// Delegates domain logic to `ChatService::prepare_resend_turn()` and
/// spawns the LLM worker using the shared `chat_worker::spawn_llm_worker` helper.
#[tauri::command]
pub async fn chat_resend(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    checkpoint_id: String,
    provider_id: Option<String>,
    request_mode: Option<RequestMode>,
    knowledge_collections: Option<Vec<String>>,
    thinking_effort: Option<String>,
    plan_mode: Option<String>,
    operation_mode: Option<OperationMode>,
) -> Result<ChatStarted, String> {
    let thinking = thinking_effort.and_then(|e| {
        use y_core::provider::{ThinkingConfig, ThinkingEffort};
        let effort = match e.as_str() {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            _ => return None,
        };
        Some(ThinkingConfig { effort })
    });

    // Delegate domain logic to the service layer.
    let mut prepared = ChatService::prepare_resend_turn(
        &state.container,
        ResendTurnRequest {
            session_id: SessionId(session_id.clone()),
            checkpoint_id,
            provider_id,
            request_mode,
            knowledge_collections,
            thinking,
            plan_mode,
            operation_mode,
        },
    )
    .await
    .map_err(|e| format!("{e}"))?;

    let run_id = uuid::Uuid::new_v4().to_string();
    let result_sid = session_id.clone();
    let result_run_id = run_id.clone();

    let prepared_session_id = prepared.session_id.clone();
    apply_prepared_prompt_context(&state, &prepared_session_id, &mut prepared).await;

    let _ = app.emit(
        "chat:started",
        ChatStartedPayload {
            run_id: run_id.clone(),
            session_id: session_id.clone(),
        },
    );

    // Register cancellation token.
    let cancel_token = CancellationToken::new();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id.clone(), cancel_token.clone());
    }

    // Clear stale turn-meta cache.
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&session_id);
    }

    y_service::chat_worker::spawn_llm_worker(
        TauriEventSink(app),
        state.container.clone(),
        prepared,
        run_id.clone(),
        Arc::clone(&state.turn_meta_cache),
        Arc::clone(&state.pending_runs),
        cancel_token,
        false, // resend -- no title generation
    );

    Ok(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    })
}

// ---------------------------------------------------------------------------
// Chat checkpoint list command
// ---------------------------------------------------------------------------

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
/// previously did (`session_get_messages` + `chat_checkpoint_list` + index
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

    let Some(message_index) = message_index else {
        tracing::warn!("chat_find_checkpoint_for_resend: user message not found in transcript");
        return Ok(None);
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
                y_core::session::ChatMessageStatus::Pruned => "pruned".to_string(),
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

// ---------------------------------------------------------------------------
// Context compaction command (/compact slash command)
// ---------------------------------------------------------------------------

/// Manually trigger context compaction for a session.
///
/// Runs pruning first (unconditionally), then compaction (unconditionally).
/// Bypasses both the delta-based pruning threshold and the percentage-based
/// compaction threshold.
#[tauri::command]
pub async fn context_compact(
    _app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<CompactResult, String> {
    let sid = SessionId(session_id);
    let report = y_service::context_optimization::ContextOptimizationService::compact_now(
        &state.container,
        &sid,
    )
    .await
    .map_err(|e| format!("{e}"))?;

    Ok(CompactResult {
        messages_pruned: report.messages_pruned,
        messages_compacted: report.messages_compacted,
        tokens_saved: report.pruning_tokens_saved + report.compaction_tokens_saved,
        summary: report.compaction_summary,
    })
}

// ---------------------------------------------------------------------------
// User interaction (AskUser) commands
// ---------------------------------------------------------------------------

/// Payload emitted on `chat:AskUser` when the LLM needs user input.
#[derive(Debug, Serialize, Clone)]
pub struct AskUserPayload {
    pub run_id: String,
    pub session_id: String,
    pub interaction_id: String,
    pub questions: serde_json::Value,
}

/// Deliver the user's answer to a pending `AskUser` interaction.
///
/// Called by the frontend after the user selects options in the `AskUserDialog`.
/// The `interaction_id` must match the one from the `chat:AskUser` event.
#[tauri::command]
pub async fn chat_answer_question(
    state: State<'_, AppState>,
    interaction_id: String,
    answers: serde_json::Value,
) -> Result<bool, String> {
    let delivered =
        y_service::user_interaction_orchestrator::UserInteractionOrchestrator::deliver_answer(
            &interaction_id,
            answers,
            &state.container.pending_interactions,
        )
        .await;

    if !delivered {
        tracing::warn!(
            interaction_id = %interaction_id,
            "chat_answer_question: failed to deliver answer (interaction may have timed out)"
        );
    }

    Ok(delivered)
}

// ---------------------------------------------------------------------------
// Permission approval commands
// ---------------------------------------------------------------------------

/// Payload emitted on `chat:PermissionRequest` when a tool needs user approval.
#[derive(Debug, Serialize, Clone)]
pub struct PermissionRequestPayload {
    pub run_id: String,
    pub session_id: String,
    pub request_id: String,
    pub tool_name: String,
    pub action_description: String,
    pub reason: String,
    pub content_preview: Option<String>,
}

/// Deliver the user's permission decision (approve/deny) to a pending tool.
///
/// Called by the frontend after the user clicks Allow/Deny in the
/// permission prompt. The `request_id` must match the one from the
/// `chat:PermissionRequest` event.
#[tauri::command]
pub async fn chat_answer_permission(
    state: State<'_, AppState>,
    request_id: String,
    decision: PermissionPromptResponse,
) -> Result<bool, String> {
    let delivered = {
        let mut map = state.container.pending_permissions.lock().await;
        if let Some(pending) = map.remove(&request_id) {
            pending.send(decision).is_ok()
        } else {
            false
        }
    };

    if !delivered {
        tracing::warn!(
            request_id = %request_id,
            "chat_answer_permission: failed to deliver decision (request may have timed out)"
        );
    }

    Ok(delivered)
}

// ---------------------------------------------------------------------------
// Plan review commands
// ---------------------------------------------------------------------------

/// Payload emitted on `chat:PlanReview` when the plan orchestrator needs
/// user approval or rejection before executing phases.
#[derive(Debug, Serialize, Clone)]
pub struct PlanReviewRequestPayload {
    pub run_id: String,
    pub session_id: String,
    pub review_id: String,
    pub plan: serde_json::Value,
}

/// Deliver the user's plan review decision (approve/reject) to the waiting
/// plan orchestrator.
///
/// Called by the frontend after the user interacts with the plan review
/// dialog. The `review_id` must match the one from the `chat:PlanReview` event.
#[tauri::command]
pub async fn chat_answer_plan_review(
    state: State<'_, AppState>,
    review_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<bool, String> {
    let plan_decision = match decision.as_str() {
        "approve" => PlanReviewDecision::Approve,
        "revise" => PlanReviewDecision::Revise {
            feedback: feedback.unwrap_or_default(),
        },
        "reject" => PlanReviewDecision::Reject {
            feedback: feedback.unwrap_or_default(),
        },
        other => {
            return Err(format!("invalid plan review decision: {other}"));
        }
    };

    let delivered = y_service::plan_orchestrator::PlanOrchestrator::deliver_review_decision(
        &review_id,
        plan_decision,
        &state.container.pending_plan_reviews,
    )
    .await;

    if !delivered {
        tracing::warn!(
            review_id = %review_id,
            "chat_answer_plan_review: failed to deliver decision (review may have timed out)"
        );
    }

    Ok(delivered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn resolve_turn_working_directory_uses_tmp_when_session_has_no_workspace() {
        let state_dir = Path::new("/tmp/y-agent-state");

        assert_eq!(
            resolve_turn_working_directory(None, None, state_dir).as_deref(),
            Some("/tmp/y-agent-state/tmp")
        );
    }

    #[test]
    fn resolve_turn_working_directory_prefers_agent_then_workspace() {
        let state_dir = Path::new("/tmp/y-agent-state");

        assert_eq!(
            resolve_turn_working_directory(
                Some("/tmp/agent".to_string()),
                Some("/tmp/workspace".to_string()),
                state_dir,
            )
            .as_deref(),
            Some("/tmp/agent")
        );
        assert_eq!(
            resolve_turn_working_directory(None, Some("/tmp/workspace".to_string()), state_dir)
                .as_deref(),
            Some("/tmp/workspace")
        );
    }
}
