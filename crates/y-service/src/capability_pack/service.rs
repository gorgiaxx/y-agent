use std::path::Path;
use std::sync::Arc;

use super::activation::{
    CapabilityPackActivationGrant, CapabilityPackActivationReceipt,
    CapabilityPackActivationRevocationReceipt, CapabilityPackActivationStore,
    CapabilityPackLiveActivationReceipt,
};
use super::durable::DurableCapabilityPackInstaller;
use super::journal::CapabilityPackTransactionJournal;
use super::live::CapabilityPackLiveOwners;
use super::owner::CapabilityPackOwnerBackend;
use super::ownership::{CapabilityPackInstallIntent, CapabilityPackOwnershipStore};
use super::transaction::{
    change_label, CapabilityPackChangeKind, CapabilityPackInstallError,
    CapabilityPackInstallOptions, CapabilityPackInstallReceipt, CapabilityPackInstaller,
    CapabilityPackPreview,
};
use super::validator::{
    CapabilityPackValidationReport, CapabilityPackValidator, ValidatedCapabilityPack,
};
use crate::container::ServiceContainer;
use crate::workspace::{WorkspaceService, WorkspaceTrustStatus};
use crate::{PermissionPromptResponse, TurnEvent, TurnEventSender};
use tokio_util::sync::CancellationToken;
use y_core::permission_types::{PermissionBehavior, PermissionReason, PermissionResult};
use y_core::types::SessionId;
use y_guardrails::permission_pipeline::ToolPermissionRequest;

