//! Local Capability Pack manifest and staging validation.

mod activation;
mod durable;
mod journal;
mod live;
mod manifest;
mod owner;
mod ownership;
mod service;
mod transaction;
mod validator;

pub use activation::{
    CapabilityPackActivationGrant, CapabilityPackActivationReceipt,
    CapabilityPackActivationRevocationReceipt, CapabilityPackLiveActivationReceipt,
};
pub use durable::DurableCapabilityPackInstaller;
pub use journal::{
    CapabilityPackJournalError, CapabilityPackTransactionJournal, CapabilityPackTransactionRecord,
    CapabilityPackTransactionResource, CapabilityPackTransactionState,
    CapabilityPackTransactionStatus,
};
pub use manifest::{
    CapabilityPackManifest, CapabilityPackMetadata, CapabilityResourceDeclaration,
    CapabilityResourceKind,
};
pub use service::{
    CapabilityPackRemoveReceipt, CapabilityPackRollbackReceipt, CapabilityPackService,
};
pub use transaction::{
    CapabilityPackChange, CapabilityPackChangeKind, CapabilityPackInstallError,
    CapabilityPackInstallOptions, CapabilityPackInstallReceipt, CapabilityPackInstaller,
    CapabilityPackPreview, DeclarativeCapabilityBackend,
};
pub use validator::{
    CapabilityPackIssue, CapabilityPackIssueCode, CapabilityPackProvenance,
    CapabilityPackSourceKind, CapabilityPackValidationReport, CapabilityPackValidator,
    ValidatedCapabilityPack, ValidatedCapabilityResource,
};

pub(crate) use live::compose_hook_config;
