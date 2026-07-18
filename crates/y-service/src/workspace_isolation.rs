//! Service-owned policy for delegated workspace isolation.

use y_core::agent::{
    DelegationError, WorkspaceIsolationMetadata, WorkspaceIsolationMode,
    WorkspaceIsolationPreference,
};
#[cfg(feature = "worktree_isolation")]
use y_core::agent::{WorkspaceCleanupStatus, WorkspaceConflictStatus};
use y_core::tool::{ToolCategory, ToolDefinition};

use crate::ServiceContainer;

#[cfg(feature = "worktree_isolation")]
const MAX_DELEGATION_PATCH_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceIsolationDecision {
    pub mode: WorkspaceIsolationMode,
    pub fail_closed: bool,
}

pub fn resolve_workspace_isolation(
    preference: WorkspaceIsolationPreference,
    write_capable: bool,
) -> WorkspaceIsolationDecision {
    if write_capable {
        return WorkspaceIsolationDecision {
            mode: WorkspaceIsolationMode::Worktree,
            fail_closed: true,
        };
    }

    match preference {
        WorkspaceIsolationPreference::Auto | WorkspaceIsolationPreference::Shared => {
            WorkspaceIsolationDecision {
                mode: WorkspaceIsolationMode::Shared,
                fail_closed: false,
            }
        }
        WorkspaceIsolationPreference::PreferWorktree => WorkspaceIsolationDecision {
            mode: WorkspaceIsolationMode::Worktree,
            fail_closed: false,
        },
        WorkspaceIsolationPreference::RequireWorktree => WorkspaceIsolationDecision {
            mode: WorkspaceIsolationMode::Worktree,
            fail_closed: true,
        },
    }
}

pub fn tools_are_write_capable(definitions: &[ToolDefinition]) -> bool {
    definitions.iter().any(|definition| {
        definition.capabilities.filesystem.mutation.is_some()
            || definition.category == ToolCategory::Shell
    })
}

pub struct DelegatedWorkspace {
    working_directory: Option<String>,
    metadata: Option<WorkspaceIsolationMetadata>,
    #[cfg(feature = "worktree_isolation")]
    lease: Option<y_runtime::worktree::WorktreeLease>,
}

impl DelegatedWorkspace {
    #[cfg(feature = "worktree_isolation")]
    pub async fn prepare(
        container: &ServiceContainer,
        parent_working_directory: Option<String>,
        preference: WorkspaceIsolationPreference,
        snapshot_id: Option<String>,
        write_capable: bool,
        interactive: bool,
    ) -> Result<Self, DelegationError> {
        if !interactive && snapshot_id.is_none() {
            return Ok(Self::passthrough(parent_working_directory));
        }

        if let Some(snapshot_id) = snapshot_id {
            let parent_workspace =
                parent_working_directory.ok_or_else(|| DelegationError::DelegationFailed {
                    message: "workspace snapshot resume requires a parent working directory"
                        .to_string(),
                })?;

            let worktree_id = format!("delegation-{}", uuid::Uuid::new_v4());
            let lease = container
                .worktree_manager
                .rehydrate(
                    std::path::Path::new(&parent_workspace),
                    &snapshot_id,
                    &worktree_id,
                )
                .await
                .map_err(|error| DelegationError::DelegationFailed {
                    message: format!(
                        "failed to rehydrate required workspace snapshot '{snapshot_id}': {error}"
                    ),
                })?;
            return Ok(Self::from_lease(lease, preference, Some(snapshot_id)));
        }

        let decision = resolve_workspace_isolation(preference, write_capable);
        if decision.mode == WorkspaceIsolationMode::Shared {
            return Ok(Self::shared(parent_working_directory, preference, None));
        }

        let Some(parent_workspace) = parent_working_directory else {
            let message = "worktree isolation requires a parent working directory".to_string();
            if decision.fail_closed {
                return Err(DelegationError::DelegationFailed { message });
            }
            return Ok(Self::shared(None, preference, Some(message)));
        };

        let worktree_id = format!("delegation-{}", uuid::Uuid::new_v4());
        match container
            .worktree_manager
            .create(std::path::Path::new(&parent_workspace), &worktree_id)
            .await
        {
            Ok(lease) => Ok(Self::from_lease(lease, preference, None)),
            Err(error) if decision.fail_closed => Err(DelegationError::DelegationFailed {
                message: format!("failed to provision required worktree: {error}"),
            }),
            Err(error) => Ok(Self::shared(
                Some(parent_workspace),
                preference,
                Some(format!("preferred worktree unavailable: {error}")),
            )),
        }
    }

