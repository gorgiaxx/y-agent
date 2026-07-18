use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::validator::{
    CapabilityPackProvenance, ValidatedCapabilityPack, ValidatedCapabilityResource,
};

const TRANSACTION_FILE: &str = "transaction.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPackTransactionStatus {
    Prepared,
    Applying,
    AwaitingCommit,
    CommitDecided,
    RollingBack,
    Committed,
    RolledBack,
    CompensationFailed,
}

impl CapabilityPackTransactionStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::RolledBack | Self::CompensationFailed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPackTransactionState {
    Pending,
    Snapshotted,
    Applied,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackTransactionResource {
    pub resource: ValidatedCapabilityResource,
    pub state: CapabilityPackTransactionState,
    pub snapshot: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackTransactionRecord {
    pub id: String,
    pub pack_id: String,
    pub pack_version: String,
    pub provenance: CapabilityPackProvenance,
    pub status: CapabilityPackTransactionStatus,
    pub resources: Vec<CapabilityPackTransactionResource>,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub ownership_managed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityPackJournalError {
    #[error("invalid transaction id: {0}")]
    InvalidTransactionId(String),
    #[error("capability-pack journal I/O failure at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("capability-pack journal parse failure at {path}: {message}")]
    Parse { path: PathBuf, message: String },
}

#[derive(Debug, Clone)]
pub struct CapabilityPackTransactionJournal {
    root: PathBuf,
}

impl CapabilityPackTransactionJournal {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn begin(
        &self,
        pack: &ValidatedCapabilityPack,
    ) -> Result<CapabilityPackTransactionRecord, CapabilityPackJournalError> {
        let mut resources = pack.resources.clone();
        resources.sort_by(super::transaction::resource_order);
        let record = CapabilityPackTransactionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            pack_id: pack.id.clone(),
            pack_version: pack.version.clone(),
            provenance: pack.provenance.clone(),
            status: CapabilityPackTransactionStatus::Prepared,
            resources: resources
                .into_iter()
                .map(|resource| CapabilityPackTransactionResource {
                    resource,
                    state: CapabilityPackTransactionState::Pending,
                    snapshot: None,
                })
                .collect(),
            errors: Vec::new(),
            ownership_managed: false,
        };
        self.save(&record)?;
        Ok(record)
    }

    pub fn save(
        &self,
        record: &CapabilityPackTransactionRecord,
    ) -> Result<(), CapabilityPackJournalError> {
        validate_id(&record.id)?;
        let directory = self.root.join(&record.id);
        std::fs::create_dir_all(&directory).map_err(|error| io_error(&directory, &error))?;
        let target = directory.join(TRANSACTION_FILE);
        let temporary = directory.join(format!(".{TRANSACTION_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
            CapabilityPackJournalError::Parse {
                path: target.clone(),
                message: error.to_string(),
            }
        })?;
        let mut file =
            std::fs::File::create(&temporary).map_err(|error| io_error(&temporary, &error))?;
        file.write_all(&bytes)
            .map_err(|error| io_error(&temporary, &error))?;
        file.write_all(b"\n")
            .map_err(|error| io_error(&temporary, &error))?;
        file.sync_all()
            .map_err(|error| io_error(&temporary, &error))?;
        std::fs::rename(&temporary, &target).map_err(|error| io_error(&target, &error))?;
        sync_directory(&directory)?;
        Ok(())
    }

    pub fn load(
        &self,
        transaction_id: &str,
    ) -> Result<CapabilityPackTransactionRecord, CapabilityPackJournalError> {
        validate_id(transaction_id)?;
        let path = self.root.join(transaction_id).join(TRANSACTION_FILE);
        load_record(&path)
    }

    pub fn load_all(
        &self,
    ) -> Result<Vec<CapabilityPackTransactionRecord>, CapabilityPackJournalError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let entries =
            std::fs::read_dir(&self.root).map_err(|error| io_error(&self.root, &error))?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error(&self.root, &error))?;
            if entry
                .file_type()
                .map_err(|error| io_error(&entry.path(), &error))?
                .is_dir()
            {
                paths.push(entry.path().join(TRANSACTION_FILE));
            }
        }
        paths.sort();
        paths
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| load_record(&path))
            .collect()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn load_record(path: &Path) -> Result<CapabilityPackTransactionRecord, CapabilityPackJournalError> {
    let bytes = std::fs::read(path).map_err(|error| io_error(path, &error))?;
    let record: CapabilityPackTransactionRecord =
        serde_json::from_slice(&bytes).map_err(|error| CapabilityPackJournalError::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    validate_id(&record.id)?;
    Ok(record)
}

fn validate_id(transaction_id: &str) -> Result<(), CapabilityPackJournalError> {
    uuid::Uuid::parse_str(transaction_id)
        .map(|_| ())
        .map_err(|_| CapabilityPackJournalError::InvalidTransactionId(transaction_id.to_string()))
}

fn sync_directory(path: &Path) -> Result<(), CapabilityPackJournalError> {
    let directory = std::fs::File::open(path).map_err(|error| io_error(path, &error))?;
    directory.sync_all().map_err(|error| io_error(path, &error))
}

fn io_error(path: &Path, error: &std::io::Error) -> CapabilityPackJournalError {
    CapabilityPackJournalError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