pub struct CapabilityPackService;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityPackRollbackReceipt {
    pub pack_id: String,
    pub removed_version: String,
    pub restored_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityPackRemoveReceipt {
    pub pack_id: String,
    pub removed_versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityPackInspection {
    pub validation: CapabilityPackValidationReport,
    pub preview: Option<CapabilityPackPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstalledCapabilityPackSummary {
    pub pack_id: String,
    pub current_version: String,
    pub current_transaction_id: String,
    pub installed_versions: Vec<String>,
    pub resources: Vec<String>,
    pub executable_resources: Vec<String>,
    pub activation_grants: Vec<CapabilityPackActivationGrant>,
    pub live_resources: Vec<String>,
}

impl CapabilityPackService {
    pub async fn inspect_local(
        container: &ServiceContainer,
        pack_root: &Path,
    ) -> Result<CapabilityPackInspection, CapabilityPackInstallError> {
        let validation = CapabilityPackValidator::validate(pack_root);
        let Some(pack) = validation.pack.as_ref().filter(|_| validation.valid) else {
            return Ok(CapabilityPackInspection {
                validation,
                preview: None,
            });
        };
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let backend = CapabilityPackOwnerBackend::new(container);
        let preview = CapabilityPackInstaller::preview(&backend, pack).await?;
        Ok(CapabilityPackInspection {
            validation,
            preview: Some(preview),
        })
    }

    pub async fn install_local(
        container: &ServiceContainer,
        pack_root: &Path,
        options: CapabilityPackInstallOptions,
    ) -> Result<CapabilityPackInstallReceipt, CapabilityPackInstallError> {
        let validation = CapabilityPackValidator::validate(pack_root);
        let Some(pack) = validation.pack.filter(|_| validation.valid) else {
            let message = validation
                .issues
                .iter()
                .map(|issue| format!("{:?}: {}", issue.code, issue.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(CapabilityPackInstallError::ResourceValidationFailed {
                resource: "capability-pack".to_string(),
                message,
            });
        };
        Self::install(container, &pack, options).await
    }

    pub async fn list_installed(
        container: &ServiceContainer,
    ) -> Result<Vec<InstalledCapabilityPackSummary>, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let index = ownership.load().map_err(|error| ownership_error(&error))?;
        let grants = activation_store(container)
            .grants()
            .map_err(activation_error)?;
        let mut summaries = Vec::with_capacity(index.packs.len());
        for (pack_id, installed) in index.packs {
            let resources = installed
                .versions
                .last()
                .into_iter()
                .flat_map(|version| &version.resources)
                .map(|resource| resource.key.clone())
                .collect::<Vec<_>>();
            let executable_resources = resources
                .iter()
                .filter(|key| {
                    key.starts_with("mcp:") || key.starts_with("hook:") || key.starts_with("lsp:")
                })
                .cloned()
                .collect::<Vec<_>>();
            let activation_grants = grants
                .iter()
                .filter(|grant| {
                    grant.pack_id == pack_id
                        && grant.pack_version == installed.current_version
                        && grant.transaction_id == installed.current_transaction_id
                })
                .cloned()
                .collect::<Vec<_>>();
            let live_resources =
                CapabilityPackLiveOwners::active_resources(container, &executable_resources)
                    .await
                    .map_err(activation_error)?;
            summaries.push(InstalledCapabilityPackSummary {
                pack_id,
                current_version: installed.current_version,
                current_transaction_id: installed.current_transaction_id,
                installed_versions: installed
                    .versions
                    .iter()
                    .map(|version| version.version.clone())
                    .collect(),
                resources,
                executable_resources,
                activation_grants,
                live_resources,
            });
        }
        Ok(summaries)
    }

    pub async fn grant_activation(
        container: &ServiceContainer,
        workspace_service: &WorkspaceService,
        pack_id: &str,
        workspace_path: &Path,
        session_id: &SessionId,
        progress: Option<&TurnEventSender>,
        cancel_token: Option<&CancellationToken>,
    ) -> Result<CapabilityPackActivationReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let trust = workspace_service
            .workspace_trust(workspace_path)
            .map_err(|error| CapabilityPackInstallError::Activation {
                message: format!("failed to resolve workspace trust: {error}"),
            })?;
        if trust.status != WorkspaceTrustStatus::Trusted {
            return Err(CapabilityPackInstallError::Activation {
                message: format!("workspace is not trusted: {:?}", trust.status),
            });
        }
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let index = ownership.load().map_err(|error| ownership_error(&error))?;
        let installed =
            index
                .packs
                .get(pack_id)
                .ok_or_else(|| CapabilityPackInstallError::Activation {
                    message: format!("pack is not installed: {pack_id}"),
                })?;
        let version =
            installed
                .versions
                .last()
                .ok_or_else(|| CapabilityPackInstallError::Activation {
                    message: format!("pack has no installed version: {pack_id}"),
                })?;
        let executable_resources = version
            .resources
            .iter()
            .map(|resource| resource.key.clone())
            .filter(|key| {
                key.starts_with("mcp:") || key.starts_with("hook:") || key.starts_with("lsp:")
            })
            .collect::<Vec<_>>();
        if executable_resources.is_empty() {
            return Err(CapabilityPackInstallError::Activation {
                message: "pack has no executable declarations".to_string(),
            });
        }
        let content = format!(
            "{}@{} workspace={} resources={}",
            pack_id,
            installed.current_version,
            trust.canonical_path,
            executable_resources.join(",")
        );
        let tool_result = PermissionResult {
            behavior: PermissionBehavior::Ask,
            reason: PermissionReason::SafetyCheck {
                reason: "capability-pack executable activation".to_string(),
            },
            message: Some("executable capability activation requires approval".to_string()),
            updated_input: None,
        };
        let decision = container.guardrail_manager.evaluate_tool_permission(
            ToolPermissionRequest::new("CapabilityPackActivate", true, &tool_result)
                .with_input_content(Some(&content)),
        );
        if decision.behavior == PermissionBehavior::Deny {
            return Err(CapabilityPackInstallError::Activation {
                message: decision
                    .message
                    .unwrap_or_else(|| "activation denied by permission policy".to_string()),
            });
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        container
            .session_state
            .pending_permissions
            .lock()
            .await
            .insert(
                request_id.clone(),
                crate::chat_types::PendingPermission::new(session_id.clone(), response_tx),
            );
        if let Some(progress) = progress {
            let _ = progress.send(TurnEvent::PermissionRequest {
                request_id: request_id.clone(),
                tool_name: "CapabilityPackActivate".to_string(),
                action_description: format!("Activate executable declarations from {pack_id}"),
                reason: "workspace is trusted; explicit activation approval is still required"
                    .to_string(),
                content_preview: Some(content),
            });
        }
        let wait_timeout =
            std::time::Duration::from_millis(container.guardrail_manager.config().hitl.timeout_ms);
        let response = await_activation_response(
            container,
            &request_id,
            response_rx,
            wait_timeout,
            cancel_token,
        )
        .await;
        if !matches!(
            response,
            Some(
                PermissionPromptResponse::Approve
                    | PermissionPromptResponse::ApproveAlways
                    | PermissionPromptResponse::AllowAllForSession
            )
        ) {
            return Err(CapabilityPackInstallError::Activation {
                message: "activation denied, cancelled, or timed out".to_string(),
            });
        }
        let grant = CapabilityPackActivationGrant {
            pack_id: pack_id.to_string(),
            pack_version: installed.current_version.clone(),
            transaction_id: installed.current_transaction_id.clone(),
            canonical_workspace: trust.canonical_path,
            approved_at: chrono::Utc::now().to_rfc3339(),
        };
        let grant = CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json"),
        )
        .grant(grant)
        .map_err(|message| CapabilityPackInstallError::Activation { message })?;
        Ok(CapabilityPackActivationReceipt {
            grant,
            executable_resources,
        })
    }

    pub async fn activate_granted(
        container: &Arc<ServiceContainer>,
        workspace_service: &WorkspaceService,
        pack_id: &str,
        workspace_path: &Path,
    ) -> Result<CapabilityPackLiveActivationReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let trust = workspace_service
            .workspace_trust(workspace_path)
            .map_err(|error| {
                activation_error(format!("failed to resolve workspace trust: {error}"))
            })?;
        if trust.status != WorkspaceTrustStatus::Trusted {
            return Err(activation_error(format!(
                "workspace is not trusted: {:?}",
                trust.status
            )));
        }
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let index = ownership.load().map_err(|error| ownership_error(&error))?;
        let installed = index
            .packs
            .get(pack_id)
            .ok_or_else(|| activation_error(format!("pack is not installed: {pack_id}")))?;
        let resource_keys = current_executable_resources(installed);
        let activations = activation_store(container);
        let grant = activations
            .grants()
            .map_err(activation_error)?
            .into_iter()
            .find(|grant| {
                grant.pack_id == pack_id
                    && grant.pack_version == installed.current_version
                    && grant.transaction_id == installed.current_transaction_id
                    && grant.canonical_workspace == trust.canonical_path
            })
            .ok_or_else(|| {
                activation_error(format!(
                    "no current activation grant for {pack_id} in {}",
                    trust.canonical_path
                ))
            })?;
        #[cfg(all(feature = "hook_handlers", feature = "llm_hooks"))]
        container.install_hook_agent_runner();
        let activated_resources = CapabilityPackLiveOwners::activate(container, &resource_keys)
            .await
            .map_err(activation_error)?;
        Ok(CapabilityPackLiveActivationReceipt {
            grant,
            activated_resources,
        })
    }

    pub async fn revoke_activation(
        container: &ServiceContainer,
        workspace_service: &WorkspaceService,
        pack_id: &str,
        workspace_path: &Path,
    ) -> Result<CapabilityPackActivationRevocationReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let trust = workspace_service
            .workspace_trust(workspace_path)
            .map_err(|error| {
                activation_error(format!("failed to resolve workspace path: {error}"))
            })?;
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let index = ownership.load().map_err(|error| ownership_error(&error))?;
        let installed = index
            .packs
            .get(pack_id)
            .ok_or_else(|| activation_error(format!("pack is not installed: {pack_id}")))?;
        let resource_keys = current_executable_resources(installed);
        let activations = activation_store(container);
        let revoked = activations
            .revoke_grant(&installed.current_transaction_id, &trust.canonical_path)
            .map_err(activation_error)?;
        if !revoked {
            return Err(activation_error(format!(
                "no current activation grant for {pack_id} in {}",
                trust.canonical_path
            )));
        }
        let has_remaining_grants = activations
            .grants()
            .map_err(activation_error)?
            .iter()
            .any(|grant| grant.transaction_id == installed.current_transaction_id);
        let deactivated_resources = if has_remaining_grants {
            Vec::new()
        } else {
            CapabilityPackLiveOwners::deactivate(container, &resource_keys)
                .await
                .map_err(activation_error)?
        };
        Ok(CapabilityPackActivationRevocationReceipt {
            pack_id: pack_id.to_string(),
            pack_version: installed.current_version.clone(),
            canonical_workspace: trust.canonical_path,
            deactivated_resources,
        })
    }

    pub async fn install(
        container: &ServiceContainer,
        pack: &ValidatedCapabilityPack,
        options: CapabilityPackInstallOptions,
    ) -> Result<CapabilityPackInstallReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let backend = CapabilityPackOwnerBackend::new(container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let activations = activation_store(container);
        reconcile_managed_commits(&journal, &ownership)?;
        let intent = ownership
            .validate_install(pack)
            .map_err(|error| ownership_error(&error))?;
        let preview = CapabilityPackInstaller::preview(&backend, pack).await?;
        let replacements = preview
            .changes
            .iter()
            .filter(|change| change.change == CapabilityPackChangeKind::Replace)
            .map(change_label)
            .collect::<Vec<_>>();
        if !replacements.is_empty() && !options.allow_replacements {
            return Err(CapabilityPackInstallError::ReplacementApprovalRequired {
                resources: replacements,
            });
        }
        if let CapabilityPackInstallIntent::Update {
            previous_transaction_id,
        } = intent
        {
            let index = ownership.load().map_err(|error| ownership_error(&error))?;
            if let Some(installed) = index.packs.get(&pack.id) {
                CapabilityPackLiveOwners::deactivate(
                    container,
                    &current_executable_resources(installed),
                )
                .await
                .map_err(activation_error)?;
            }
            activations
                .revoke_transaction(&previous_transaction_id)
                .map_err(activation_error)?;
        }
        let mut pending =
            DurableCapabilityPackInstaller::install_pending(&backend, &journal, pack, options)
                .await?;
        DurableCapabilityPackInstaller::decide_managed_commit(&backend, &journal, &mut pending)
            .await?;
        ownership
            .commit(pending.record())
            .map_err(|error| ownership_error(&error))?;
        DurableCapabilityPackInstaller::mark_committed(&journal, &mut pending)?;
        Ok(pending.into_receipt())
    }

    pub async fn rollback(
        container: &ServiceContainer,
        pack_id: &str,
    ) -> Result<CapabilityPackRollbackReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let backend = CapabilityPackOwnerBackend::new(container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let activations = CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json"),
        );
        reconcile_managed_commits(&journal, &ownership)?;
        rollback_current(
            container,
            &backend,
            &journal,
            &ownership,
            &activations,
            pack_id,
        )
        .await
    }

