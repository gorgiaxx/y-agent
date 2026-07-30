//! Session command handlers — list, create, get messages, delete, truncate.

use tauri::State;

use y_core::session::SessionState;
use y_core::types::SessionId;
use y_service::{
    decode_session_prompt_config, encode_session_prompt_config, ChildSessionInfo, MessageInfo,
    SessionInfo, SessionPromptConfig, SessionService,
};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all sessions, sorted by last updated.
#[tauri::command]
pub async fn session_list(
    state: State<'_, AppState>,
    agent_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let workspaces = y_service::WorkspaceService::new(&state.config_dir);
    y_service::SessionService::backfill_legacy_assignments(
        &state.container.session_manager,
        &workspaces,
    )
    .await
    .map_err(|error| format!("Failed to migrate session workspaces: {error}"))?;
    let filter = y_core::session::SessionFilter {
        agent_id: agent_id.map(y_core::types::AgentId::from_string),
        state: Some(SessionState::Active),
        ..Default::default()
    };
    let sessions = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map_err(|e| format!("Failed to list sessions: {e}"))?;

    Ok(SessionService::session_infos(&state.container.session_manager, sessions).await)
}

/// List resumable sessions in exactly one workspace, sorted by last updated.
#[tauri::command]
pub async fn session_list_resumable(
    state: State<'_, AppState>,
    workspace_path: String,
    agent_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let workspaces = y_service::WorkspaceService::new(&state.config_dir);
    y_service::SessionService::backfill_legacy_assignments(
        &state.container.session_manager,
        &workspaces,
    )
    .await
    .map_err(|error| format!("Failed to migrate session workspaces: {error}"))?;

    let sessions = y_service::SessionService::list_resumable_sessions(
        &state.container.session_manager,
        std::path::Path::new(&workspace_path),
        agent_id.map(y_core::types::AgentId::from_string),
    )
    .await
    .map_err(|error| format!("Failed to list resumable sessions: {error}"))?;

    Ok(SessionService::session_infos(&state.container.session_manager, sessions).await)
}

/// List a session's direct child sessions (sub-agents), oldest first.
///
/// Powers drill-in: each plan phase / loop round / delegated task runs in its
/// own child session whose transcript is rendered with the same chat pipeline.
#[tauri::command]
pub async fn session_list_children(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<ChildSessionInfo>, String> {
    let sid = SessionId(session_id);
    SessionService::child_session_infos(&state.container.session_manager, &sid)
        .await
        .map_err(|error| format!("Failed to list child sessions: {error}"))
}

/// Create a new session.
#[tauri::command]
pub async fn session_create(
    state: State<'_, AppState>,
    title: Option<String>,
    agent_id: Option<String>,
    workspace_path: Option<String>,
) -> Result<SessionInfo, String> {
    SessionService::create_main_session(
        &state.container.session_manager,
        title,
        agent_id,
        workspace_path.as_deref(),
    )
    .await
    .map_err(|error| format!("Failed to create session: {error}"))
}

/// Get all messages in a session.
#[tauri::command]
pub async fn session_get_messages(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<MessageInfo>, String> {
    let sid = SessionId(session_id);

    SessionService::message_infos(&state.container.session_manager, &sid, None)
        .await
        .map_err(|error| format!("Failed to read display transcript: {error}"))
}

/// Delete a session from the GUI list.
///
/// Backend semantics are a soft-delete: mark the session tombstone and clear
/// transcript content. This keeps referential integrity for internal tables.
#[tauri::command]
pub async fn session_delete(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .delete_session(&sid)
        .await
        .map_err(|e| format!("Failed to delete session: {e}"))?;
    state.container.cleanup_session_state(&sid).await;
    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&sid.0);
    }
    Ok(())
}

