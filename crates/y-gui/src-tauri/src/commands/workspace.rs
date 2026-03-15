//! Workspace command handlers -- CRUD for folder-backed workspaces.
//!
//! Workspaces are GUI-level metadata stored in `~/.config/y-agent/workspaces.toml`.
//! Each workspace references a directory on the local filesystem and groups sessions
//! by a simple session-to-workspace mapping in `session_workspaces.toml`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Persistent data structures
// ---------------------------------------------------------------------------

/// A single workspace record (persisted in workspaces.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// The top-level TOML document for workspaces.toml.
#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspacesFile {
    #[serde(default)]
    workspaces: Vec<WorkspaceRecord>,
}

/// The top-level TOML document for session_workspaces.toml.
/// Maps session_id -> workspace_id.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionWorkspacesFile {
    #[serde(default)]
    assignments: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// File I/O helpers
// ---------------------------------------------------------------------------

fn workspaces_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("workspaces.toml")
}

fn session_workspaces_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("session_workspaces.toml")
}

fn read_workspaces(config_dir: &std::path::Path) -> WorkspacesFile {
    let path = workspaces_path(config_dir);
    if !path.exists() {
        return WorkspacesFile::default();
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_default()
}

fn write_workspaces(config_dir: &std::path::Path, data: &WorkspacesFile) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let content = toml::to_string_pretty(data)?;
    std::fs::write(workspaces_path(config_dir), content)?;
    Ok(())
}

fn read_session_workspaces(config_dir: &std::path::Path) -> SessionWorkspacesFile {
    let path = session_workspaces_path(config_dir);
    if !path.exists() {
        return SessionWorkspacesFile::default();
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_default()
}

fn write_session_workspaces(
    config_dir: &std::path::Path,
    data: &SessionWorkspacesFile,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let content = toml::to_string_pretty(data)?;
    std::fs::write(session_workspaces_path(config_dir), content)?;
    Ok(())
}

/// Resolve the workspace path for a given session.
///
/// Returns `Some(path)` if the session is assigned to a workspace,
/// `None` otherwise.
pub(crate) fn resolve_workspace_path(config_dir: &std::path::Path, session_id: &str) -> Option<String> {
    let sw = read_session_workspaces(config_dir);
    let ws_id = sw.assignments.get(session_id)?;
    let ws_data = read_workspaces(config_dir);
    ws_data
        .workspaces
        .iter()
        .find(|w| w.id == *ws_id)
        .map(|w| w.path.clone())
}

// ---------------------------------------------------------------------------
// Workspace CRUD commands
// ---------------------------------------------------------------------------

/// List all workspaces.
#[tauri::command]
pub async fn workspace_list(state: State<'_, AppState>) -> Result<Vec<WorkspaceRecord>, String> {
    let data = read_workspaces(&state.config_dir);
    Ok(data.workspaces)
}

/// Create a new workspace.
#[tauri::command]
pub async fn workspace_create(
    state: State<'_, AppState>,
    name: String,
    path: String,
) -> Result<WorkspaceRecord, String> {
    let record = WorkspaceRecord {
        id: Uuid::new_v4().to_string(),
        name,
        path,
    };
    let mut data = read_workspaces(&state.config_dir);
    data.workspaces.push(record.clone());
    write_workspaces(&state.config_dir, &data)
        .map_err(|e| format!("Failed to save workspaces: {e}"))?;
    Ok(record)
}

/// Update an existing workspace's name and/or path.
#[tauri::command]
pub async fn workspace_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    path: String,
) -> Result<(), String> {
    let mut data = read_workspaces(&state.config_dir);
    let entry = data
        .workspaces
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or_else(|| format!("Workspace not found: {id}"))?;
    entry.name = name;
    entry.path = path;
    write_workspaces(&state.config_dir, &data)
        .map_err(|e| format!("Failed to save workspaces: {e}"))?;
    Ok(())
}

/// Delete a workspace and remove all its session assignments.
#[tauri::command]
pub async fn workspace_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut data = read_workspaces(&state.config_dir);
    data.workspaces.retain(|w| w.id != id);
    write_workspaces(&state.config_dir, &data)
        .map_err(|e| format!("Failed to save workspaces: {e}"))?;

    // Remove all session assignments for the deleted workspace.
    let mut sw = read_session_workspaces(&state.config_dir);
    sw.assignments.retain(|_, wid| wid != &id);
    write_session_workspaces(&state.config_dir, &sw)
        .map_err(|e| format!("Failed to save session assignments: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Session-workspace assignment commands
// ---------------------------------------------------------------------------

/// Return the full session_id -> workspace_id map.
#[tauri::command]
pub async fn workspace_session_map(
    state: State<'_, AppState>,
) -> Result<HashMap<String, String>, String> {
    let sw = read_session_workspaces(&state.config_dir);
    Ok(sw.assignments)
}

/// Assign a session to a workspace (overwrites any previous assignment).
#[tauri::command]
pub async fn workspace_assign_session(
    state: State<'_, AppState>,
    workspace_id: String,
    session_id: String,
) -> Result<(), String> {
    let mut sw = read_session_workspaces(&state.config_dir);
    sw.assignments.insert(session_id, workspace_id);
    write_session_workspaces(&state.config_dir, &sw)
        .map_err(|e| format!("Failed to save session assignments: {e}"))?;
    Ok(())
}

/// Remove a session's workspace assignment.
#[tauri::command]
pub async fn workspace_unassign_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let mut sw = read_session_workspaces(&state.config_dir);
    sw.assignments.remove(&session_id);
    write_session_workspaces(&state.config_dir, &sw)
        .map_err(|e| format!("Failed to save session assignments: {e}"))?;
    Ok(())
}
