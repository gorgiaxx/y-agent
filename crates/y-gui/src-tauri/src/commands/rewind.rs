//! Tauri commands for the rewind feature.
//!
//! Exposes file history listing and rewind execution to the GUI.

use tauri::State;

use crate::state::AppState;

/// List available rewind points for a session.
///
/// Returns points in reverse chronological order (most recent first),
/// each containing message preview, diff stats, and timestamp.
#[tauri::command]
pub async fn rewind_list_points(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<y_service::RewindPointInfo>, String> {
    let session_id = y_core::types::SessionId(session_id);

    y_service::RewindService::list_rewind_points(&state.container, &session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Execute a rewind to a specific message boundary.
///
/// Performs three-phase rollback:
/// 1. Truncate conversation transcripts to the target message
/// 2. Restore files to their state at that message boundary
/// 3. Invalidate checkpoints after the target
///
/// Returns a `RewindResult` with details about what was restored/deleted.
#[tauri::command]
pub async fn rewind_execute(
    state: State<'_, AppState>,
    session_id: String,
    target_message_id: String,
) -> Result<y_service::RewindResult, String> {
    let session_id = y_core::types::SessionId(session_id);

    y_service::RewindService::execute_rewind(&state.container, &session_id, &target_message_id)
        .await
        .map_err(|e| e.to_string())
}

/// Restore files to a message boundary without truncating transcripts.
///
/// Used by the GUI undo flow where `chat_undo` already handles transcript
/// and checkpoint rollback. This only performs file restoration.
#[tauri::command]
pub async fn rewind_restore_files(
    state: State<'_, AppState>,
    session_id: String,
    target_message_id: String,
) -> Result<(), String> {
    let session_id = y_core::types::SessionId(session_id);

    // File restoration is best-effort: if no file history exists for this
    // session (e.g. the session never made file changes), we silently succeed.
    match y_service::RewindService::restore_files_only(
        &state.container,
        &session_id,
        &target_message_id,
    )
    .await
    {
        Ok(_) | Err(y_service::RewindError::NoHistory(_)) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
