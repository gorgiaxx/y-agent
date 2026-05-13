//! Session command handlers — list, create, get messages, delete, truncate.

use serde::Serialize;
use tauri::State;

use y_core::session::{CreateSessionOptions, SessionFilter, SessionState, SessionType};
use y_core::types::SessionId;
use y_service::{
    decode_session_prompt_config, encode_session_prompt_config, session_prompt_config_has_content,
    SessionPromptConfig,
};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Session info returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub agent_id: Option<String>,
    pub title: Option<String>,
    pub manual_title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub has_custom_prompt: bool,
}

fn is_user_visible_session(session_type: SessionType, state: SessionState) -> bool {
    session_type.is_user_facing() && state == SessionState::Active
}

/// A message in the session transcript.
#[derive(Debug, Serialize, Clone)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub tool_calls: Vec<ToolCallBrief>,
    /// Arbitrary metadata (model info, tool results, usage, etc.).
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    /// Skill names attached to this user message (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

/// Brief tool call info for display.
#[derive(Debug, Serialize, Clone)]
pub struct ToolCallBrief {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all sessions, sorted by last updated.
#[tauri::command]
pub async fn session_list(
    state: State<'_, AppState>,
    agent_id: Option<String>,
) -> Result<Vec<SessionInfo>, String> {
    let filter = SessionFilter {
        agent_id: agent_id.map(y_core::types::AgentId::from_string),
        state: Some(SessionState::Active),
        ..SessionFilter::default()
    };
    let sessions = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map_err(|e| format!("Failed to list sessions: {e}"))?;

    // Collect session IDs that have custom prompt composition.
    let mut custom_prompt_ids = std::collections::HashSet::new();
    for s in &sessions {
        if let Ok(stored) = state
            .container
            .session_manager
            .get_custom_system_prompt(&s.id)
            .await
        {
            let config = decode_session_prompt_config(stored);
            if session_prompt_config_has_content(&config) {
                custom_prompt_ids.insert(s.id.0.clone());
            }
        }
    }

    let mut infos: Vec<SessionInfo> = sessions
        .into_iter()
        .filter(|session| is_user_visible_session(session.session_type, session.state))
        .map(|s| {
            let has_custom = custom_prompt_ids.contains(&s.id.0);
            SessionInfo {
                id: s.id.0.clone(),
                agent_id: s.agent_id.as_ref().map(|id| id.0.clone()),
                title: s.title.clone(),
                manual_title: s.manual_title.clone(),
                created_at: s.created_at.to_rfc3339(),
                updated_at: s.updated_at.to_rfc3339(),
                message_count: s.message_count as usize,
                has_custom_prompt: has_custom,
            }
        })
        .collect();

    // Sort by updated_at descending (newest first).
    infos.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::is_user_visible_session;
    use y_core::session::{SessionState, SessionType};

    #[test]
    fn test_user_visible_session_requires_active_state() {
        assert!(is_user_visible_session(
            SessionType::Main,
            SessionState::Active
        ));
        assert!(is_user_visible_session(
            SessionType::Branch,
            SessionState::Active
        ));

        assert!(!is_user_visible_session(
            SessionType::Main,
            SessionState::Archived
        ));
        assert!(!is_user_visible_session(
            SessionType::Main,
            SessionState::Tombstone
        ));
        assert!(!is_user_visible_session(
            SessionType::SubAgent,
            SessionState::Active
        ));
    }
}

/// Create a new session.
#[tauri::command]
pub async fn session_create(
    state: State<'_, AppState>,
    title: Option<String>,
    agent_id: Option<String>,
) -> Result<SessionInfo, String> {
    let session = state
        .container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: agent_id.map(y_core::types::AgentId::from_string),
            title,
        })
        .await
        .map_err(|e| format!("Failed to create session: {e}"))?;

    Ok(SessionInfo {
        id: session.id.0.clone(),
        agent_id: session.agent_id.as_ref().map(|id| id.0.clone()),
        title: session.title.clone(),
        manual_title: session.manual_title.clone(),
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        message_count: 0,
        has_custom_prompt: false,
    })
}

/// Get all messages in a session.
#[tauri::command]
pub async fn session_get_messages(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<MessageInfo>, String> {
    let sid = SessionId(session_id);

    let messages = state
        .container
        .session_manager
        .read_display_transcript(&sid)
        .await
        .map_err(|e| format!("Failed to read display transcript: {e}"))?;

    Ok(messages
        .iter()
        .map(|m| {
            // Extract skills from metadata if present.
            let skills = m
                .metadata
                .get("skills")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<String>>()
                })
                .filter(|v| !v.is_empty());

            MessageInfo {
                id: m.message_id.clone(),
                role: format!("{:?}", m.role).to_lowercase(),
                content: m.content.clone(),
                timestamp: m.timestamp.to_rfc3339(),
                tool_calls: m
                    .tool_calls
                    .iter()
                    .map(|tc| ToolCallBrief {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.to_string(),
                    })
                    .collect(),
                metadata: m.metadata.clone(),
                skills,
            }
        })
        .collect())
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
