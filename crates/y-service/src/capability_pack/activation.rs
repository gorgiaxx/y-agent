use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const ACTIVATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackActivationGrant {
    pub pack_id: String,
    pub pack_version: String,
    pub transaction_id: String,
    pub canonical_workspace: String,
    pub approved_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackActivationReceipt {
    pub grant: CapabilityPackActivationGrant,
    pub executable_resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackLiveActivationReceipt {
    pub grant: CapabilityPackActivationGrant,
    pub activated_resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPackActivationRevocationReceipt {
    pub pack_id: String,
    pub pack_version: String,
    pub canonical_workspace: String,
    pub deactivated_resources: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CapabilityPackActivationIndex {
    #[serde(default = "activation_schema_version")]
    schema_version: u32,
    #[serde(default)]
    grants: Vec<CapabilityPackActivationGrant>,
}

pub(crate) struct CapabilityPackActivationStore {
    path: PathBuf,
}

impl CapabilityPackActivationStore {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn grant(
        &self,
        grant: CapabilityPackActivationGrant,
    ) -> Result<CapabilityPackActivationGrant, String> {
        let mut index = self.load()?;
        index.grants.retain(|existing| {
            !(existing.transaction_id == grant.transaction_id
                && existing.canonical_workspace == grant.canonical_workspace)
        });
        index.grants.push(grant.clone());
        index.grants.sort_by(|left, right| {
            left.pack_id
                .cmp(&right.pack_id)
                .then_with(|| left.pack_version.cmp(&right.pack_version))
                .then_with(|| left.canonical_workspace.cmp(&right.canonical_workspace))
        });
        self.save(&index)?;
        Ok(grant)
    }

    pub(crate) fn revoke_transaction(&self, transaction_id: &str) -> Result<(), String> {
        let mut index = self.load()?;
        let original_len = index.grants.len();
        index
            .grants
            .retain(|grant| grant.transaction_id != transaction_id);
        if index.grants.len() != original_len {
            self.save(&index)?;
        }
        Ok(())
    }

    pub(crate) fn grants(&self) -> Result<Vec<CapabilityPackActivationGrant>, String> {
        self.load().map(|index| index.grants)
    }

    pub(crate) fn revoke_grant(
        &self,
        transaction_id: &str,
        canonical_workspace: &str,
    ) -> Result<bool, String> {
        let mut index = self.load()?;
        let original_len = index.grants.len();
        index.grants.retain(|grant| {
            grant.transaction_id != transaction_id
                || grant.canonical_workspace != canonical_workspace
        });
        let revoked = index.grants.len() != original_len;
        if revoked {
            self.save(&index)?;
        }
        Ok(revoked)
    }

    fn load(&self) -> Result<CapabilityPackActivationIndex, String> {
        if !self.path.exists() {
            return Ok(CapabilityPackActivationIndex {
                schema_version: ACTIVATION_SCHEMA_VERSION,
                grants: Vec::new(),
            });
        }
        let bytes = std::fs::read(&self.path)
            .map_err(|error| format!("failed to read {}: {error}", self.path.display()))?;
        let index: CapabilityPackActivationIndex = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse {}: {error}", self.path.display()))?;
        if index.schema_version != ACTIVATION_SCHEMA_VERSION {
            return Err(format!(
                "unsupported activation schema version: {}",
                index.schema_version
            ));
        }
        Ok(index)
    }

    fn save(&self, index: &CapabilityPackActivationIndex) -> Result<(), String> {
        let parent = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(index)
            .map_err(|error| format!("failed to serialize activation grants: {error}"))?;
        let mut file = std::fs::File::create(&temporary)
            .map_err(|error| format!("failed to create {}: {error}", temporary.display()))?;
        file.write_all(&bytes)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("failed to write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|error| format!("failed to replace {}: {error}", self.path.display()))?;
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("failed to sync {}: {error}", parent.display()))
    }
}

const fn activation_schema_version() -> u32 {
    ACTIVATION_SCHEMA_VERSION
}
