//! Workspace service -- CRUD for folder-backed workspaces and
//! session-to-workspace mapping.
//!
//! Workspaces are metadata stored in `workspaces.toml` and
//! `session_workspaces.toml` inside the config directory. Each workspace
//! references a directory on the local filesystem and groups sessions via a
//! simple session-to-workspace mapping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Persistent data structures
// ---------------------------------------------------------------------------

/// A single workspace record (persisted in `workspaces.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// The top-level TOML document for `workspaces.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspacesFile {
    #[serde(default)]
    workspaces: Vec<WorkspaceRecord>,
}

/// The top-level TOML document for `session_workspaces.toml`.
/// Maps `session_id` -> `workspace_id`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionWorkspacesFile {
    #[serde(default)]
    assignments: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// WorkspaceService
// ---------------------------------------------------------------------------

/// Service for managing workspaces and session-workspace assignments.
///
/// All operations are stateless file I/O against TOML files in the
/// config directory. This struct is cheap to construct.
pub struct WorkspaceService {
    config_dir: PathBuf,
}

impl WorkspaceService {
    /// Create a new `WorkspaceService` rooted at the given config directory.
    pub fn new(config_dir: &Path) -> Self {
        Self {
            config_dir: config_dir.to_path_buf(),
        }
    }

    // -- Workspace CRUD ---------------------------------------------------

    /// List all workspaces.
    pub fn list(&self) -> Vec<WorkspaceRecord> {
        self.read_workspaces().workspaces
    }

