use serde::de::DeserializeOwned;
use serde::Serialize;

use super::journal::{
    CapabilityPackTransactionJournal, CapabilityPackTransactionRecord,
    CapabilityPackTransactionState, CapabilityPackTransactionStatus,
};
use super::transaction::{
    change_label, resource_label, CapabilityPackChangeKind, CapabilityPackInstallError,
    CapabilityPackInstallOptions, CapabilityPackInstallReceipt, CapabilityPackInstaller,
    DeclarativeCapabilityBackend,
};
use super::validator::ValidatedCapabilityPack;

pub struct DurableCapabilityPackInstaller;

pub(crate) struct PendingCapabilityPackInstall {
    receipt: CapabilityPackInstallReceipt,
    record: CapabilityPackTransactionRecord,
}

impl PendingCapabilityPackInstall {
    pub(crate) fn record(&self) -> &CapabilityPackTransactionRecord {
        &self.record
    }

    pub(crate) fn into_receipt(self) -> CapabilityPackInstallReceipt {
        self.receipt
    }
}

impl DurableCapabilityPackInstaller {
    pub async fn install<B>(
        backend: &B,
        journal: &CapabilityPackTransactionJournal,
        pack: &ValidatedCapabilityPack,
        options: CapabilityPackInstallOptions,
    ) -> Result<CapabilityPackInstallReceipt, CapabilityPackInstallError>
    where
        B: DeclarativeCapabilityBackend,
        B::Snapshot: Serialize + DeserializeOwned,
    {
        let mut pending = Self::install_pending(backend, journal, pack, options).await?;
        pending.record.status = CapabilityPackTransactionStatus::Committed;
        if let Err(error) = journal.save(&pending.record) {
            return fail_and_rollback(
                backend,
                journal,
                &mut pending.record,
                format!("pack {}@{}", pack.id, pack.version),
                error.to_string(),
                FailureKind::Journal,
            )
            .await;
        }
        Ok(pending.receipt)
    }

    pub(crate) async fn install_pending<B>(
        backend: &B,
        journal: &CapabilityPackTransactionJournal,
        pack: &ValidatedCapabilityPack,
        options: CapabilityPackInstallOptions,
    ) -> Result<PendingCapabilityPackInstall, CapabilityPackInstallError>
    where
        B: DeclarativeCapabilityBackend,
        B::Snapshot: Serialize + DeserializeOwned,
    {
        let preview = CapabilityPackInstaller::preview(backend, pack).await?;
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

        let mut record = journal.begin(pack).map_err(|error| journal_error(&error))?;
        record.status = CapabilityPackTransactionStatus::Applying;
        journal
            .save(&record)
            .map_err(|error| journal_error(&error))?;

        for index in 0..record.resources.len() {
            let should_apply = preview.changes.iter().any(|change| {
                let resource = &record.resources[index].resource;
                change.resource_kind == resource.kind
                    && change.resource_id == resource.id
                    && matches!(
                        change.change,
                        CapabilityPackChangeKind::Add | CapabilityPackChangeKind::Replace
                    )
            });
            if !should_apply {
                continue;
            }
            let resource = record.resources[index].resource.clone();
            let snapshot = match backend.snapshot(&resource).await {
                Ok(snapshot) => snapshot,
                Err(message) => {
                    return fail_and_rollback(
                        backend,
                        journal,
                        &mut record,
                        resource_label(&resource),
                        format!("snapshot failed: {message}"),
                        FailureKind::Snapshot,
                    )
                    .await;
                }
            };
            let snapshot = match serde_json::to_value(&snapshot) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return fail_and_rollback(
                        backend,
                        journal,
                        &mut record,
                        resource_label(&resource),
                        format!("failed to serialize snapshot: {error}"),
                        FailureKind::Journal,
                    )
                    .await;
                }
            };
            record.resources[index].snapshot = Some(snapshot);
            record.resources[index].state = CapabilityPackTransactionState::Snapshotted;
            if let Err(error) = journal.save(&record) {
                return fail_and_rollback(
                    backend,
                    journal,
                    &mut record,
                    resource_label(&resource),
                    error.to_string(),
                    FailureKind::Journal,
                )
                .await;
            }