    #[cfg(not(feature = "worktree_isolation"))]
    pub fn prepare(
        _container: &ServiceContainer,
        parent_working_directory: Option<String>,
        preference: WorkspaceIsolationPreference,
        snapshot_id: Option<String>,
        write_capable: bool,
        interactive: bool,
    ) -> std::future::Ready<Result<Self, DelegationError>> {
        let result = if !interactive && snapshot_id.is_none() {
            Ok(Self::passthrough(parent_working_directory))
        } else if let Some(snapshot_id) = snapshot_id {
            Err(DelegationError::DelegationFailed {
                message: format!(
                    "workspace snapshot '{snapshot_id}' cannot be resumed because worktree isolation is unavailable in this build"
                ),
            })
        } else {
            let decision = resolve_workspace_isolation(preference, write_capable);
            if decision.mode == WorkspaceIsolationMode::Shared {
                Ok(Self::shared(parent_working_directory, preference, None))
            } else {
                let message = "worktree isolation is unavailable in this build".to_string();
                if decision.fail_closed {
                    Err(DelegationError::DelegationFailed { message })
                } else {
                    Ok(Self::shared(
                        parent_working_directory,
                        preference,
                        Some(message),
                    ))
                }
            }
        };
        std::future::ready(result)
    }

    pub fn working_directory(&self) -> Option<String> {
        self.working_directory.clone()
    }

    #[cfg(feature = "worktree_isolation")]
    pub async fn finalize(
        mut self,
        container: &ServiceContainer,
    ) -> Option<WorkspaceIsolationMetadata> {
        if let Some(lease) = self.lease.take() {
            if let Some(metadata) = self.metadata.as_mut() {
                let snapshot_id = format!("snapshot-{}", uuid::Uuid::new_v4());
                let snapshot_persisted = match container
                    .worktree_manager
                    .snapshot(&lease, &snapshot_id)
                    .await
                {
                    Ok(snapshot) => {
                        metadata.snapshot_id = Some(snapshot.id);
                        metadata.changed_files = snapshot.changed_files;
                        metadata.patch = bounded_patch(snapshot.patch, metadata);
                        true
                    }
                    Err(error) => {
                        metadata.evidence_error =
                            Some(format!("failed to persist workspace snapshot: {error}"));
                        match container.worktree_manager.capture_diff(&lease).await {
                            Ok(diff) => {
                                metadata.changed_files = diff.changed_files;
                                metadata.patch = bounded_patch(diff.patch, metadata);
                            }
                            Err(diff_error) => {
                                append_evidence_error(
                                    metadata,
                                    format!("failed to capture workspace diff: {diff_error}"),
                                );
                            }
                        }
                        false
                    }
                };

                if snapshot_persisted {
                    match container.worktree_manager.cleanup(lease).await {
                        Ok(()) => metadata.cleanup_status = WorkspaceCleanupStatus::Cleaned,
                        Err(error) => {
                            metadata.cleanup_status = WorkspaceCleanupStatus::Failed;
                            metadata.cleanup_error = Some(error.to_string());
                        }
                    }
                } else {
                    metadata.cleanup_status = WorkspaceCleanupStatus::Failed;
                    metadata.cleanup_error = Some(
                        "worktree retained because no durable snapshot was persisted".to_string(),
                    );
                }
            }
        }
        self.metadata
    }

    #[cfg(not(feature = "worktree_isolation"))]
    pub fn finalize(
        self,
        _container: &ServiceContainer,
    ) -> std::future::Ready<Option<WorkspaceIsolationMetadata>> {
        std::future::ready(self.metadata)
    }

    fn passthrough(working_directory: Option<String>) -> Self {
        Self {
            working_directory,
            metadata: None,
            #[cfg(feature = "worktree_isolation")]
            lease: None,
        }
    }

    #[cfg(feature = "worktree_isolation")]
    fn from_lease(
        lease: y_runtime::worktree::WorktreeLease,
        preference: WorkspaceIsolationPreference,
        snapshot_id: Option<String>,
    ) -> Self {
        let conflict_status = if snapshot_id.is_some() {
            WorkspaceConflictStatus::Clean
        } else {
            WorkspaceConflictStatus::NotChecked
        };
        let working_directory = lease.working_directory().to_string_lossy().to_string();
        let metadata = WorkspaceIsolationMetadata {
            preference,
            mode: WorkspaceIsolationMode::Worktree,
            worktree_id: Some(lease.id().to_string()),
            snapshot_id,
            workspace_path: Some(lease.worktree_path().to_string_lossy().to_string()),
            base_revision: Some(lease.base_revision().to_string()),
            changed_files: Vec::new(),
            patch: None,
            evidence_error: None,
            cleanup_status: WorkspaceCleanupStatus::Pending,
            cleanup_error: None,
            conflict_status,
        };
        Self {
            working_directory: Some(working_directory),
            metadata: Some(metadata),
            lease: Some(lease),
        }
    }