    /// Create a new workspace. Returns the created record.
    pub fn create(&self, name: String, path: String) -> anyhow::Result<WorkspaceRecord> {
        let record = WorkspaceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            path,
        };
        let mut data = self.read_workspaces_result()?;
        data.workspaces.push(record.clone());
        self.write_workspaces(&data)?;
        Ok(record)
    }

    /// Update an existing workspace's name and/or path.
    pub fn update(&self, id: &str, name: String, path: String) -> anyhow::Result<()> {
        let mut data = self.read_workspaces_result()?;
        let entry = data
            .workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or_else(|| anyhow::anyhow!("Workspace not found: {id}"))?;
        entry.name = name;
        entry.path = path;
        self.write_workspaces(&data)
    }

    /// Delete a workspace and remove all its session assignments.
    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        let mut data = self.read_workspaces_result()?;
        data.workspaces.retain(|w| w.id != id);
        self.write_workspaces(&data)?;

        // Remove all session assignments for the deleted workspace.
        let mut sw = self.read_session_workspaces_result()?;
        sw.assignments.retain(|_, wid| wid != id);
        self.write_session_workspaces(&sw)
    }

    // -- Session-workspace mapping ----------------------------------------

    /// Return the full `session_id` -> `workspace_id` map.
    pub fn session_map(&self) -> HashMap<String, String> {
        self.read_session_workspaces().assignments
    }

    /// Assign a session to a workspace (overwrites any previous assignment).
    pub fn assign_session(&self, workspace_id: String, session_id: String) -> anyhow::Result<()> {
        let mut sw = self.read_session_workspaces_result()?;
        sw.assignments.insert(session_id, workspace_id);
        self.write_session_workspaces(&sw)
    }

    /// Remove a session's workspace assignment.
    pub fn unassign_session(&self, session_id: &str) -> anyhow::Result<()> {
        let mut sw = self.read_session_workspaces_result()?;
        sw.assignments.remove(session_id);
        self.write_session_workspaces(&sw)
    }

    /// Resolve the workspace path for a given session.
    ///
    /// Returns `Some(path)` if the session is assigned to a workspace,
    /// `None` otherwise.
    pub fn resolve_workspace_path(&self, session_id: &str) -> Option<String> {
        let sw = self.read_session_workspaces();
        let ws_id = sw.assignments.get(session_id)?;
        let ws_data = self.read_workspaces();
        ws_data
            .workspaces
            .iter()
            .find(|w| w.id == *ws_id)
            .map(|w| w.path.clone())
    }

    // -- Private file I/O helpers -----------------------------------------

    fn workspaces_path(&self) -> PathBuf {
        self.config_dir.join("workspaces.toml")
    }

    fn session_workspaces_path(&self) -> PathBuf {
        self.config_dir.join("session_workspaces.toml")
    }

    fn read_workspaces(&self) -> WorkspacesFile {
        self.read_workspaces_result().unwrap_or_else(|error| {
            tracing::warn!(error = %error, "failed to load workspaces; using empty set");
            WorkspacesFile::default()
        })
    }

    fn read_workspaces_result(&self) -> anyhow::Result<WorkspacesFile> {
        let path = self.workspaces_path();
        if !path.exists() {
            return Ok(WorkspacesFile::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        toml::from_str(&content)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))
    }

    fn write_workspaces(&self, data: &WorkspacesFile) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        let content = toml::to_string_pretty(data)?;
        std::fs::write(self.workspaces_path(), content)?;
        Ok(())
    }

    fn read_session_workspaces(&self) -> SessionWorkspacesFile {
        self.read_session_workspaces_result()
            .unwrap_or_else(|error| {
                tracing::warn!(
                    error = %error,
                    "failed to load session workspace assignments; using empty set"
                );
                SessionWorkspacesFile::default()
            })
    }

    fn read_session_workspaces_result(&self) -> anyhow::Result<SessionWorkspacesFile> {
        let path = self.session_workspaces_path();
        if !path.exists() {
            return Ok(SessionWorkspacesFile::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        toml::from_str(&content)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))
    }

    fn write_session_workspaces(&self, data: &SessionWorkspacesFile) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        let content = toml::to_string_pretty(data)?;
        std::fs::write(self.session_workspaces_path(), content)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_service() -> (WorkspaceService, TempDir) {
        let dir = TempDir::new().unwrap();
        let svc = WorkspaceService::new(dir.path());
        (svc, dir)
    }

    #[test]
    fn test_workspace_crud() {
        let (svc, _dir) = make_service();

        // Initially empty.
        assert!(svc.list().is_empty());

        // Create.
        let ws = svc.create("My Project".into(), "/tmp/proj".into()).unwrap();
        assert_eq!(ws.name, "My Project");
        assert_eq!(ws.path, "/tmp/proj");

        // List.
        let all = svc.list();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, ws.id);

        // Update.
        svc.update(&ws.id, "Renamed".into(), "/tmp/other".into())
            .unwrap();
        let all = svc.list();
        assert_eq!(all[0].name, "Renamed");
        assert_eq!(all[0].path, "/tmp/other");

        // Delete.
        svc.delete(&ws.id).unwrap();
        assert!(svc.list().is_empty());
    }

    #[test]
    fn test_session_assignment() {
        let (svc, _dir) = make_service();
        let ws = svc.create("WS".into(), "/tmp/ws".into()).unwrap();

        // Assign.
        svc.assign_session(ws.id.clone(), "session-1".into())
            .unwrap();
        assert_eq!(svc.session_map().len(), 1);

        // Resolve.
        assert_eq!(
            svc.resolve_workspace_path("session-1"),
            Some("/tmp/ws".into())
        );
        assert_eq!(svc.resolve_workspace_path("session-2"), None);

        // Unassign.
        svc.unassign_session("session-1").unwrap();
        assert!(svc.session_map().is_empty());
        assert_eq!(svc.resolve_workspace_path("session-1"), None);
    }

    #[test]
    fn test_delete_cascades_session_assignments() {
        let (svc, _dir) = make_service();
        let ws = svc.create("WS".into(), "/tmp/ws".into()).unwrap();
        svc.assign_session(ws.id.clone(), "s1".into()).unwrap();
        svc.assign_session(ws.id.clone(), "s2".into()).unwrap();

        svc.delete(&ws.id).unwrap();

        // Both assignments should be gone.
        assert!(svc.session_map().is_empty());
    }

    #[test]
    fn test_update_nonexistent_returns_error() {
        let (svc, _dir) = make_service();
        let result = svc.update("nonexistent", "N".into(), "/p".into());
        assert!(result.is_err());
    }

    #[test]
    fn test_create_fails_when_workspaces_file_is_invalid() {
        let (svc, dir) = make_service();
        let path = dir.path().join("workspaces.toml");
        std::fs::write(&path, "invalid = [").unwrap();
        let original = std::fs::read_to_string(&path).unwrap();

        let result = svc.create("Broken".into(), "/tmp/broken".into());

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn test_assign_session_fails_when_assignment_file_is_invalid() {
        let (svc, dir) = make_service();
        let ws = svc.create("WS".into(), "/tmp/ws".into()).unwrap();
        let path = dir.path().join("session_workspaces.toml");
        std::fs::write(&path, "assignments = { broken").unwrap();
        let original = std::fs::read_to_string(&path).unwrap();

        let result = svc.assign_session(ws.id, "session-1".into());

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
