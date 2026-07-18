//! Git worktree provisioning for isolated sub-agent execution.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("workspace is not inside a Git repository: {path}")]
    NotGitRepository { path: String },
    #[error("workspace path is outside repository root: {path}")]
    WorkspaceOutsideRepository { path: String },
    #[error("workspace has uncommitted changes and cannot be isolated from HEAD: {path}")]
    DirtyWorkingTree { path: String },
    #[error("invalid worktree id '{id}'")]
    InvalidWorktreeId { id: String },
    #[error("worktree path already exists: {path}")]
    AlreadyExists { path: String },
    #[error("worktree snapshot already exists: {id}")]
    SnapshotAlreadyExists { id: String },
    #[error("worktree snapshot not found: {id}")]
    SnapshotNotFound { id: String },
    #[error("worktree snapshot '{id}' belongs to a different repository")]
    SnapshotRepositoryMismatch { id: String },
    #[error("worktree snapshot '{id}' is invalid: {message}")]
    InvalidSnapshot { id: String, message: String },
    #[error("git {operation} failed: {stderr}")]
    GitCommandFailed { operation: String, stderr: String },
    #[error("filesystem operation failed for {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub struct WorktreeLease {
    id: String,
    repository_root: PathBuf,
    worktree_path: PathBuf,
    working_directory: PathBuf,
    base_revision: String,
}

impl WorktreeLease {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn worktree_path(&self) -> &Path {
        &self.worktree_path
    }

    pub fn working_directory(&self) -> PathBuf {
        self.working_directory.clone()
    }