    fn shared(
        working_directory: Option<String>,
        preference: WorkspaceIsolationPreference,
        evidence_error: Option<String>,
    ) -> Self {
        let mut metadata = WorkspaceIsolationMetadata::shared(preference);
        metadata.evidence_error = evidence_error;
        Self {
            working_directory,
            metadata: Some(metadata),
            #[cfg(feature = "worktree_isolation")]
            lease: None,
        }
    }
}

#[cfg(feature = "worktree_isolation")]
fn bounded_patch(mut patch: String, metadata: &mut WorkspaceIsolationMetadata) -> Option<String> {
    if patch.is_empty() {
        return None;
    }
    if patch.len() > MAX_DELEGATION_PATCH_BYTES {
        let mut boundary = MAX_DELEGATION_PATCH_BYTES;
        while !patch.is_char_boundary(boundary) {
            boundary -= 1;
        }
        patch.truncate(boundary);
        append_evidence_error(
            metadata,
            format!("patch exceeded {MAX_DELEGATION_PATCH_BYTES} bytes and was truncated"),
        );
    }
    Some(patch)
}

#[cfg(feature = "worktree_isolation")]
fn append_evidence_error(metadata: &mut WorkspaceIsolationMetadata, message: String) {
    metadata.evidence_error = Some(match metadata.evidence_error.take() {
        Some(existing) => format!("{existing}; {message}"),
        None => message,
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use y_core::agent::{
        DelegationError, WorkspaceCleanupStatus, WorkspaceIsolationMode,
        WorkspaceIsolationPreference,
    };

    use crate::config::ServiceConfig;
    use crate::container::ServiceContainer;

    use super::{resolve_workspace_isolation, DelegatedWorkspace};

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command should start");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_repository(repo: &Path) {
        std::fs::create_dir_all(repo).expect("repository directory");
        git(repo, &["init", "--quiet"]);
        git(repo, &["config", "user.email", "tests@y-agent.invalid"]);
        git(repo, &["config", "user.name", "y-agent tests"]);
        std::fs::write(repo.join("README.md"), "base\n").expect("seed file");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "--quiet", "-m", "initial"]);
    }

    async fn make_test_container(temp: &tempfile::TempDir) -> ServiceContainer {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: temp.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        ServiceContainer::from_config(&config)
            .await
            .expect("test container should build")
    }

    #[test]
    fn auto_shares_read_only_delegations() {
        let decision = resolve_workspace_isolation(WorkspaceIsolationPreference::Auto, false);
        assert_eq!(decision.mode, WorkspaceIsolationMode::Shared);
        assert!(!decision.fail_closed);
    }

    #[test]
    fn auto_and_shared_are_strengthened_for_write_capability() {
        for preference in [
            WorkspaceIsolationPreference::Auto,
            WorkspaceIsolationPreference::Shared,
        ] {
            let decision = resolve_workspace_isolation(preference, true);
            assert_eq!(decision.mode, WorkspaceIsolationMode::Worktree);
            assert!(decision.fail_closed);
        }
    }

    #[test]
    fn preference_and_requirement_have_distinct_failure_policy() {
        let preferred =
            resolve_workspace_isolation(WorkspaceIsolationPreference::PreferWorktree, false);
        assert_eq!(preferred.mode, WorkspaceIsolationMode::Worktree);
        assert!(!preferred.fail_closed);

        let required =
            resolve_workspace_isolation(WorkspaceIsolationPreference::RequireWorktree, false);
        assert_eq!(required.mode, WorkspaceIsolationMode::Worktree);
        assert!(required.fail_closed);
    }

    #[cfg(feature = "worktree_isolation")]
    #[tokio::test]
    async fn delegated_writer_uses_worktree_and_returns_bounded_evidence() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let repository = temp.path().join("repository");
        initialize_repository(&repository);
        let container = make_test_container(&temp).await;

        let workspace = DelegatedWorkspace::prepare(
            &container,
            Some(repository.display().to_string()),
            WorkspaceIsolationPreference::Auto,
            None,
            true,
            true,
        )
        .await
        .expect("write-capable delegation should get a worktree");
        let isolated_directory = workspace
            .working_directory()
            .expect("isolated working directory");
        assert_ne!(Path::new(&isolated_directory), repository);
        assert!(Path::new(&isolated_directory).exists());

        tokio::fs::write(
            Path::new(&isolated_directory).join("result.txt"),
            "isolated\n",
        )
        .await
        .expect("write delegated result");

        let metadata = workspace
            .finalize(&container)
            .await
            .expect("interactive delegation should report isolation metadata");
        assert_eq!(metadata.mode, WorkspaceIsolationMode::Worktree);
        assert_eq!(metadata.cleanup_status, WorkspaceCleanupStatus::Cleaned);
        assert!(metadata.snapshot_id.is_some());
        assert_eq!(metadata.changed_files, vec!["result.txt"]);
        assert!(metadata
            .patch
            .as_deref()
            .is_some_and(|patch| patch.contains("isolated")));
        assert!(!Path::new(&isolated_directory).exists());
        assert!(!repository.join("result.txt").exists());

        let snapshot_id = metadata.snapshot_id.expect("durable snapshot id");
        let resumed = DelegatedWorkspace::prepare(
            &container,
            Some(repository.display().to_string()),
            WorkspaceIsolationPreference::Auto,
            Some(snapshot_id),
            true,
            true,
        )
        .await
        .expect("snapshot should rehydrate into a new worktree");
        let resumed_directory = resumed
            .working_directory()
            .expect("resumed working directory");
        assert_eq!(
            tokio::fs::read_to_string(Path::new(&resumed_directory).join("result.txt"))
                .await
                .expect("resumed result"),
            "isolated\n"
        );
        let resumed_metadata = resumed
            .finalize(&container)
            .await
            .expect("resumed delegation metadata");
        assert_eq!(
            resumed_metadata.cleanup_status,
            WorkspaceCleanupStatus::Cleaned
        );
        assert!(!Path::new(&resumed_directory).exists());
    }

    #[cfg(feature = "worktree_isolation")]
    #[tokio::test]
    async fn required_worktree_fails_closed_outside_git_repository() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let workspace = temp.path().join("plain-directory");
        std::fs::create_dir_all(&workspace).expect("plain directory");
        let container = make_test_container(&temp).await;

        let error = DelegatedWorkspace::prepare(
            &container,
            Some(workspace.display().to_string()),
            WorkspaceIsolationPreference::RequireWorktree,
            None,
            false,
            true,
        )
        .await
        .err()
        .expect("required worktree should fail outside Git");

        assert!(matches!(error, DelegationError::DelegationFailed { .. }));
        assert!(error
            .to_string()
            .contains("failed to provision required worktree"));
    }

    #[cfg(feature = "worktree_isolation")]
    #[tokio::test]
    async fn concurrent_writers_receive_distinct_worktrees_and_snapshots() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let repository = temp.path().join("repository");
        initialize_repository(&repository);
        let container = make_test_container(&temp).await;
        let parent_workspace = repository.display().to_string();

        let (first, second) = tokio::join!(
            DelegatedWorkspace::prepare(
                &container,
                Some(parent_workspace.clone()),
                WorkspaceIsolationPreference::Auto,
                None,
                true,
                true,
            ),
            DelegatedWorkspace::prepare(
                &container,
                Some(parent_workspace),
                WorkspaceIsolationPreference::Auto,
                None,
                true,
                true,
            )
        );
        let first = first.expect("first worktree");
        let second = second.expect("second worktree");
        let first_directory = first.working_directory().expect("first directory");
        let second_directory = second.working_directory().expect("second directory");
        assert_ne!(first_directory, second_directory);

        tokio::fs::write(Path::new(&first_directory).join("parallel.txt"), "first\n")
            .await
            .expect("first write");
        tokio::fs::write(
            Path::new(&second_directory).join("parallel.txt"),
            "second\n",
        )
        .await
        .expect("second write");

        let (first_metadata, second_metadata) =
            tokio::join!(first.finalize(&container), second.finalize(&container));
        let first_metadata = first_metadata.expect("first metadata");
        let second_metadata = second_metadata.expect("second metadata");
        assert_ne!(first_metadata.worktree_id, second_metadata.worktree_id);
        assert_ne!(first_metadata.snapshot_id, second_metadata.snapshot_id);
        assert!(first_metadata
            .patch
            .as_deref()
            .is_some_and(|patch| patch.contains("first")));
        assert!(second_metadata
            .patch
            .as_deref()
            .is_some_and(|patch| patch.contains("second")));
        assert!(!repository.join("parallel.txt").exists());
    }
}
