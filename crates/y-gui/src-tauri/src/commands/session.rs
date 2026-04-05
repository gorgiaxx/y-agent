//! Session command handlers — list, create, get messages, delete, truncate.

use serde::Serialize;
use tauri::State;

use y_core::session::{CreateSessionOptions, SessionFilter, SessionType};
use y_core::types::SessionId;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Session info returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
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
pub async fn session_list(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let filter = SessionFilter::default();
    let sessions = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map_err(|e| format!("Failed to list sessions: {e}"))?;

    let mut infos: Vec<SessionInfo> = sessions
        .into_iter()
        .map(|s| SessionInfo {
            id: s.id.0.clone(),
            title: s.title.clone(),
            created_at: s.created_at.to_rfc3339(),
            updated_at: s.updated_at.to_rfc3339(),
            message_count: s.message_count as usize,
        })
        .collect();

    // Sort by updated_at descending (newest first).
    infos.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(infos)
}

/// Create a new session.
#[tauri::command]
pub async fn session_create(
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<SessionInfo, String> {
    let session = state
        .container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title,
        })
        .await
        .map_err(|e| format!("Failed to create session: {e}"))?;

    Ok(SessionInfo {
        id: session.id.0.clone(),
        title: session.title.clone(),
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        message_count: 0,
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

/// Hard-delete a session from the database.
///
/// This permanently removes the session metadata and clears its transcript.
/// Any in-progress runs for this session should have completed before calling this.
#[tauri::command]
pub async fn session_delete(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .delete_session(&sid)
        .await
        .map_err(|e| format!("Failed to delete session: {e}"))?;
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
        title: fork.title.clone(),
        created_at: fork.created_at.to_rfc3339(),
        updated_at: fork.updated_at.to_rfc3339(),
        message_count: fork.message_count as usize,
    })
}