            if let Err(message) = backend.apply(&resource).await {
                return fail_and_rollback(
                    backend,
                    journal,
                    &mut record,
                    resource_label(&resource),
                    message,
                    FailureKind::Apply,
                )
                .await;
            }
            record.resources[index].state = CapabilityPackTransactionState::Applied;
            if let Err(error) = journal.save(&record) {
                return fail_and_rollback(
                    backend,
                    journal,
                    &mut record,
                    resource_label(&resource),
                    error.to_string(),
                    FailureKind::Journal,
                )
                .await;
            }
        }

        record.status = CapabilityPackTransactionStatus::AwaitingCommit;
        if let Err(error) = journal.save(&record) {
            return fail_and_rollback(
                backend,
                journal,
                &mut record,
                format!("pack {}@{}", pack.id, pack.version),
                error.to_string(),
                FailureKind::Journal,
            )
            .await;
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
        Ok(PendingCapabilityPackInstall {
            receipt: CapabilityPackInstallReceipt {
                pack_id: preview.pack_id,
                pack_version: preview.pack_version,
                applied,
            },
            record,
        })
    }

    pub(crate) async fn decide_managed_commit<B>(
        backend: &B,
        journal: &CapabilityPackTransactionJournal,
        pending: &mut PendingCapabilityPackInstall,
    ) -> Result<(), CapabilityPackInstallError>
    where
        B: DeclarativeCapabilityBackend,
        B::Snapshot: Serialize + DeserializeOwned,
    {
        pending.record.ownership_managed = true;
        pending.record.status = CapabilityPackTransactionStatus::CommitDecided;
        if let Err(error) = journal.save(&pending.record) {
            let failed_resource = format!(
                "pack {}@{}",
                pending.record.pack_id, pending.record.pack_version
            );
            return fail_and_rollback(
                backend,
                journal,
                &mut pending.record,
                failed_resource,
                error.to_string(),
                FailureKind::Journal,
            )
            .await;
        }
        Ok(())
    }

    pub(crate) fn mark_committed(
        journal: &CapabilityPackTransactionJournal,
        pending: &mut PendingCapabilityPackInstall,
    ) -> Result<(), CapabilityPackInstallError> {
        pending.record.status = CapabilityPackTransactionStatus::Committed;
        journal
            .save(&pending.record)
            .map_err(|error| journal_error(&error))
    }

    pub(crate) fn mark_record_committed(
        journal: &CapabilityPackTransactionJournal,
        record: &mut CapabilityPackTransactionRecord,
    ) -> Result<(), CapabilityPackInstallError> {
        record.status = CapabilityPackTransactionStatus::Committed;
        journal.save(record).map_err(|error| journal_error(&error))
    }

    pub(crate) async fn rollback_managed<B>(
        backend: &B,
        journal: &CapabilityPackTransactionJournal,
        transaction_id: &str,
    ) -> Result<CapabilityPackTransactionRecord, CapabilityPackInstallError>
    where
        B: DeclarativeCapabilityBackend,
        B::Snapshot: Serialize + DeserializeOwned,
    {
        let mut record = journal
            .load(transaction_id)
            .map_err(|error| journal_error(&error))?;
        if !record.ownership_managed
            || !matches!(
                record.status,
                CapabilityPackTransactionStatus::Committed
                    | CapabilityPackTransactionStatus::RollingBack
            )
        {
            return Err(CapabilityPackInstallError::Ownership {
                message: format!("transaction {transaction_id} is not a current managed commit"),
            });
        }
        if record.status == CapabilityPackTransactionStatus::Committed {
            record.status = CapabilityPackTransactionStatus::RollingBack;
            journal
                .save(&record)
                .map_err(|error| journal_error(&error))?;
        }
        let errors = rollback_record(backend, journal, &mut record).await;
        if !errors.is_empty() {
            return Err(CapabilityPackInstallError::RecoveryFailed {
                transaction_id: transaction_id.to_string(),
                errors,
            });
        }
        Ok(record)
    }

    pub async fn recover<B>(
        backend: &B,
        journal: &CapabilityPackTransactionJournal,
    ) -> Result<Vec<String>, CapabilityPackInstallError>
    where
        B: DeclarativeCapabilityBackend,
        B::Snapshot: Serialize + DeserializeOwned,
    {
        let records = journal.load_all().map_err(|error| journal_error(&error))?;
        let mut recovered = Vec::new();
        for mut record in records {
            if record.status == CapabilityPackTransactionStatus::CompensationFailed {
                let errors = if record.errors.is_empty() {
                    vec!["manual intervention required".to_string()]
                } else {
                    record.errors
                };
                return Err(CapabilityPackInstallError::RecoveryFailed {
                    transaction_id: record.id,
                    errors,
                });
            }
            if record.status == CapabilityPackTransactionStatus::CommitDecided
                && record.ownership_managed
            {
                return Err(CapabilityPackInstallError::RecoveryFailed {
                    transaction_id: record.id,
                    errors: vec!["managed commit requires ownership reconciliation".to_string()],
                });
            }
            if record.status.is_terminal() {
                continue;
            }
            let transaction_id = record.id.clone();
            let errors = rollback_record(backend, journal, &mut record).await;
            if !errors.is_empty() {
                return Err(CapabilityPackInstallError::RecoveryFailed {
                    transaction_id,
                    errors,
                });
            }
            recovered.push(transaction_id);
        }
        Ok(recovered)
    }
}

