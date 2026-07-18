use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::manifest::CapabilityResourceKind;
use super::validator::{ValidatedCapabilityPack, ValidatedCapabilityResource};

/// Dry-run classification for one validated resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPackChangeKind {
    Add,
    Replace,
    Unchanged,
}

/// One deterministic dry-run change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackChange {
    pub resource_kind: CapabilityResourceKind,
    pub resource_id: String,
    pub change: CapabilityPackChangeKind,
    pub requires_activation: bool,
    pub current_sha256: Option<String>,
    pub desired_sha256: String,
}

/// Side-effect-free change set shown before installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackPreview {
    pub pack_id: String,
    pub pack_version: String,
    pub can_apply: bool,
    pub changes: Vec<CapabilityPackChange>,
}

/// Explicit approvals that affect declarative installation only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilityPackInstallOptions {
    pub allow_replacements: bool,
}

/// Successful logical transaction result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackInstallReceipt {
    pub pack_id: String,
    pub pack_version: String,
    pub applied: Vec<CapabilityPackChange>,
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityPackInstallError {
    #[error("failed to validate {resource}: {message}")]
    ResourceValidationFailed { resource: String, message: String },
    #[error("failed to inspect {resource}: {message}")]
    InspectionFailed { resource: String, message: String },
    #[error("replacement approval is required for: {resources:?}")]
    ReplacementApprovalRequired { resources: Vec<String> },
    #[error("failed to snapshot {resource}: {message}")]
    SnapshotFailed { resource: String, message: String },
    #[error("failed to apply {resource}: {message}")]
    ApplyFailed { resource: String, message: String },
    #[error(
        "installation failed at {failed_resource}: {apply_error}; compensation also failed: {compensation_errors:?}"
    )]
    CompensationFailed {
        failed_resource: String,
        apply_error: String,
        compensation_errors: Vec<String>,
    },
    #[error("capability-pack transaction journal failure: {message}")]
    Journal { message: String },
    #[error("capability-pack ownership failure: {message}")]
    Ownership { message: String },
    #[error("capability-pack activation failure: {message}")]
    Activation { message: String },
    #[error("failed to recover transaction {transaction_id}: {errors:?}")]
    RecoveryFailed {
        transaction_id: String,
        errors: Vec<String>,
    },
}

/// Existing capability owner adapter used by the compensation engine.
#[async_trait]
pub trait DeclarativeCapabilityBackend: Sync {
    type Snapshot: Send;

    async fn validate(&self, _resource: &ValidatedCapabilityResource) -> Result<(), String> {
        Ok(())
    }

