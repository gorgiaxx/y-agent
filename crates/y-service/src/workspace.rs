//! Workspace service -- CRUD, session mapping, and project-configuration trust.
//!
//! Workspaces are metadata stored in `workspaces.toml` and
//! `session_workspaces.toml` inside the config directory. Explicit trust
//! decisions use canonical paths and are stored in `workspace_trust.toml`.
//! Each workspace
//! references a directory on the local filesystem and groups sessions via a
//! simple session-to-workspace mapping.

use std::collections::HashMap;
use std::io::Write;
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

/// Persisted trust state for one canonical workspace path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTrustStatus {
    /// No explicit decision exists for this canonical path.
    Unknown,
    /// Project-origin configuration may be activated.
    Trusted,
    /// Project-origin configuration must remain blocked.
    Untrusted,
}

/// Result of resolving trust for a workspace path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceTrustDecision {
    /// Canonical filesystem identity used by the trust store.
    pub canonical_path: String,
    /// Explicit or default trust state.
    pub status: WorkspaceTrustStatus,
    /// Last mutation timestamp for explicit decisions.
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceTrustRecord {
    canonical_path: String,
    status: WorkspaceTrustStatus,
    updated_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspaceTrustFile {
    #[serde(default)]
    workspaces: Vec<WorkspaceTrustRecord>,
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

    // -- Workspace trust -------------------------------------------------

    /// Resolve trust for a workspace using its canonical filesystem path.
    pub fn workspace_trust(&self, path: &Path) -> anyhow::Result<WorkspaceTrustDecision> {
        let canonical_path = canonical_workspace_path(path)?;
        self.ensure_trust_store_outside_workspace(&canonical_path)?;
        let data = self.read_workspace_trust_result()?;
        let record = data
            .workspaces
            .iter()
            .find(|record| record.canonical_path == canonical_path);

        Ok(WorkspaceTrustDecision {
            canonical_path,
            status: record.map_or(WorkspaceTrustStatus::Unknown, |record| record.status),
            updated_at: record.map(|record| record.updated_at.clone()),
        })
    }

    /// Explicitly trust project-origin configuration from a workspace.
    pub fn trust_workspace(&self, path: &Path) -> anyhow::Result<WorkspaceTrustDecision> {
        self.set_workspace_trust(path, WorkspaceTrustStatus::Trusted)
    }

    /// Explicitly block project-origin configuration from a workspace.
    pub fn untrust_workspace(&self, path: &Path) -> anyhow::Result<WorkspaceTrustDecision> {
        self.set_workspace_trust(path, WorkspaceTrustStatus::Untrusted)
    }

    fn set_workspace_trust(
        &self,
        path: &Path,
        status: WorkspaceTrustStatus,
    ) -> anyhow::Result<WorkspaceTrustDecision> {
        let canonical_path = canonical_workspace_path(path)?;
        std::fs::create_dir_all(&self.config_dir)?;
        self.ensure_trust_store_outside_workspace(&canonical_path)?;
        let mut data = self.read_workspace_trust_result()?;
        let updated_at = chrono::Utc::now().to_rfc3339();
        data.workspaces
            .retain(|record| record.canonical_path != canonical_path);
        data.workspaces.push(WorkspaceTrustRecord {
            canonical_path: canonical_path.clone(),
            status,
            updated_at: updated_at.clone(),
        });
        data.workspaces
            .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
        self.write_workspace_trust(&data)?;
        tracing::info!(workspace = %canonical_path, ?status, "workspace trust decision updated");

        Ok(WorkspaceTrustDecision {
            canonical_path,
            status,
            updated_at: Some(updated_at),
        })
    }

    fn ensure_trust_store_outside_workspace(
        &self,
        canonical_workspace: &str,
    ) -> anyhow::Result<()> {
        if !self.config_dir.exists() {
            return Ok(());
        }
        let canonical_config_dir = std::fs::canonicalize(&self.config_dir).map_err(|error| {
            anyhow::anyhow!(
                "failed to canonicalize trust store directory {}: {error}",
                self.config_dir.display()
            )
        })?;
        if canonical_config_dir.starts_with(Path::new(canonical_workspace)) {
            anyhow::bail!(
                "workspace trust store must be outside the workspace: {}",
                canonical_config_dir.display()
            );
        }
        Ok(())
    }

    // -- Private file I/O helpers -----------------------------------------

    fn workspaces_path(&self) -> PathBuf {
        self.config_dir.join("workspaces.toml")
    }

    fn session_workspaces_path(&self) -> PathBuf {
        self.config_dir.join("session_workspaces.toml")
    }

    fn workspace_trust_path(&self) -> PathBuf {
        self.config_dir.join("workspace_trust.toml")
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

    fn read_workspace_trust_result(&self) -> anyhow::Result<WorkspaceTrustFile> {
        let path = self.workspace_trust_path();
        if !path.exists() {
            return Ok(WorkspaceTrustFile::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        toml::from_str(&content)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))
    }

    fn write_workspace_trust(&self, data: &WorkspaceTrustFile) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        let content = toml::to_string_pretty(data)?;
        let path = self.workspace_trust_path();
        let mut temporary = tempfile::NamedTempFile::new_in(&self.config_dir)?;
        temporary.write_all(content.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|error| {
            anyhow::anyhow!("failed to persist {}: {}", path.display(), error.error)
        })?;
        Ok(())
    }
}

fn canonical_workspace_path(path: &Path) -> anyhow::Result<String> {
    if !path.is_dir() {
        anyhow::bail!(
            "workspace path is not an existing directory: {}",
            path.display()
        );
    }
    std::fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| anyhow::anyhow!("failed to canonicalize {}: {error}", path.display()))
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

    #[test]
    fn workspace_trust_is_explicit_persistent_and_path_bound() {
        let (svc, dir) = make_service();
        let workspace = dir.path().join("project");
        let moved_workspace = dir.path().join("project-moved");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&moved_workspace).unwrap();

        assert_eq!(
            svc.workspace_trust(&workspace).unwrap().status,
            WorkspaceTrustStatus::Unknown
        );

        svc.trust_workspace(&workspace).unwrap();
        let reloaded = WorkspaceService::new(dir.path());
        assert_eq!(
            reloaded.workspace_trust(&workspace).unwrap().status,
            WorkspaceTrustStatus::Trusted
        );
        assert_eq!(
            reloaded.workspace_trust(&moved_workspace).unwrap().status,
            WorkspaceTrustStatus::Unknown
        );

        reloaded.untrust_workspace(&workspace).unwrap();
        assert_eq!(
            reloaded.workspace_trust(&workspace).unwrap().status,
            WorkspaceTrustStatus::Untrusted
        );
    }

    #[test]
    fn corrupt_workspace_trust_store_fails_closed_without_overwrite() {
        let (svc, dir) = make_service();
        let workspace = dir.path().join("project");
        std::fs::create_dir_all(&workspace).unwrap();
        let trust_path = dir.path().join("workspace_trust.toml");
        std::fs::write(&trust_path, "invalid = [").unwrap();
        let original = std::fs::read_to_string(&trust_path).unwrap();

        assert!(svc.workspace_trust(&workspace).is_err());
        assert_eq!(std::fs::read_to_string(&trust_path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_trust_uses_canonical_path_for_symlinks() {
        use std::os::unix::fs::symlink;

        let (svc, dir) = make_service();
        let workspace = dir.path().join("project");
        let alias = dir.path().join("project-alias");
        std::fs::create_dir_all(&workspace).unwrap();
        symlink(&workspace, &alias).unwrap();

        svc.trust_workspace(&alias).unwrap();

        let direct = svc.workspace_trust(&workspace).unwrap();
        let through_alias = svc.workspace_trust(&alias).unwrap();
        assert_eq!(direct.status, WorkspaceTrustStatus::Trusted);
        assert_eq!(direct.canonical_path, through_alias.canonical_path);
    }

    #[test]
    fn workspace_cannot_store_its_own_trust_grant() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("project");
        let project_owned_config = workspace.join(".y-agent-user");
        std::fs::create_dir_all(&project_owned_config).unwrap();
        let service = WorkspaceService::new(&project_owned_config);

        let result = service.trust_workspace(&workspace);

        assert!(result.is_err());
        assert!(!project_owned_config.join("workspace_trust.toml").exists());
    }
}