#[derive(Debug, Clone, Copy)]
enum FailureKind {
    Snapshot,
    Apply,
    Journal,
}

async fn fail_and_rollback<B, T>(
    backend: &B,
    journal: &CapabilityPackTransactionJournal,
    record: &mut CapabilityPackTransactionRecord,
    failed_resource: String,
    failure: String,
    kind: FailureKind,
) -> Result<T, CapabilityPackInstallError>
where
    B: DeclarativeCapabilityBackend,
    B::Snapshot: Serialize + DeserializeOwned,
{
    record.errors.push(format!("{failed_resource}: {failure}"));
    let compensation_errors = rollback_record(backend, journal, record).await;
    if !compensation_errors.is_empty() {
        return Err(CapabilityPackInstallError::CompensationFailed {
            failed_resource,
            apply_error: failure,
            compensation_errors,
        });
    }
    match kind {
        FailureKind::Snapshot => Err(CapabilityPackInstallError::SnapshotFailed {
            resource: failed_resource,
            message: failure,
        }),
        FailureKind::Apply => Err(CapabilityPackInstallError::ApplyFailed {
            resource: failed_resource,
            message: failure,
        }),
        FailureKind::Journal => Err(CapabilityPackInstallError::Journal { message: failure }),
    }
}

async fn rollback_record<B>(
    backend: &B,
    journal: &CapabilityPackTransactionJournal,
    record: &mut CapabilityPackTransactionRecord,
) -> Vec<String>
where
    B: DeclarativeCapabilityBackend,
    B::Snapshot: Serialize + DeserializeOwned,
{
    let mut errors = Vec::new();
    record.status = CapabilityPackTransactionStatus::RollingBack;
    if let Err(error) = journal.save(record) {
        errors.push(error.to_string());
    }
    for index in (0..record.resources.len()).rev() {
        let Some(snapshot_value) = record.resources[index].snapshot.clone() else {
            continue;
        };
        let snapshot: B::Snapshot = match serde_json::from_value(snapshot_value) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                errors.push(format!(
                    "{} snapshot decode: {error}",
                    resource_label(&record.resources[index].resource)
                ));
                continue;
            }
        };
        let resource = record.resources[index].resource.clone();
        match backend.restore(&resource, snapshot).await {
            Ok(()) => {
                record.resources[index].state = CapabilityPackTransactionState::Restored;
                if let Err(error) = journal.save(record) {
                    errors.push(error.to_string());
                }
            }
            Err(error) => errors.push(format!("{}: {error}", resource_label(&resource))),
        }
    }
    record.status = if errors.is_empty() {
        CapabilityPackTransactionStatus::RolledBack
    } else {
        CapabilityPackTransactionStatus::CompensationFailed
    };
    record.errors.extend(errors.iter().cloned());
    if let Err(error) = journal.save(record) {
        errors.push(error.to_string());
    }
    errors
}

fn journal_error(error: &super::journal::CapabilityPackJournalError) -> CapabilityPackInstallError {
    CapabilityPackInstallError::Journal {
        message: error.to_string(),
    }
}
