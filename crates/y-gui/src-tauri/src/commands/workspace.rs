//! Workspace command handlers -- thin delegation to [`y_service::WorkspaceService`].
//!
//! All business logic (CRUD, session mapping, TOML persistence) lives in
//! `y-service`. These handlers convert Tauri arguments to service calls.

use std::collections::HashMap;

use tauri::State;

use y_service::{WorkspaceRecord, WorkspaceService};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Build a [`WorkspaceService`] from the application state.
fn svc(state: &AppState) -> WorkspaceService {
    WorkspaceService::new(&state.config_dir)
}

/// Resolve the workspace path for a given session.
///
/// Returns `Some(path)` if the session is assigned to a workspace,
/// `None` otherwise.
pub(crate) fn resolve_workspace_path(
    config_dir: &std::path::Path,
    session_id: &str,
) -> Option<String> {
    WorkspaceService::new(config_dir).resolve_workspace_path(session_id)
}

// ---------------------------------------------------------------------------
// Workspace CRUD commands
// ---------------------------------------------------------------------------

/// List all workspaces.
#[tauri::command]
pub async fn workspace_list(state: State<'_, AppState>) -> Result<Vec<WorkspaceRecord>, String> {
    Ok(svc(&state).list())
}

/// Create a new workspace.
#[tauri::command]
pub async fn workspace_create(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> Result<WorkspaceRecord, String> {
    svc(&state)
        .create(name, path)
        .map_err(|e| format!("Failed to create workspace: {e}"))
}

/// Update an existing workspace's name and/or path.
#[tauri::command]
pub async fn workspace_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    path: String,
) -> Result<(), String> {
    svc(&state)
        .update(&id, name, path)
        .map_err(|e| format!("Failed to update workspace: {e}"))
}

/// Delete a workspace and remove all its session assignments.
#[tauri::command]
pub async fn workspace_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    svc(&state)
        .delete(&id)
        .map_err(|e| format!("Failed to delete workspace: {e}"))
}

// ---------------------------------------------------------------------------
// Session-workspace assignment commands
// ---------------------------------------------------------------------------

/// Return the full `session_id` -> `workspace_id` map.
#[tauri::command]
pub async fn workspace_session_map(
    state: State<'_, AppState>,
) -> Result<HashMap<String, String>, String> {
    Ok(svc(&state).session_map())
}

/// Assign a session to a workspace (overwrites any previous assignment).
#[tauri::command]
pub async fn workspace_assign_session(
    state: State<'_, AppState>,
    workspace_id: String,
    session_id: String,
) -> Result<(), String> {
    svc(&state)
        .assign_session(workspace_id, session_id)
        .map_err(|e| format!("Failed to assign session: {e}"))
}

/// Remove a session's workspace assignment.
#[tauri::command]
pub async fn workspace_unassign_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    svc(&state)
        .unassign_session(&session_id)
        .map_err(|e| format!("Failed to unassign session: {e}"))
}