    pub async fn remove(
        container: &ServiceContainer,
        pack_id: &str,
    ) -> Result<CapabilityPackRemoveReceipt, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let backend = CapabilityPackOwnerBackend::new(container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let activations = CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json"),
        );
        reconcile_managed_commits(&journal, &ownership)?;
        let mut removed_versions = Vec::new();
        loop {
            let index = ownership.load().map_err(|error| ownership_error(&error))?;
            if !index.packs.contains_key(pack_id) {
                break;
            }
            let receipt = rollback_current(
                container,
                &backend,
                &journal,
                &ownership,
                &activations,
                pack_id,
            )
            .await?;
            removed_versions.push(receipt.removed_version);
        }
        if removed_versions.is_empty() {
            return Err(CapabilityPackInstallError::Ownership {
                message: format!("pack is not installed: {pack_id}"),
            });
        }
        Ok(CapabilityPackRemoveReceipt {
            pack_id: pack_id.to_string(),
            removed_versions,
        })
    }

    pub(crate) async fn recover(
        container: &ServiceContainer,
    ) -> Result<Vec<String>, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let backend = CapabilityPackOwnerBackend::new(container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let activations = CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json"),
        );
        reconcile_managed_commits(&journal, &ownership)?;
        let recovered = DurableCapabilityPackInstaller::recover(&backend, &journal).await?;
        for record in journal
            .load_all()
            .map_err(|error| CapabilityPackInstallError::Journal {
                message: error.to_string(),
            })?
        {
            if record.ownership_managed
                && record.status == super::journal::CapabilityPackTransactionStatus::RolledBack
            {
                ownership
                    .uncommit(&record)
                    .map_err(|error| ownership_error(&error))?;
                activations
                    .revoke_transaction(&record.id)
                    .map_err(|message| CapabilityPackInstallError::Activation { message })?;
            }
        }
        Ok(recovered)
    }

    pub(crate) async fn reconcile_live_activations(
        container: &Arc<ServiceContainer>,
    ) -> Result<Vec<String>, CapabilityPackInstallError> {
        let _guard = container.capability_pack_lifecycle_lock.lock().await;
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let index = ownership.load().map_err(|error| ownership_error(&error))?;
        let activations = activation_store(container);
        let grants = activations.grants().map_err(activation_error)?;
        let workspace_service = container.config_dir.as_deref().map(WorkspaceService::new);
        let mut activated_transactions = std::collections::HashSet::new();
        let mut errors = Vec::new();
        for grant in grants {
            let current = index.packs.get(&grant.pack_id).is_some_and(|installed| {
                installed.current_version == grant.pack_version
                    && installed.current_transaction_id == grant.transaction_id
            });
            let trusted = workspace_service.as_ref().is_some_and(|service| {
                service
                    .workspace_trust(Path::new(&grant.canonical_workspace))
                    .is_ok_and(|decision| {
                        decision.status == WorkspaceTrustStatus::Trusted
                            && decision.canonical_path == grant.canonical_workspace
                    })
            });
            if !current || !trusted {
                activations
                    .revoke_grant(&grant.transaction_id, &grant.canonical_workspace)
                    .map_err(activation_error)?;
                continue;
            }
            if !activated_transactions.insert(grant.transaction_id.clone()) {
                continue;
            }
            let Some(installed) = index.packs.get(&grant.pack_id) else {
                continue;
            };
            if let Err(error) = CapabilityPackLiveOwners::activate(
                container,
                &current_executable_resources(installed),
            )
            .await
            {
                errors.push(format!(
                    "failed to activate {}@{}: {error}",
                    grant.pack_id, grant.pack_version
                ));
            }
        }
        Ok(errors)
    }
}

async fn await_activation_response(
    container: &ServiceContainer,
    request_id: &str,
    response_rx: tokio::sync::oneshot::Receiver<PermissionPromptResponse>,
    wait_timeout: std::time::Duration,
    cancel_token: Option<&CancellationToken>,
) -> Option<PermissionPromptResponse> {
    let wait = tokio::time::timeout(wait_timeout, response_rx);
    let response = if let Some(token) = cancel_token {
        tokio::select! {
            result = wait => result.ok().and_then(Result::ok),
            () = token.cancelled() => None,
        }
    } else {
        wait.await.ok().and_then(Result::ok)
    };
    container
        .session_state
        .pending_permissions
        .lock()
        .await
        .remove(request_id);
    response
}

fn reconcile_managed_commits(
    journal: &CapabilityPackTransactionJournal,
    ownership: &CapabilityPackOwnershipStore,
) -> Result<(), CapabilityPackInstallError> {
    let mut records = journal
        .load_all()
        .map_err(|error| CapabilityPackInstallError::Journal {
            message: error.to_string(),
        })?;
    records.sort_by(|left, right| {
        left.pack_id.cmp(&right.pack_id).then_with(|| {
            let left_version = semver::Version::parse(&left.pack_version);
            let right_version = semver::Version::parse(&right.pack_version);
            match (left_version, right_version) {
                (Ok(left_version), Ok(right_version)) => left_version.cmp(&right_version),
                _ => left.pack_version.cmp(&right.pack_version),
            }
        })
    });
    for mut record in records {
        if !record.ownership_managed
            || !matches!(
                record.status,
                super::journal::CapabilityPackTransactionStatus::CommitDecided
                    | super::journal::CapabilityPackTransactionStatus::Committed
            )
        {
            continue;
        }
        ownership
            .commit(&record)
            .map_err(|error| ownership_error(&error))?;
        if record.status == super::journal::CapabilityPackTransactionStatus::CommitDecided {
            DurableCapabilityPackInstaller::mark_record_committed(journal, &mut record)?;
        }
    }
    Ok(())
}