/// Truncate a session's transcript to keep only the first `keep_count` messages.
///
/// This is used by the frontend to handle undo/resend after a cancelled run
/// where no checkpoint was created.
#[tauri::command]
pub async fn session_truncate_messages(
    state: State<'_, AppState>,
    session_id: String,
    keep_count: usize,
) -> Result<(), String> {
    let sid = SessionId(session_id);
    // Truncate both display and context transcript stores.
    state
        .container
        .session_manager
        .display_transcript_store()
        .truncate(&sid, keep_count)
        .await
        .map_err(|e| format!("Failed to truncate display transcript: {e}"))?;
    state
        .container
        .session_manager
        .transcript_store()
        .truncate(&sid, keep_count)
        .await
        .map_err(|e| format!("Failed to truncate context transcript: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Context reset persistence
// ---------------------------------------------------------------------------

/// Get the persisted context reset index for a session.
///
/// Returns `null` if no reset has been set (full context is used).
#[tauri::command]
pub async fn session_get_context_reset(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<u32>, String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .get_context_reset_index(&sid)
        .await
        .map_err(|e| format!("Failed to get context reset: {e}"))
}

/// Set or clear the context reset index for a session.
///
/// Pass `null` for `index` to clear (use full context).
#[tauri::command]
pub async fn session_set_context_reset(
    state: State<'_, AppState>,
    session_id: String,
    index: Option<u32>,
) -> Result<(), String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_context_reset_index(&sid, index)
        .await
        .map_err(|e| format!("Failed to set context reset: {e}"))
}

// ---------------------------------------------------------------------------
// Per-session custom system prompt
// ---------------------------------------------------------------------------

/// Get the custom system prompt for a session.
///
/// Returns `null` if no custom prompt has been set (global prompt is used).
#[tauri::command]
pub async fn session_get_custom_prompt(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    let sid = SessionId(session_id);
    let stored = state
        .container
        .session_manager
        .get_custom_system_prompt(&sid)
        .await
        .map_err(|e| format!("Failed to get custom prompt: {e}"))?;
    Ok(decode_session_prompt_config(stored).system_prompt)
}

/// Set or clear the custom system prompt for a session.
///
/// Pass `null` for `prompt` to clear (revert to global prompt).
#[tauri::command]
pub async fn session_set_custom_prompt(
    state: State<'_, AppState>,
    session_id: String,
    prompt: Option<String>,
) -> Result<(), String> {
    let sid = SessionId(session_id);
    let config = SessionPromptConfig {
        system_prompt: prompt,
        prompt_section_ids: Vec::new(),
        template_id: None,
    };
    state
        .container
        .session_manager
        .set_custom_system_prompt(&sid, encode_session_prompt_config(&config))
        .await
        .map_err(|e| format!("Failed to set custom prompt: {e}"))
}

/// Get the full prompt composition config for a session.
#[tauri::command]
pub async fn session_get_prompt_config(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionPromptConfig, String> {
    let sid = SessionId(session_id);
    let stored = state
        .container
        .session_manager
        .get_custom_system_prompt(&sid)
        .await
        .map_err(|e| format!("Failed to get prompt config: {e}"))?;
    Ok(decode_session_prompt_config(stored))
}

/// Set or clear the full prompt composition config for a session.
#[tauri::command]
pub async fn session_set_prompt_config(
    state: State<'_, AppState>,
    session_id: String,
    config: SessionPromptConfig,
) -> Result<(), String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_custom_system_prompt(&sid, encode_session_prompt_config(&config))
        .await
        .map_err(|e| format!("Failed to set prompt config: {e}"))
}

// ---------------------------------------------------------------------------
// Fork (branch) session
// ---------------------------------------------------------------------------

/// Fork a session at a specific message index, creating a new Branch session.
///
/// Copies messages `[0..=message_index]` from both transcripts into a new
/// independent session. The original session is never mutated.
///
/// Returns the newly created `SessionInfo` so the frontend can navigate to it.
#[tauri::command]
pub async fn session_fork(
    state: State<'_, AppState>,
    session_id: String,
    message_index: usize,
    title: Option<String>,
) -> Result<SessionInfo, String> {
    let sid = SessionId(session_id);
    let fork = state
        .container
        .session_manager
        .fork_session(&sid, message_index, title)
        .await
        .map_err(|e| format!("Failed to fork session: {e}"))?;

    Ok(SessionInfo {
        id: fork.id.0.clone(),
        agent_id: fork.agent_id.as_ref().map(|id| id.0.clone()),
        title: fork.title.clone(),
        manual_title: fork.manual_title.clone(),
        workspace_path: fork.workspace_path.clone(),
        created_at: fork.created_at.to_rfc3339(),
        updated_at: fork.updated_at.to_rfc3339(),
        message_count: fork.message_count as usize,
        has_custom_prompt: false,
    })
}

/// Rename a session (sets the manual title).
///
/// When a manual title is set, automatic title generation is disabled for
/// this session. Pass `null` for `title` to clear the manual title and
/// revert to auto-generated titles.
#[tauri::command]
pub async fn session_rename(
    state: State<'_, AppState>,
    session_id: String,
    title: Option<String>,
) -> Result<(), String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_manual_title(&sid, title)
        .await
        .map_err(|e| format!("Failed to rename session: {e}"))?;
    Ok(())
}