    pub fn base_revision(&self) -> &str {
        &self.base_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeDiff {
    pub changed_files: Vec<String>,
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeSnapshot {
    pub id: String,
    pub base_revision: String,
    pub changed_files: Vec<String>,
    pub patch: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WorktreeSnapshotRecord {
    id: String,
    repository_root: String,
    relative_working_directory: String,
    base_revision: String,
    changed_files: Vec<String>,
    patch: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct WorktreeManager {
    isolation_root: PathBuf,
    git_lock: Arc<Mutex<()>>,
}

impl WorktreeManager {
    pub fn new(isolation_root: PathBuf) -> Self {
        Self {
            isolation_root,
            git_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn create(
        &self,
        working_directory: &Path,
        worktree_id: &str,
    ) -> Result<WorktreeLease, WorktreeError> {
        validate_worktree_id(worktree_id)?;
        let working_directory =
            tokio::fs::canonicalize(working_directory)
                .await
                .map_err(|source| WorktreeError::Io {
                    path: working_directory.display().to_string(),
                    source,
                })?;

        let repository_root = discover_repository_root(&working_directory).await?;
        let relative_directory =
            working_directory
                .strip_prefix(&repository_root)
                .map_err(|_| WorktreeError::WorkspaceOutsideRepository {
                    path: working_directory.display().to_string(),
                })?;
        ensure_clean_worktree(&repository_root).await?;
        let base_revision = run_git(&repository_root, ["rev-parse", "HEAD"], "rev-parse HEAD")
            .await?
            .trim()
            .to_string();

        self.create_at_revision(
            repository_root,
            relative_directory.to_path_buf(),
            worktree_id,
            base_revision,
        )
        .await
    }

    async fn create_at_revision(
        &self,
        repository_root: PathBuf,
        relative_directory: PathBuf,
        worktree_id: &str,
        base_revision: String,
    ) -> Result<WorktreeLease, WorktreeError> {
        validate_relative_directory(&relative_directory, worktree_id)?;

        tokio::fs::create_dir_all(&self.isolation_root)
            .await
            .map_err(|source| WorktreeError::Io {
                path: self.isolation_root.display().to_string(),
                source,
            })?;
        let isolation_root = tokio::fs::canonicalize(&self.isolation_root)
            .await
            .map_err(|source| WorktreeError::Io {
                path: self.isolation_root.display().to_string(),
                source,
            })?;
        let worktree_path = isolation_root.join(worktree_id);
        if worktree_path.exists() {
            return Err(WorktreeError::AlreadyExists {
                path: worktree_path.display().to_string(),
            });
        }

        let _git_guard = self.git_lock.lock().await;
        run_git(
            &repository_root,
            [
                OsString::from("worktree"),
                OsString::from("add"),
                OsString::from("--detach"),
                worktree_path.as_os_str().to_os_string(),
                OsString::from(&base_revision),
            ],
            "worktree add",
        )
        .await?;

        let canonical_worktree = match tokio::fs::canonicalize(&worktree_path).await {
            Ok(path) => path,
            Err(source) => {
                cleanup_created_worktree(&repository_root, &worktree_path).await;
                return Err(WorktreeError::Io {
                    path: worktree_path.display().to_string(),
                    source,
                });
            }
        };
        if !canonical_worktree.starts_with(&isolation_root) {
            cleanup_created_worktree(&repository_root, &canonical_worktree).await;
            return Err(WorktreeError::WorkspaceOutsideRepository {
                path: canonical_worktree.display().to_string(),
            });
        }

        let isolated_working_directory = canonical_worktree.join(relative_directory);
        if let Err(source) = tokio::fs::create_dir_all(&isolated_working_directory).await {
            cleanup_created_worktree(&repository_root, &canonical_worktree).await;
            return Err(WorktreeError::Io {
                path: isolated_working_directory.display().to_string(),
                source,
            });
        }

        Ok(WorktreeLease {
            id: worktree_id.to_string(),
            repository_root,
            working_directory: isolated_working_directory,
            worktree_path: canonical_worktree,
            base_revision,
        })
    }

    pub async fn capture_diff(&self, lease: &WorktreeLease) -> Result<WorktreeDiff, WorktreeError> {
        run_git(
            lease.worktree_path(),
            ["add", "--intent-to-add", "--all"],
            "add --intent-to-add",
        )
        .await?;
        let patch = run_git(
            lease.worktree_path(),
            ["diff", "--binary", "--no-ext-diff", "HEAD"],
            "diff",
        )
        .await?;
        let changed = run_git(
            lease.worktree_path(),
            ["diff", "--name-only", "-z", "HEAD"],
            "diff --name-only",
        )
        .await?;
        let mut changed_files = changed
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        changed_files.sort();
        changed_files.dedup();
        Ok(WorktreeDiff {
            changed_files,
            patch,
        })
    }

    pub async fn snapshot(
        &self,
        lease: &WorktreeLease,
        snapshot_id: &str,
    ) -> Result<WorktreeSnapshot, WorktreeError> {
        validate_snapshot_id(snapshot_id)?;
        let snapshot_path = self.snapshot_path(snapshot_id);
        if snapshot_path.exists() {
            return Err(WorktreeError::SnapshotAlreadyExists {
                id: snapshot_id.to_string(),
            });
        }

        let diff = self.capture_diff(lease).await?;
        let relative_working_directory = lease
            .working_directory()
            .strip_prefix(lease.worktree_path())
            .map_err(|_| WorktreeError::InvalidSnapshot {
                id: snapshot_id.to_string(),
                message: "working directory is outside the worktree".to_string(),
            })?
            .to_string_lossy()
            .to_string();
        let record = WorktreeSnapshotRecord {
            id: snapshot_id.to_string(),
            repository_root: lease.repository_root().to_string_lossy().to_string(),
            relative_working_directory,
            base_revision: lease.base_revision().to_string(),
            changed_files: diff.changed_files.clone(),
            patch: diff.patch.clone(),
            created_at: chrono::Utc::now(),
        };
        let encoded =
            serde_json::to_vec_pretty(&record).map_err(|error| WorktreeError::InvalidSnapshot {
                id: snapshot_id.to_string(),
                message: error.to_string(),
            })?;

        let snapshots_directory = self.isolation_root.join("snapshots");
        tokio::fs::create_dir_all(&snapshots_directory)
            .await
            .map_err(|source| WorktreeError::Io {
                path: snapshots_directory.display().to_string(),
                source,
            })?;
        let temporary_path =
            snapshots_directory.join(format!(".{snapshot_id}.{}.tmp", uuid::Uuid::new_v4()));
        tokio::fs::write(&temporary_path, encoded)
            .await
            .map_err(|source| WorktreeError::Io {
                path: temporary_path.display().to_string(),
                source,
            })?;
        if let Err(source) = tokio::fs::rename(&temporary_path, &snapshot_path).await {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(WorktreeError::Io {
                path: snapshot_path.display().to_string(),
                source,
            });
        }

        Ok(WorktreeSnapshot {
            id: snapshot_id.to_string(),
            base_revision: record.base_revision,
            changed_files: record.changed_files,
            patch: diff.patch,
        })
    }

    pub async fn rehydrate(
        &self,
        working_directory: &Path,
        snapshot_id: &str,
        worktree_id: &str,
    ) -> Result<WorktreeLease, WorktreeError> {
        validate_snapshot_id(snapshot_id)?;
        validate_worktree_id(worktree_id)?;
        let working_directory =
            tokio::fs::canonicalize(working_directory)
                .await
                .map_err(|source| WorktreeError::Io {
                    path: working_directory.display().to_string(),
                    source,
                })?;
        let repository_root = discover_repository_root(&working_directory).await?;
        let record = self.load_snapshot(snapshot_id).await?;
        if Path::new(&record.repository_root) != repository_root {
            return Err(WorktreeError::SnapshotRepositoryMismatch {
                id: snapshot_id.to_string(),
            });
        }

        let relative_directory = PathBuf::from(&record.relative_working_directory);
        validate_relative_directory(&relative_directory, snapshot_id)?;
        let lease = self
            .create_at_revision(
                repository_root,
                relative_directory,
                worktree_id,
                record.base_revision,
            )
            .await?;
        if !record.patch.is_empty() {
            if let Err(error) = run_git_with_stdin(
                lease.worktree_path(),
                ["apply", "--binary", "--whitespace=nowarn", "-"],
                "apply snapshot",
                record.patch.as_bytes(),
            )
            .await
            {
                let _ = self.cleanup(lease).await;
                return Err(error);
            }
        }
        Ok(lease)
    }

    pub async fn cleanup(&self, lease: WorktreeLease) -> Result<(), WorktreeError> {
        let _git_guard = self.git_lock.lock().await;
        run_git(
            lease.repository_root(),
            [
                OsString::from("worktree"),
                OsString::from("remove"),
                OsString::from("--force"),
                lease.worktree_path().as_os_str().to_os_string(),
            ],
            "worktree remove",
        )
        .await?;
        run_git(
            lease.repository_root(),
            ["worktree", "prune"],
            "worktree prune",
        )
        .await?;
        Ok(())
    }

    async fn load_snapshot(
        &self,
        snapshot_id: &str,
    ) -> Result<WorktreeSnapshotRecord, WorktreeError> {
        let path = self.snapshot_path(snapshot_id);
        let encoded = match tokio::fs::read(&path).await {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(WorktreeError::SnapshotNotFound {
                    id: snapshot_id.to_string(),
                });
            }
            Err(source) => {
                return Err(WorktreeError::Io {
                    path: path.display().to_string(),
                    source,
                });
            }
        };
        let record: WorktreeSnapshotRecord =
            serde_json::from_slice(&encoded).map_err(|error| WorktreeError::InvalidSnapshot {
                id: snapshot_id.to_string(),
                message: error.to_string(),
            })?;
        if record.id != snapshot_id {
            return Err(WorktreeError::InvalidSnapshot {
                id: snapshot_id.to_string(),
                message: "snapshot id does not match its filename".to_string(),
            });
        }
        Ok(record)
    }

    fn snapshot_path(&self, snapshot_id: &str) -> PathBuf {
        self.isolation_root
            .join("snapshots")
            .join(format!("{snapshot_id}.json"))
    }
}

fn validate_worktree_id(worktree_id: &str) -> Result<(), WorktreeError> {
    if worktree_id.is_empty()
        || worktree_id.len() > 128
        || worktree_id == "snapshots"
        || !worktree_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(WorktreeError::InvalidWorktreeId {
            id: worktree_id.to_string(),
        });
    }
    Ok(())
}

fn validate_snapshot_id(snapshot_id: &str) -> Result<(), WorktreeError> {
    validate_worktree_id(snapshot_id)
}

fn validate_relative_directory(path: &Path, id: &str) -> Result<(), WorktreeError> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(WorktreeError::InvalidSnapshot {
            id: id.to_string(),
            message: "working directory must be relative and remain inside the worktree"
                .to_string(),
        });
    }
    Ok(())
}

async fn cleanup_created_worktree(repository_root: &Path, worktree_path: &Path) {
    let _ = run_git(
        repository_root,
        [
            OsString::from("worktree"),
            OsString::from("remove"),
            OsString::from("--force"),
            worktree_path.as_os_str().to_os_string(),
        ],
        "worktree remove after provisioning failure",
    )
    .await;
    let _ = run_git(
        repository_root,
        ["worktree", "prune"],
        "worktree prune after provisioning failure",
    )
    .await;
}

async fn discover_repository_root(working_directory: &Path) -> Result<PathBuf, WorktreeError> {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .arg("-C")
        .arg(working_directory)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .await
        .map_err(|source| WorktreeError::Io {
            path: working_directory.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(WorktreeError::NotGitRepository {
            path: working_directory.display().to_string(),
        });
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    tokio::fs::canonicalize(&root)
        .await
        .map_err(|source| WorktreeError::Io { path: root, source })
}

async fn ensure_clean_worktree(repository_root: &Path) -> Result<(), WorktreeError> {
    let status = run_git(
        repository_root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
        "status --porcelain",
    )
    .await?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(WorktreeError::DirtyWorkingTree {
            path: repository_root.display().to_string(),
        })
    }
}

async fn run_git<I, S>(
    working_directory: &Path,
    args: I,
    operation: &str,
) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .arg("-C")
        .arg(working_directory)
        .args(args)
        .output()
        .await
        .map_err(|source| WorktreeError::Io {
            path: working_directory.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(WorktreeError::GitCommandFailed {
            operation: operation.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn run_git_with_stdin<I, S>(
    working_directory: &Path,
    args: I,
    operation: &str,
    stdin: &[u8],
) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .arg("-C")
        .arg(working_directory)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| WorktreeError::Io {
            path: working_directory.display().to_string(),
            source,
        })?;
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| WorktreeError::InvalidSnapshot {
            id: operation.to_string(),
            message: "git stdin was unavailable".to_string(),
        })?;
    child_stdin
        .write_all(stdin)
        .await
        .map_err(|source| WorktreeError::Io {
            path: working_directory.display().to_string(),
            source,
        })?;
    drop(child_stdin);
    let output = child
        .wait_with_output()
        .await
        .map_err(|source| WorktreeError::Io {
            path: working_directory.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(WorktreeError::GitCommandFailed {
            operation: operation.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::{WorktreeError, WorktreeManager};

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn initialized_repo() -> TempDir {
        let repo = TempDir::new().unwrap();
        git(repo.path(), &["init", "--quiet"]);
        git(
            repo.path(),
            &["config", "user.email", "tests@y-agent.local"],
        );
        git(repo.path(), &["config", "user.name", "y-agent tests"]);
        std::fs::create_dir(repo.path().join("src")).unwrap();
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[tokio::test]
    async fn creates_detached_worktree_at_base_revision_and_preserves_subdirectory() {
        let repo = initialized_repo();
        let isolation_root = TempDir::new().unwrap();
        let manager = WorktreeManager::new(isolation_root.path().to_path_buf());
        let base_revision = git(repo.path(), &["rev-parse", "HEAD"]);
        let canonical_isolation_root = std::fs::canonicalize(isolation_root.path()).unwrap();

        let lease = manager
            .create(&repo.path().join("src"), "delegation-1")
            .await
            .unwrap();

        assert_eq!(lease.base_revision(), base_revision);
        assert!(lease.worktree_path().starts_with(canonical_isolation_root));
        assert_eq!(lease.working_directory(), lease.worktree_path().join("src"));
        assert_eq!(
            git(lease.worktree_path(), &["rev-parse", "HEAD"]),
            base_revision
        );
        assert_eq!(
            git(
                lease.worktree_path(),
                &["rev-parse", "--abbrev-ref", "HEAD"]
            ),
            "HEAD"
        );

        manager.cleanup(lease).await.unwrap();
    }

    #[tokio::test]
    async fn captures_tracked_and_untracked_changes_before_cleanup() {
        let repo = initialized_repo();
        let isolation_root = TempDir::new().unwrap();
        let manager = WorktreeManager::new(isolation_root.path().to_path_buf());
        let lease = manager.create(repo.path(), "delegation-2").await.unwrap();

        std::fs::write(
            lease.worktree_path().join("src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .unwrap();
        std::fs::write(lease.worktree_path().join("new.txt"), "new\n").unwrap();

        let diff = manager.capture_diff(&lease).await.unwrap();

        assert_eq!(diff.changed_files, vec!["new.txt", "src/lib.rs"]);
        assert!(diff.patch.contains("src/lib.rs"));
        assert!(diff.patch.contains("new.txt"));

        let worktree_path = lease.worktree_path().to_path_buf();
        manager.cleanup(lease).await.unwrap();
        assert!(!worktree_path.exists());
    }

    #[tokio::test]
    async fn snapshots_and_rehydrates_changes_after_worktree_cleanup() {
        let repo = initialized_repo();
        let isolation_root = TempDir::new().unwrap();
        let manager = WorktreeManager::new(isolation_root.path().to_path_buf());
        let lease = manager
            .create(repo.path(), "delegation-snapshot")
            .await
            .unwrap();
        let base_revision = lease.base_revision().to_string();

        std::fs::write(lease.worktree_path().join("resume.txt"), "resume me\n").unwrap();
        let snapshot = manager.snapshot(&lease, "snapshot-1").await.unwrap();
        assert_eq!(snapshot.id, "snapshot-1");
        assert_eq!(snapshot.base_revision, base_revision);
        assert_eq!(snapshot.changed_files, vec!["resume.txt"]);

        manager.cleanup(lease).await.unwrap();
        let resumed = manager
            .rehydrate(repo.path(), "snapshot-1", "delegation-resumed")
            .await
            .unwrap();

        assert_eq!(resumed.base_revision(), base_revision);
        assert_eq!(
            std::fs::read_to_string(resumed.worktree_path().join("resume.txt")).unwrap(),
            "resume me\n"
        );
        let resumed_diff = manager.capture_diff(&resumed).await.unwrap();
        assert_eq!(resumed_diff.changed_files, vec!["resume.txt"]);

        manager.cleanup(resumed).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_non_repository_without_creating_isolation_directory() {
        let workspace = TempDir::new().unwrap();
        let isolation_root = TempDir::new().unwrap();
        let manager = WorktreeManager::new(isolation_root.path().join("worktrees"));

        let error = manager
            .create(workspace.path(), "delegation-3")
            .await
            .unwrap_err();

        assert!(matches!(error, WorktreeError::NotGitRepository { .. }));
        assert!(!isolation_root
            .path()
            .join("worktrees/delegation-3")
            .exists());
    }

    #[tokio::test]
    async fn rejects_dirty_parent_instead_of_silently_using_head() {
        let repo = initialized_repo();
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn value() -> u8 { 99 }\n",
        )
        .unwrap();
        let isolation_root = TempDir::new().unwrap();
        let manager = WorktreeManager::new(isolation_root.path().to_path_buf());

        let error = manager
            .create(repo.path(), "delegation-dirty")
            .await
            .unwrap_err();

        assert!(matches!(error, WorktreeError::DirtyWorkingTree { .. }));
        assert!(!isolation_root.path().join("delegation-dirty").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn provisioning_does_not_execute_repository_checkout_hooks() {
        use std::os::unix::fs::PermissionsExt as _;

        let repo = initialized_repo();
        let isolation_root = TempDir::new().unwrap();
        let marker = isolation_root.path().join("hook-ran");
        let hook = repo.path().join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).unwrap();

        let manager = WorktreeManager::new(isolation_root.path().join("worktrees"));
        let lease = manager
            .create(repo.path(), "delegation-no-hooks")
            .await
            .unwrap();

        assert!(!marker.exists());
        manager.cleanup(lease).await.unwrap();
    }
}