async fn rollback_current(
    container: &ServiceContainer,
    backend: &CapabilityPackOwnerBackend<'_>,
    journal: &CapabilityPackTransactionJournal,
    ownership: &CapabilityPackOwnershipStore,
    activations: &CapabilityPackActivationStore,
    pack_id: &str,
) -> Result<CapabilityPackRollbackReceipt, CapabilityPackInstallError> {
    let index = ownership.load().map_err(|error| ownership_error(&error))?;
    let installed =
        index
            .packs
            .get(pack_id)
            .ok_or_else(|| CapabilityPackInstallError::Ownership {
                message: format!("pack is not installed: {pack_id}"),
            })?;
    let transaction_id = installed.current_transaction_id.clone();
    let removed_version = installed.current_version.clone();
    CapabilityPackLiveOwners::deactivate(container, &current_executable_resources(installed))
        .await
        .map_err(activation_error)?;
    activations
        .revoke_transaction(&transaction_id)
        .map_err(activation_error)?;
    let record =
        DurableCapabilityPackInstaller::rollback_managed(backend, journal, &transaction_id).await?;
    let updated = ownership
        .uncommit(&record)
        .map_err(|error| ownership_error(&error))?;
    let restored_version = updated
        .packs
        .get(pack_id)
        .map(|pack| pack.current_version.clone());
    Ok(CapabilityPackRollbackReceipt {
        pack_id: pack_id.to_string(),
        removed_version,
        restored_version,
    })
}

fn activation_store(container: &ServiceContainer) -> CapabilityPackActivationStore {
    CapabilityPackActivationStore::new(
        container
            .data_dir
            .join("capability-packs/activation-grants.json"),
    )
}

fn current_executable_resources(
    installed: &super::ownership::InstalledCapabilityPack,
) -> Vec<String> {
    installed
        .versions
        .last()
        .into_iter()
        .flat_map(|version| &version.resources)
        .map(|resource| resource.key.clone())
        .filter(|key| {
            key.starts_with("mcp:") || key.starts_with("hook:") || key.starts_with("lsp:")
        })
        .collect()
}

fn activation_error(message: impl Into<String>) -> CapabilityPackInstallError {
    CapabilityPackInstallError::Activation {
        message: message.into(),
    }
}