    async fn current_hash(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Option<String>, String>;

    async fn snapshot(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Self::Snapshot, String>;

    async fn apply(&self, resource: &ValidatedCapabilityResource) -> Result<(), String>;

    async fn restore(
        &self,
        resource: &ValidatedCapabilityResource,
        snapshot: Self::Snapshot,
    ) -> Result<(), String>;
}

pub struct CapabilityPackInstaller;

impl CapabilityPackInstaller {
    pub async fn preview<B: DeclarativeCapabilityBackend>(
        backend: &B,
        pack: &ValidatedCapabilityPack,
    ) -> Result<CapabilityPackPreview, CapabilityPackInstallError> {
        let mut resources = pack.resources.iter().collect::<Vec<_>>();
        resources.sort_by(|left, right| resource_order(left, right));
        let mut changes = Vec::with_capacity(resources.len());
        for resource in resources {
            backend.validate(resource).await.map_err(|message| {
                CapabilityPackInstallError::ResourceValidationFailed {
                    resource: resource_label(resource),
                    message,
                }
            })?;
            let current = backend.current_hash(resource).await.map_err(|message| {
                CapabilityPackInstallError::InspectionFailed {
                    resource: resource_label(resource),
                    message,
                }
            })?;
            let change = match current.as_deref() {
                None => CapabilityPackChangeKind::Add,
                Some(hash) if hash == resource.sha256 => CapabilityPackChangeKind::Unchanged,
                Some(_) => CapabilityPackChangeKind::Replace,
            };
            changes.push(CapabilityPackChange {
                resource_kind: resource.kind,
                resource_id: resource.id.clone(),
                change,
                requires_activation: is_executable(resource.kind),
                current_sha256: current,
                desired_sha256: resource.sha256.clone(),
            });
        }
        Ok(CapabilityPackPreview {
            pack_id: pack.id.clone(),
            pack_version: pack.version.clone(),
            can_apply: true,
            changes,
        })
    }

    pub async fn install<B: DeclarativeCapabilityBackend>(
        backend: &B,
        pack: &ValidatedCapabilityPack,
        options: CapabilityPackInstallOptions,
    ) -> Result<CapabilityPackInstallReceipt, CapabilityPackInstallError> {
        let preview = Self::preview(backend, pack).await?;
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

        let mut resources = pack
            .resources
            .iter()
            .filter(|resource| {
                preview.changes.iter().any(|change| {
                    change.resource_kind == resource.kind
                        && change.resource_id == resource.id
                        && matches!(
                            change.change,
                            CapabilityPackChangeKind::Add | CapabilityPackChangeKind::Replace
                        )
                })
            })
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| resource_order(left, right));

        let mut snapshots = Vec::new();
        for resource in resources {
            let snapshot = match backend.snapshot(resource).await {
                Ok(snapshot) => snapshot,
                Err(message) => {
                    let compensation_errors = compensate(backend, snapshots).await;
                    if compensation_errors.is_empty() {
                        return Err(CapabilityPackInstallError::SnapshotFailed {
                            resource: resource_label(resource),
                            message,
                        });
                    }
                    return Err(CapabilityPackInstallError::CompensationFailed {
                        failed_resource: resource_label(resource),
                        apply_error: format!("snapshot failed: {message}"),
                        compensation_errors,
                    });
                }
            };
            snapshots.push((resource, snapshot));
            if let Err(message) = backend.apply(resource).await {
                let compensation_errors = compensate(backend, snapshots).await;
                if compensation_errors.is_empty() {
                    return Err(CapabilityPackInstallError::ApplyFailed {
                        resource: resource_label(resource),
                        message,
                    });
                }
                return Err(CapabilityPackInstallError::CompensationFailed {
                    failed_resource: resource_label(resource),
                    apply_error: message,
                    compensation_errors,
                });
            }
        }

        let applied = preview
            .changes
            .into_iter()
            .filter(|change| {
                matches!(
                    change.change,
                    CapabilityPackChangeKind::Add | CapabilityPackChangeKind::Replace
                )
            })
            .collect();
        Ok(CapabilityPackInstallReceipt {
            pack_id: preview.pack_id,
            pack_version: preview.pack_version,
            applied,
        })
    }
}

async fn compensate<B: DeclarativeCapabilityBackend>(
    backend: &B,
    snapshots: Vec<(&ValidatedCapabilityResource, B::Snapshot)>,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (resource, snapshot) in snapshots.into_iter().rev() {
        if let Err(error) = backend.restore(resource, snapshot).await {
            errors.push(format!("{}: {error}", resource_label(resource)));
        }
    }
    errors
}

pub(crate) fn is_executable(kind: CapabilityResourceKind) -> bool {
    matches!(
        kind,
        CapabilityResourceKind::Mcp | CapabilityResourceKind::Hook | CapabilityResourceKind::Lsp
    )
}

pub(crate) fn resource_order(
    left: &ValidatedCapabilityResource,
    right: &ValidatedCapabilityResource,
) -> std::cmp::Ordering {
    left.kind
        .as_str()
        .cmp(right.kind.as_str())
        .then_with(|| left.id.cmp(&right.id))
}

pub(crate) fn resource_label(resource: &ValidatedCapabilityResource) -> String {
    format!("{}:{}", resource.kind.as_str(), resource.id)
}

pub(crate) fn change_label(change: &CapabilityPackChange) -> String {
    format!("{}:{}", change.resource_kind.as_str(), change.resource_id)
}