fn ownership_error(
    error: &super::ownership::CapabilityPackOwnershipError,
) -> CapabilityPackInstallError {
    CapabilityPackInstallError::Ownership {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::capability_pack::owner::{hash_entry, CapabilityPackOwnerBackend};
    use crate::capability_pack::{
        CapabilityPackInstallOptions, CapabilityPackProvenance, CapabilityPackSourceKind,
        CapabilityPackTransactionStatus, CapabilityResourceKind, DurableCapabilityPackInstaller,
        ValidatedCapabilityPack, ValidatedCapabilityResource,
    };
    use crate::{ServiceConfig, ServiceContainer};

    async fn setup() -> (tempfile::TempDir, ServiceConfig, ServiceContainer) {
        setup_with_hooks(y_hooks::HookConfig::default()).await
    }

    async fn setup_with_hooks(
        hooks: y_hooks::HookConfig,
    ) -> (tempfile::TempDir, ServiceConfig, ServiceContainer) {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path();
        let config_dir = root.join("config");
        std::fs::create_dir_all(config_dir.join("prompts")).expect("prompts dir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: root.join("state.db").display().to_string(),
            pool_size: 1,
            wal_enabled: true,
            transcript_dir: root.join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        config.prompts_dir = Some(config_dir.join("prompts"));
        config.skills_dir = Some(config_dir.join("skills"));
        config.hooks = hooks;
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("service container");
        (temp, config, container)
    }

    fn write_skill(path: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(path).expect("skill dir");
        std::fs::write(
            path.join("skill.toml"),
            format!(
                r#"name = "{name}"
description = "Capability pack lifecycle test"
root_content = "{content}"
"#,
            ),
        )
        .expect("skill manifest");
        std::fs::write(path.join("root.md"), content).expect("skill root");
    }

    fn write_local_pack(root: &Path, pack_id: &str, version: &str, skill_id: &str) {
        let skill_path = root.join("skills").join(skill_id);
        write_skill(&skill_path, skill_id, "# Local capability pack skill");
        let resource_hash = hash_entry(&skill_path).expect("resource hash");
        std::fs::write(
            root.join("capability-pack.toml"),
            format!(
                r#"[pack]
schema_version = 1
id = "{pack_id}"
version = "{version}"
description = "Presentation lifecycle test"

[[resources]]
kind = "skill"
id = "{skill_id}"
path = "skills/{skill_id}"
sha256 = "{resource_hash}"
"#,
            ),
        )
        .expect("pack manifest");
    }

    #[tokio::test]
    async fn presentation_inspection_validates_and_previews_a_local_pack() {
        let (temp, _config, container) = setup().await;
        let pack_root = temp.path().join("local-pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        write_local_pack(&pack_root, "local-pack", "1.0.0", "review-rust");

        let inspection = CapabilityPackService::inspect_local(&container, &pack_root)
            .await
            .expect("inspect pack");

        assert!(inspection.validation.valid);
        assert_eq!(inspection.preview.expect("preview").pack_id, "local-pack");
    }

    #[tokio::test]
    async fn presentation_install_and_list_report_installed_without_implying_activation() {
        let (temp, _config, container) = setup().await;
        let pack_root = temp.path().join("local-pack");
        std::fs::create_dir_all(&pack_root).expect("pack root");
        write_local_pack(&pack_root, "local-pack", "1.0.0", "review-rust");

        CapabilityPackService::install_local(
            &container,
            &pack_root,
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install pack");
        let installed = CapabilityPackService::list_installed(&container)
            .await
            .expect("list packs");

        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].pack_id, "local-pack");
        assert_eq!(installed[0].current_version, "1.0.0");
        assert_eq!(installed[0].installed_versions, vec!["1.0.0"]);
        assert!(installed[0].activation_grants.is_empty());
        assert!(installed[0].live_resources.is_empty());
    }

    fn pack(id: &str, version: &str, root: PathBuf) -> ValidatedCapabilityPack {
        let resource_path = root.join("review-rust");
        ValidatedCapabilityPack {
            schema_version: 1,
            id: id.to_string(),
            version: version.to_string(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: root.clone(),
                manifest_path: root.join("capability-pack.toml"),
                manifest_sha256: hash_entry(&resource_path).expect("manifest stand-in hash"),
            },
            resources: vec![ValidatedCapabilityResource {
                kind: CapabilityResourceKind::Skill,
                id: "review-rust".into(),
                sha256: hash_entry(&resource_path).expect("resource hash"),
                path: resource_path,
            }],
        }
    }

    fn executable_pack(id: &str, root: PathBuf) -> ValidatedCapabilityPack {
        executable_pack_with_config(
            id,
            root,
            y_tools::McpServerConfig {
                name: "pack-mcp".to_string(),
                transport: "stdio".to_string(),
                command: Some("must-not-start".to_string()),
                args: Vec::new(),
                url: None,
                env: std::collections::HashMap::new(),
                enabled: true,
                headers: std::collections::HashMap::new(),
                startup_timeout_secs: 30,
                tool_timeout_secs: 120,
                cwd: None,
                bearer_token: None,
                enabled_tools: None,
                disabled_tools: None,
                auto_reconnect: true,
                max_reconnect_attempts: 5,
            },
        )
    }

    fn hook_pack(id: &str, root: PathBuf) -> ValidatedCapabilityPack {
        let resource_path = root.join("audit-hook.toml");
        std::fs::create_dir_all(&root).expect("pack root");
        std::fs::write(
            &resource_path,
            r#"hook_point = "post_tool_execute"
matcher = "*"

[[handlers]]
type = "command"
command = "/bin/true"
"#,
        )
        .expect("hook declaration");
        ValidatedCapabilityPack {
            schema_version: 1,
            id: id.to_string(),
            version: "1.0.0".to_string(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: root.clone(),
                manifest_path: root.join("capability-pack.toml"),
                manifest_sha256: hash_entry(&resource_path).expect("manifest stand-in hash"),
            },
            resources: vec![ValidatedCapabilityResource {
                kind: CapabilityResourceKind::Hook,
                id: "audit-hook".into(),
                sha256: hash_entry(&resource_path).expect("resource hash"),
                path: resource_path,
            }],
        }
    }

    #[cfg(feature = "lsp")]
    fn lsp_pack(id: &str, root: PathBuf) -> ValidatedCapabilityPack {
        let resource_path = root.join("pack-language.toml");
        std::fs::create_dir_all(&root).expect("pack root");
        std::fs::write(
            &resource_path,
            r#"id = "pack-language"
command = "pack-language-server"
language_id = "pack-language"
extensions = ["pack"]
root_markers = ["pack.toml"]
"#,
        )
        .expect("LSP declaration");
        ValidatedCapabilityPack {
            schema_version: 1,
            id: id.to_string(),
            version: "1.0.0".to_string(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: root.clone(),
                manifest_path: root.join("capability-pack.toml"),
                manifest_sha256: hash_entry(&resource_path).expect("manifest stand-in hash"),
            },
            resources: vec![ValidatedCapabilityResource {
                kind: CapabilityResourceKind::Lsp,
                id: "pack-language".into(),
                sha256: hash_entry(&resource_path).expect("resource hash"),
                path: resource_path,
            }],
        }
    }

    fn hook_config(hook_point: &str) -> y_hooks::HookConfig {
        let mut hook_handlers = std::collections::HashMap::new();
        hook_handlers.insert(
            hook_point.to_string(),
            vec![y_hooks::config::HookHandlerGroupConfig {
                matcher: "*".to_string(),
                timeout_ms: None,
                handlers: vec![y_hooks::config::HandlerConfig::Command {
                    command: "/bin/true".to_string(),
                    r#async: false,
                }],
            }],
        );
        y_hooks::HookConfig {
            hook_handlers,
            ..y_hooks::HookConfig::default()
        }
    }

    fn executable_pack_with_config(
        id: &str,
        root: PathBuf,
        config: y_tools::McpServerConfig,
    ) -> ValidatedCapabilityPack {
        executable_pack_with_config_version(id, "1.0.0", root, config)
    }

    fn executable_pack_with_config_version(
        id: &str,
        version: &str,
        root: PathBuf,
        config: y_tools::McpServerConfig,
    ) -> ValidatedCapabilityPack {
        let resource_path = root.join("pack-mcp.toml");
        std::fs::create_dir_all(&root).expect("pack root");
        std::fs::write(&resource_path, toml::to_string(&config).expect("MCP TOML"))
            .expect("MCP declaration");
        ValidatedCapabilityPack {
            schema_version: 1,
            id: id.to_string(),
            version: version.to_string(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: root.clone(),
                manifest_path: root.join("capability-pack.toml"),
                manifest_sha256: hash_entry(&resource_path).expect("manifest stand-in hash"),
            },
            resources: vec![ValidatedCapabilityResource {
                kind: CapabilityResourceKind::Mcp,
                id: "pack-mcp".into(),
                sha256: hash_entry(&resource_path).expect("resource hash"),
                path: resource_path,
            }],
        }
    }

    #[cfg(unix)]
    fn live_mcp_pack(id: &str, root: PathBuf) -> ValidatedCapabilityPack {
        live_mcp_pack_version(id, "1.0.0", root)
    }

    #[cfg(unix)]
    fn live_mcp_pack_version(id: &str, version: &str, root: PathBuf) -> ValidatedCapabilityPack {
        let script = root.join("mock-mcp.sh");
        std::fs::create_dir_all(&root).expect("pack root");
        std::fs::write(
            &script,
            r#"#!/bin/sh
IFS= read -r _
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","serverInfo":{"name":"pack-mcp","version":"1.0"},"capabilities":{"tools":{}},"instructions":"Pack MCP instructions"}}'
IFS= read -r _
IFS= read -r _
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"Search from pack","inputSchema":{"type":"object"}}]}}'
while IFS= read -r _; do :; done
"#,
        )
        .expect("mock MCP server");
        executable_pack_with_config_version(
            id,
            version,
            root,
            y_tools::McpServerConfig {
                name: "pack-mcp".to_string(),
                transport: "stdio".to_string(),
                command: Some("/bin/sh".to_string()),
                args: vec![script.display().to_string()],
                url: None,
                env: std::collections::HashMap::new(),
                enabled: true,
                headers: std::collections::HashMap::new(),
                startup_timeout_secs: 5,
                tool_timeout_secs: 5,
                cwd: None,
                bearer_token: None,
                enabled_tools: None,
                disabled_tools: None,
                auto_reconnect: false,
                max_reconnect_attempts: 0,
            },
        )
    }

    async fn approve_activation(
        container: &ServiceContainer,
        workspace_service: &WorkspaceService,
        workspace: &Path,
    ) -> CapabilityPackActivationReceipt {
        let session_id = SessionId("activation-session".into());
        let (progress, mut events) = TurnEventSender::channel();
        let activation = CapabilityPackService::grant_activation(
            container,
            workspace_service,
            "exec-pack",
            workspace,
            &session_id,
            Some(&progress),
            None,
        );
        let responder = async {
            let (event, _) = events.recv().await.expect("permission event");
            let TurnEvent::PermissionRequest { request_id, .. } = event else {
                panic!("expected permission request");
            };
            container
                .session_state
                .pending_permissions
                .lock()
                .await
                .remove(&request_id)
                .expect("pending activation approval")
                .send(PermissionPromptResponse::Approve)
                .expect("approve activation");
        };
        let (receipt, ()) = tokio::join!(activation, responder);
        receipt.expect("activation grant")
    }

    #[tokio::test]
    async fn managed_install_persists_ownership_and_monotonic_update_history() {
        let (temp, _config, container) = setup().await;
        let first_root = temp.path().join("pack-v1");
        write_skill(
            &first_root.join("review-rust"),
            "review-rust",
            "Version one.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.0.0", first_root),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("initial install");

        let second_root = temp.path().join("pack-v2");
        write_skill(
            &second_root.join("review-rust"),
            "review-rust",
            "Version two.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.1.0", second_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("pack update");

        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        )
        .load()
        .expect("ownership index");
        let installed = ownership.packs.get("rust-team").expect("installed pack");
        assert_eq!(installed.current_version, "1.1.0");
        assert_eq!(installed.versions.len(), 2);
        assert_eq!(ownership.generation, 2);
        assert_eq!(
            std::fs::read_to_string(
                container
                    .skills_dir
                    .as_ref()
                    .expect("skills dir")
                    .join("review-rust/root.md"),
            )
            .expect("live skill"),
            "Version two."
        );
    }

    #[tokio::test]
    async fn managed_install_rejects_cross_pack_takeover_before_live_mutation() {
        let (temp, _config, container) = setup().await;
        let first_root = temp.path().join("pack-a");
        write_skill(
            &first_root.join("review-rust"),
            "review-rust",
            "Owned by A.",
        );
        CapabilityPackService::install(
            &container,
            &pack("pack-a", "1.0.0", first_root),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("initial install");
        let takeover_root = temp.path().join("pack-b");
        write_skill(
            &takeover_root.join("review-rust"),
            "review-rust",
            "Owned by B.",
        );

        let error = CapabilityPackService::install(
            &container,
            &pack("pack-b", "1.0.0", takeover_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect_err("cross-pack takeover");

        assert!(matches!(
            error,
            crate::capability_pack::CapabilityPackInstallError::Ownership { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(
                container
                    .skills_dir
                    .as_ref()
                    .expect("skills dir")
                    .join("review-rust/root.md"),
            )
            .expect("live skill"),
            "Owned by A."
        );
    }

    #[tokio::test]
    async fn startup_completes_a_durable_managed_commit_decision() {
        let (temp, config, container) = setup().await;
        let pack_root = temp.path().join("commit-decided-pack");
        write_skill(
            &pack_root.join("review-rust"),
            "review-rust",
            "Committed after restart.",
        );
        let pack = pack("rust-team", "1.0.0", pack_root);
        let backend = CapabilityPackOwnerBackend::new(&container);
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let mut pending = DurableCapabilityPackInstaller::install_pending(
            &backend,
            &journal,
            &pack,
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("pending install");
        DurableCapabilityPackInstaller::decide_managed_commit(&backend, &journal, &mut pending)
            .await
            .expect("commit decision");
        let transaction_id = pending.record().id.clone();
        drop(container);

        let reopened = ServiceContainer::from_config(&config)
            .await
            .expect("startup commit reconciliation");
        let record = journal.load(&transaction_id).expect("transaction record");
        assert_eq!(record.status, CapabilityPackTransactionStatus::Committed);
        let ownership = CapabilityPackOwnershipStore::new(
            reopened.data_dir.join("capability-packs/ownership.json"),
        )
        .load()
        .expect("ownership index");
        assert_eq!(
            ownership
                .packs
                .get("rust-team")
                .expect("installed pack")
                .current_transaction_id,
            transaction_id
        );
    }

    #[tokio::test]
    async fn rollback_and_remove_unwind_versions_to_the_pre_pack_resource() {
        let (temp, _config, container) = setup().await;
        let live_skill = container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust");
        write_skill(&live_skill, "review-rust", "User-owned original.");
        let first_root = temp.path().join("pack-v1");
        write_skill(
            &first_root.join("review-rust"),
            "review-rust",
            "Version one.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.0.0", first_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("initial install");
        let second_root = temp.path().join("pack-v2");
        write_skill(
            &second_root.join("review-rust"),
            "review-rust",
            "Version two.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.1.0", second_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("update");

        let rollback = CapabilityPackService::rollback(&container, "rust-team")
            .await
            .expect("rollback");
        assert_eq!(rollback.removed_version, "1.1.0");
        assert_eq!(rollback.restored_version.as_deref(), Some("1.0.0"));
        assert_eq!(
            std::fs::read_to_string(live_skill.join("root.md")).expect("rolled back skill"),
            "Version one."
        );

        let removal = CapabilityPackService::remove(&container, "rust-team")
            .await
            .expect("remove pack");
        assert_eq!(removal.removed_versions, vec!["1.0.0"]);
        assert_eq!(
            std::fs::read_to_string(live_skill.join("root.md")).expect("restored user skill"),
            "User-owned original."
        );
        let ownership = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        )
        .load()
        .expect("ownership index");
        assert!(!ownership.packs.contains_key("rust-team"));
        assert!(!ownership.resources.contains_key("skill:review-rust"));
    }

    #[tokio::test]
    async fn startup_finishes_interrupted_managed_rollback_and_repairs_ownership() {
        let (temp, config, container) = setup().await;
        let live_skill = container
            .skills_dir
            .as_ref()
            .expect("skills dir")
            .join("review-rust");
        write_skill(&live_skill, "review-rust", "User-owned original.");
        let pack_root = temp.path().join("installed-pack");
        write_skill(
            &pack_root.join("review-rust"),
            "review-rust",
            "Pack-owned version.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.0.0", pack_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("install pack");
        let ownership_store = CapabilityPackOwnershipStore::new(
            container.data_dir.join("capability-packs/ownership.json"),
        );
        let transaction_id = ownership_store
            .load()
            .expect("ownership")
            .packs
            .get("rust-team")
            .expect("installed pack")
            .current_transaction_id
            .clone();
        let journal = CapabilityPackTransactionJournal::new(
            container.data_dir.join("capability-packs/transactions"),
        );
        let mut record = journal.load(&transaction_id).expect("transaction");
        record.status = CapabilityPackTransactionStatus::RollingBack;
        journal.save(&record).expect("rollback decision");
        drop(container);

        let reopened = ServiceContainer::from_config(&config)
            .await
            .expect("startup rollback recovery");

        assert_eq!(
            std::fs::read_to_string(live_skill.join("root.md")).expect("restored skill"),
            "User-owned original."
        );
        assert!(!CapabilityPackOwnershipStore::new(
            reopened.data_dir.join("capability-packs/ownership.json")
        )
        .load()
        .expect("repaired ownership")
        .packs
        .contains_key("rust-team"));
        assert_eq!(
            journal.load(&transaction_id).expect("transaction").status,
            CapabilityPackTransactionStatus::RolledBack
        );
    }

    #[tokio::test]
    async fn startup_rebuilds_missing_ownership_history_in_version_order() {
        let (temp, config, container) = setup().await;
        let first_root = temp.path().join("pack-v1");
        write_skill(
            &first_root.join("review-rust"),
            "review-rust",
            "Version one.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.0.0", first_root),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("initial install");
        let second_root = temp.path().join("pack-v2");
        write_skill(
            &second_root.join("review-rust"),
            "review-rust",
            "Version two.",
        );
        CapabilityPackService::install(
            &container,
            &pack("rust-team", "1.1.0", second_root),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("update");
        std::fs::remove_file(container.data_dir.join("capability-packs/ownership.json"))
            .expect("remove ownership index");
        drop(container);

        let reopened = ServiceContainer::from_config(&config)
            .await
            .expect("ownership rebuild");
        let ownership = CapabilityPackOwnershipStore::new(
            reopened.data_dir.join("capability-packs/ownership.json"),
        )
        .load()
        .expect("ownership index");
        let installed = ownership.packs.get("rust-team").expect("installed pack");
        assert_eq!(installed.current_version, "1.1.0");
        assert_eq!(installed.versions.len(), 2);
    }

    #[tokio::test]
    async fn bypass_permissions_cannot_activate_an_untrusted_workspace() {
        let (temp, config, container) = setup().await;
        CapabilityPackService::install(
            &container,
            &executable_pack("exec-pack", temp.path().join("exec-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let session_id = SessionId("activation-session".into());
        container
            .session_state
            .session_permission_modes
            .write()
            .await
            .insert(
                session_id.clone(),
                y_core::permission_types::PermissionMode::BypassPermissions,
            );
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );

        let error = CapabilityPackService::grant_activation(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
            &session_id,
            None,
            None,
        )
        .await
        .expect_err("untrusted workspace");

        assert!(matches!(
            error,
            CapabilityPackInstallError::Activation { .. }
        ));
        assert!(container
            .session_state
            .pending_permissions
            .lock()
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn trusted_workspace_activation_requires_hitl_before_grant_persistence() {
        let (temp, config, container) = setup().await;
        CapabilityPackService::install(
            &container,
            &executable_pack("exec-pack", temp.path().join("exec-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        let session_id = SessionId("activation-session".into());
        let (progress, mut events) = TurnEventSender::channel();

        let activation = CapabilityPackService::grant_activation(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
            &session_id,
            Some(&progress),
            None,
        );
        let responder = async {
            let (event, _) = events.recv().await.expect("permission event");
            let TurnEvent::PermissionRequest {
                request_id,
                tool_name,
                ..
            } = event
            else {
                panic!("expected permission request");
            };
            assert_eq!(tool_name, "CapabilityPackActivate");
            let pending = container
                .session_state
                .pending_permissions
                .lock()
                .await
                .remove(&request_id)
                .expect("pending activation approval");
            pending
                .send(PermissionPromptResponse::Approve)
                .expect("approve activation");
        };
        let (receipt, ()) = tokio::join!(activation, responder);
        let receipt = receipt.expect("activation grant");

        assert_eq!(receipt.grant.pack_id, "exec-pack");
        assert_eq!(receipt.executable_resources, vec!["mcp:pack-mcp"]);
        assert_eq!(container.mcp_manager.connected_count().await, 0);
        assert_eq!(
            CapabilityPackActivationStore::new(
                container
                    .data_dir
                    .join("capability-packs/activation-grants.json")
            )
            .grants()
            .expect("activation grants")
            .len(),
            1
        );
        CapabilityPackService::rollback(&container, "exec-pack")
            .await
            .expect("rollback executable pack");
        assert!(CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json")
        )
        .grants()
        .expect("activation grants")
        .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn approved_mcp_grant_activates_and_explicit_revocation_stops_owner() {
        let (temp, config, container) = setup().await;
        let container = Arc::new(container);
        CapabilityPackService::install(
            &container,
            &live_mcp_pack("exec-pack", temp.path().join("live-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;

        let live = CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("activate MCP owner");

        assert_eq!(live.activated_resources, vec!["mcp:pack-mcp"]);
        assert_eq!(container.mcp_manager.connected_count().await, 1);
        assert!(container
            .tool_registry
            .get_definition(&y_core::types::ToolName::from_string("mcp_pack-mcp_search"))
            .await
            .is_some());
        assert!(container
            .prompt_context
            .read()
            .await
            .mcp_server_instructions
            .as_deref()
            .is_some_and(|text| text.contains("Pack MCP instructions")));

        let revoked = CapabilityPackService::revoke_activation(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("revoke MCP owner");

        assert_eq!(revoked.deactivated_resources, vec!["mcp:pack-mcp"]);
        assert_eq!(container.mcp_manager.connected_count().await, 0);
        assert!(container
            .tool_registry
            .get_definition(&y_core::types::ToolName::from_string("mcp_pack-mcp_search"))
            .await
            .is_none());
        assert!(CapabilityPackActivationStore::new(
            container
                .data_dir
                .join("capability-packs/activation-grants.json")
        )
        .grants()
        .expect("activation grants")
        .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn long_lived_startup_reconciles_a_still_trusted_mcp_grant() {
        let (temp, config, container) = setup().await;
        CapabilityPackService::install(
            &container,
            &live_mcp_pack("exec-pack", temp.path().join("live-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;
        drop(container);

        let reopened = Arc::new(
            ServiceContainer::from_config(&config)
                .await
                .expect("reopen service container"),
        );
        reopened.start_background_services().await;

        assert_eq!(reopened.mcp_manager.connected_count().await, 1);
        assert!(reopened
            .tool_registry
            .get_definition(&y_core::types::ToolName::from_string("mcp_pack-mcp_search"))
            .await
            .is_some());
        reopened.mcp_manager.close_all().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pack_update_stops_old_mcp_owner_and_invalidates_old_grant() {
        let (temp, config, container) = setup().await;
        let container = Arc::new(container);
        CapabilityPackService::install(
            &container,
            &live_mcp_pack("exec-pack", temp.path().join("live-pack-v1")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install first declaration");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;
        CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("activate first version");

        CapabilityPackService::install(
            &container,
            &live_mcp_pack_version("exec-pack", "1.1.0", temp.path().join("live-pack-v2")),
            CapabilityPackInstallOptions {
                allow_replacements: true,
            },
        )
        .await
        .expect("update pack");

        assert_eq!(container.mcp_manager.connected_count().await, 0);
        assert!(container
            .tool_registry
            .get_definition(&y_core::types::ToolName::from_string("mcp_pack-mcp_search"))
            .await
            .is_none());
        assert!(activation_store(&container)
            .grants()
            .expect("activation grants")
            .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rollback_stops_the_current_mcp_owner_before_restoring_snapshots() {
        let (temp, config, container) = setup().await;
        let container = Arc::new(container);
        CapabilityPackService::install(
            &container,
            &live_mcp_pack("exec-pack", temp.path().join("live-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;
        CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("activate MCP owner");

        CapabilityPackService::rollback(&container, "exec-pack")
            .await
            .expect("rollback pack");

        assert_eq!(container.mcp_manager.connected_count().await, 0);
        assert!(container
            .tool_registry
            .get_definition(&y_core::types::ToolName::from_string("mcp_pack-mcp_search"))
            .await
            .is_none());
        assert!(activation_store(&container)
            .grants()
            .expect("activation grants")
            .is_empty());
        assert!(!container
            .data_dir
            .join("capability-packs/declarations/mcp/pack-mcp.toml")
            .exists());
    }

    #[tokio::test]
    async fn startup_revokes_grant_after_workspace_becomes_untrusted() {
        let (temp, config, container) = setup().await;
        CapabilityPackService::install(
            &container,
            &executable_pack("exec-pack", temp.path().join("exec-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;
        workspace_service
            .untrust_workspace(&workspace)
            .expect("untrust workspace");
        drop(container);

        let reopened = Arc::new(
            ServiceContainer::from_config(&config)
                .await
                .expect("reopen service container"),
        );
        reopened.start_background_services().await;

        assert!(activation_store(&reopened)
            .grants()
            .expect("activation grants")
            .is_empty());
        assert_eq!(reopened.mcp_manager.connected_count().await, 0);
    }

    #[tokio::test]
    async fn pack_mcp_cannot_take_over_a_user_configured_server_name() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path();
        let config_dir = root.join("config");
        std::fs::create_dir_all(config_dir.join("prompts")).expect("prompts dir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: root.join("state.db").display().to_string(),
            pool_size: 1,
            wal_enabled: true,
            transcript_dir: root.join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        config.prompts_dir = Some(config_dir.join("prompts"));
        config.skills_dir = Some(config_dir.join("skills"));
        config.tools.mcp_servers.push(y_tools::McpServerConfig {
            name: "pack-mcp".to_string(),
            transport: "stdio".to_string(),
            command: Some("user-owned-server".to_string()),
            args: Vec::new(),
            url: None,
            env: std::collections::HashMap::new(),
            enabled: true,
            headers: std::collections::HashMap::new(),
            startup_timeout_secs: 30,
            tool_timeout_secs: 120,
            cwd: None,
            bearer_token: None,
            enabled_tools: None,
            disabled_tools: None,
            auto_reconnect: true,
            max_reconnect_attempts: 5,
        });
        let container = Arc::new(
            ServiceContainer::from_config(&config)
                .await
                .expect("service container"),
        );
        CapabilityPackService::install(
            &container,
            &executable_pack("exec-pack", root.join("exec-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive declarations");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(&config_dir);
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;

        let error = CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect_err("static server ownership must win");

        assert!(error
            .to_string()
            .contains("conflicts with user configuration"));
        assert_eq!(container.mcp_manager.connected_count().await, 0);
    }

    #[tokio::test]
    async fn hook_activation_overlays_and_preserves_reloaded_user_configuration() {
        let (temp, config, container) = setup_with_hooks(hook_config("pre_llm_call")).await;
        let container = Arc::new(container);
        CapabilityPackService::install(
            &container,
            &hook_pack("exec-pack", temp.path().join("hook-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive hook");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(
            config
                .prompts_dir
                .as_ref()
                .expect("prompts dir")
                .parent()
                .expect("config dir"),
        );
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;

        let live = CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("activate hook owner");
        assert_eq!(live.activated_resources, vec!["hook:audit-hook"]);
        {
            let hooks = container.hook_system.read().expect("hook system");
            let executor = hooks.handler_executor().expect("handler executor");
            assert!(executor.has_handlers(y_core::hook::HookPoint::PreLlmCall));
            assert!(executor.has_handlers(y_core::hook::HookPoint::PostToolExecute));
        }

        container.reload_hooks(&hook_config("session_created"));
        {
            let hooks = container.hook_system.read().expect("hook system");
            let executor = hooks.handler_executor().expect("handler executor");
            assert!(executor.has_handlers(y_core::hook::HookPoint::SessionCreated));
            assert!(executor.has_handlers(y_core::hook::HookPoint::PostToolExecute));
            assert!(!executor.has_handlers(y_core::hook::HookPoint::PreLlmCall));
        }

        let revoked = CapabilityPackService::revoke_activation(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("revoke hook owner");
        assert_eq!(revoked.deactivated_resources, vec!["hook:audit-hook"]);
        let hooks = container.hook_system.read().expect("hook system");
        let executor = hooks.handler_executor().expect("base handler executor");
        assert!(executor.has_handlers(y_core::hook::HookPoint::SessionCreated));
        assert!(!executor.has_handlers(y_core::hook::HookPoint::PostToolExecute));
    }

    #[cfg(feature = "lsp")]
    #[tokio::test]
    async fn lsp_activation_adds_and_revokes_only_the_pack_server_overlay() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path();
        let config_dir = root.join("config");
        std::fs::create_dir_all(config_dir.join("prompts")).expect("prompts dir");
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: root.join("state.db").display().to_string(),
            pool_size: 1,
            wal_enabled: true,
            transcript_dir: root.join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        config.prompts_dir = Some(config_dir.join("prompts"));
        config.skills_dir = Some(config_dir.join("skills"));
        config.lsp.enabled = true;
        let container = Arc::new(
            ServiceContainer::from_config(&config)
                .await
                .expect("service container"),
        );
        CapabilityPackService::install(
            &container,
            &lsp_pack("exec-pack", root.join("lsp-pack")),
            CapabilityPackInstallOptions::default(),
        )
        .await
        .expect("install inactive LSP declaration");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let workspace_service = WorkspaceService::new(&config_dir);
        workspace_service
            .trust_workspace(&workspace)
            .expect("trust workspace");
        approve_activation(&container, &workspace_service, &workspace).await;

        let live = CapabilityPackService::activate_granted(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("activate LSP owner");
        assert_eq!(live.activated_resources, vec!["lsp:pack-language"]);
        let manager = container.lsp_manager.as_ref().expect("LSP manager");
        assert!(manager.has_dynamic_server("pack-language"));
        assert!(manager.has_configured_server("rust"));

        let revoked = CapabilityPackService::revoke_activation(
            &container,
            &workspace_service,
            "exec-pack",
            &workspace,
        )
        .await
        .expect("revoke LSP owner");
        assert_eq!(revoked.deactivated_resources, vec!["lsp:pack-language"]);
        assert!(!manager.has_dynamic_server("pack-language"));
        assert!(manager.has_configured_server("rust"));
    }
}
