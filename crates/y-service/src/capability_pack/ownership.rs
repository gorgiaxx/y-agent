use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::journal::{CapabilityPackTransactionRecord, CapabilityPackTransactionStatus};
use super::validator::ValidatedCapabilityPack;

const OWNERSHIP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CapabilityPackInstallIntent {
    Fresh,
    Update { previous_transaction_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapabilityPackOwnershipIndex {
    pub schema_version: u32,
    pub generation: u64,
    pub packs: BTreeMap<String, InstalledCapabilityPack>,
    pub resources: BTreeMap<String, CapabilityResourceOwner>,
}

impl Default for CapabilityPackOwnershipIndex {
    fn default() -> Self {
        Self {
            schema_version: OWNERSHIP_SCHEMA_VERSION,
            generation: 0,
            packs: BTreeMap::new(),
            resources: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InstalledCapabilityPack {
    pub current_version: String,
    pub current_transaction_id: String,
    pub versions: Vec<InstalledCapabilityPackVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InstalledCapabilityPackVersion {
    pub version: String,
    pub transaction_id: String,
    pub manifest_sha256: String,
    pub resources: Vec<InstalledCapabilityResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InstalledCapabilityResource {
    pub key: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapabilityResourceOwner {
    pub pack_id: String,
    pub pack_version: String,
    pub transaction_id: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CapabilityPackOwnershipError {
    #[error("capability-pack ownership I/O failure at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("capability-pack ownership parse failure at {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("unsupported capability-pack ownership schema version: {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("resource {resource} is owned by {owner_pack}@{owner_version}")]
    ResourceOwnedByAnotherPack {
        resource: String,
        owner_pack: String,
        owner_version: String,
    },
    #[error("pack {pack_id} version {candidate} is not newer than installed version {installed}")]
    VersionNotNewer {
        pack_id: String,
        installed: String,
        candidate: String,
    },
    #[error("pack {pack_id} update changes the declarative resource identity set")]
    ResourceSetChanged { pack_id: String },
    #[error("transaction {transaction_id} is not eligible for ownership commit")]
    TransactionNotCommittable { transaction_id: String },
    #[error("transaction {transaction_id} is not the current version of pack {pack_id}")]
    RollbackNotCurrent {
        pack_id: String,
        transaction_id: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityPackOwnershipStore {
    path: PathBuf,
}

impl CapabilityPackOwnershipStore {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub(crate) fn load(
        &self,
    ) -> Result<CapabilityPackOwnershipIndex, CapabilityPackOwnershipError> {
        if !self.path.exists() {
            return Ok(CapabilityPackOwnershipIndex::default());
        }
        let bytes = std::fs::read(&self.path).map_err(|error| io_error(&self.path, &error))?;
        let index: CapabilityPackOwnershipIndex =
            serde_json::from_slice(&bytes).map_err(|error| {
                CapabilityPackOwnershipError::Parse {
                    path: self.path.clone(),
                    message: error.to_string(),
                }
            })?;
        if index.schema_version != OWNERSHIP_SCHEMA_VERSION {
            return Err(CapabilityPackOwnershipError::UnsupportedSchemaVersion(
                index.schema_version,
            ));
        }
        Ok(index)
    }

    pub(crate) fn validate_install(
        &self,
        pack: &ValidatedCapabilityPack,
    ) -> Result<CapabilityPackInstallIntent, CapabilityPackOwnershipError> {
        let index = self.load()?;
        validate_pack_against_index(&index, pack)
    }

    pub(crate) fn commit(
        &self,
        record: &CapabilityPackTransactionRecord,
    ) -> Result<CapabilityPackOwnershipIndex, CapabilityPackOwnershipError> {
        if !record.ownership_managed
            || !matches!(
                record.status,
                CapabilityPackTransactionStatus::CommitDecided
                    | CapabilityPackTransactionStatus::Committed
            )
        {
            return Err(CapabilityPackOwnershipError::TransactionNotCommittable {
                transaction_id: record.id.clone(),
            });
        }
        let mut index = self.load()?;
        if index.packs.values().any(|pack| {
            pack.versions
                .iter()
                .any(|version| version.transaction_id == record.id)
        }) {
            return Ok(index);
        }
        validate_record_against_index(&index, record)?;

        let resources = record
            .resources
            .iter()
            .map(|resource| InstalledCapabilityResource {
                key: resource_key(&resource.resource),
                sha256: resource.resource.sha256.clone(),
            })
            .collect::<Vec<_>>();
        let installed_version = InstalledCapabilityPackVersion {
            version: record.pack_version.clone(),
            transaction_id: record.id.clone(),
            manifest_sha256: record.provenance.manifest_sha256.clone(),
            resources: resources.clone(),
        };
        match index.packs.get_mut(&record.pack_id) {
            Some(pack) => {
                pack.current_version.clone_from(&record.pack_version);
                pack.current_transaction_id.clone_from(&record.id);
                pack.versions.push(installed_version);
            }
            None => {
                index.packs.insert(
                    record.pack_id.clone(),
                    InstalledCapabilityPack {
                        current_version: record.pack_version.clone(),
                        current_transaction_id: record.id.clone(),
                        versions: vec![installed_version],
                    },
                );
            }
        }
        for resource in resources {
            index.resources.insert(
                resource.key,
                CapabilityResourceOwner {
                    pack_id: record.pack_id.clone(),
                    pack_version: record.pack_version.clone(),
                    transaction_id: record.id.clone(),
                },
            );
        }
        index.generation = index.generation.saturating_add(1);
        self.save(&index)?;
        Ok(index)
    }

    pub(crate) fn uncommit(
        &self,
        record: &CapabilityPackTransactionRecord,
    ) -> Result<CapabilityPackOwnershipIndex, CapabilityPackOwnershipError> {
        let mut index = self.load()?;
        let Some(mut installed) = index.packs.remove(&record.pack_id) else {
            return Ok(index);
        };
        if !installed
            .versions
            .iter()
            .any(|version| version.transaction_id == record.id)
        {
            index.packs.insert(record.pack_id.clone(), installed);
            return Ok(index);
        }
        if installed.current_transaction_id != record.id {
            return Err(CapabilityPackOwnershipError::RollbackNotCurrent {
                pack_id: record.pack_id.clone(),
                transaction_id: record.id.clone(),
            });
        }
        installed.versions.pop();
        for resource in &record.resources {
            let key = resource_key(&resource.resource);
            if index
                .resources
                .get(&key)
                .is_some_and(|owner| owner.transaction_id == record.id)
            {
                index.resources.remove(&key);
            }
        }
        if let Some(previous) = installed.versions.last() {
            installed.current_version.clone_from(&previous.version);
            installed
                .current_transaction_id
                .clone_from(&previous.transaction_id);
            for resource in &previous.resources {
                index.resources.insert(
                    resource.key.clone(),
                    CapabilityResourceOwner {
                        pack_id: record.pack_id.clone(),
                        pack_version: previous.version.clone(),
                        transaction_id: previous.transaction_id.clone(),
                    },
                );
            }
            index.packs.insert(record.pack_id.clone(), installed);
        }
        index.generation = index.generation.saturating_add(1);
        self.save(&index)?;
        Ok(index)
    }

    fn save(
        &self,
        index: &CapabilityPackOwnershipIndex,
    ) -> Result<(), CapabilityPackOwnershipError> {
        let parent = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| io_error(parent, &error))?;
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(index).map_err(|error| {
            CapabilityPackOwnershipError::Parse {
                path: self.path.clone(),
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
        std::fs::rename(&temporary, &self.path).map_err(|error| io_error(&self.path, &error))?;
        sync_directory(parent)?;
        Ok(())
    }
}

fn validate_pack_against_index(
    index: &CapabilityPackOwnershipIndex,
    pack: &ValidatedCapabilityPack,
) -> Result<CapabilityPackInstallIntent, CapabilityPackOwnershipError> {
    for resource in &pack.resources {
        let key = resource_key(resource);
        if let Some(owner) = index.resources.get(&key) {
            if owner.pack_id != pack.id {
                return Err(CapabilityPackOwnershipError::ResourceOwnedByAnotherPack {
                    resource: key,
                    owner_pack: owner.pack_id.clone(),
                    owner_version: owner.pack_version.clone(),
                });
            }
        }
    }
    let Some(installed) = index.packs.get(&pack.id) else {
        return Ok(CapabilityPackInstallIntent::Fresh);
    };
    validate_newer_version(&pack.id, &installed.current_version, &pack.version)?;
    let current_resources = installed
        .versions
        .last()
        .map(|version| {
            version
                .resources
                .iter()
                .map(|resource| resource.key.clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let candidate_resources = pack
        .resources
        .iter()
        .map(resource_key)
        .collect::<BTreeSet<_>>();
    if current_resources != candidate_resources {
        return Err(CapabilityPackOwnershipError::ResourceSetChanged {
            pack_id: pack.id.clone(),
        });
    }
    Ok(CapabilityPackInstallIntent::Update {
        previous_transaction_id: installed.current_transaction_id.clone(),
    })
}

fn validate_record_against_index(
    index: &CapabilityPackOwnershipIndex,
    record: &CapabilityPackTransactionRecord,
) -> Result<(), CapabilityPackOwnershipError> {
    for resource in &record.resources {
        let key = resource_key(&resource.resource);
        if let Some(owner) = index.resources.get(&key) {
            if owner.pack_id != record.pack_id {
                return Err(CapabilityPackOwnershipError::ResourceOwnedByAnotherPack {
                    resource: key,
                    owner_pack: owner.pack_id.clone(),
                    owner_version: owner.pack_version.clone(),
                });
            }
        }
    }
    if let Some(installed) = index.packs.get(&record.pack_id) {
        validate_newer_version(
            &record.pack_id,
            &installed.current_version,
            &record.pack_version,
        )?;
        let current_resources = installed
            .versions
            .last()
            .map(|version| {
                version
                    .resources
                    .iter()
                    .map(|resource| resource.key.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let candidate_resources = record
            .resources
            .iter()
            .map(|resource| resource_key(&resource.resource))
            .collect::<BTreeSet<_>>();
        if current_resources != candidate_resources {
            return Err(CapabilityPackOwnershipError::ResourceSetChanged {
                pack_id: record.pack_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_newer_version(
    pack_id: &str,
    installed: &str,
    candidate: &str,
) -> Result<(), CapabilityPackOwnershipError> {
    let installed_version =
        semver::Version::parse(installed).map_err(|error| CapabilityPackOwnershipError::Parse {
            path: PathBuf::from("ownership version"),
            message: error.to_string(),
        })?;
    let candidate_version =
        semver::Version::parse(candidate).map_err(|error| CapabilityPackOwnershipError::Parse {
            path: PathBuf::from("candidate version"),
            message: error.to_string(),
        })?;
    if candidate_version <= installed_version {
        return Err(CapabilityPackOwnershipError::VersionNotNewer {
            pack_id: pack_id.to_string(),
            installed: installed.to_string(),
            candidate: candidate.to_string(),
        });
    }
    Ok(())
}

fn resource_key(resource: &super::validator::ValidatedCapabilityResource) -> String {
    format!("{}:{}", resource.kind.as_str(), resource.id)
}

fn sync_directory(path: &Path) -> Result<(), CapabilityPackOwnershipError> {
    let directory = std::fs::File::open(path).map_err(|error| io_error(path, &error))?;
    directory.sync_all().map_err(|error| io_error(path, &error))
}

fn io_error(path: &Path, error: &std::io::Error) -> CapabilityPackOwnershipError {
    CapabilityPackOwnershipError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::capability_pack::{
        CapabilityPackProvenance, CapabilityPackSourceKind, CapabilityPackTransactionJournal,
        CapabilityPackTransactionStatus, CapabilityResourceKind, ValidatedCapabilityPack,
        ValidatedCapabilityResource,
    };

    fn resource(kind: CapabilityResourceKind, id: &str) -> ValidatedCapabilityResource {
        ValidatedCapabilityResource {
            kind,
            id: id.to_string(),
            path: PathBuf::from(format!("/pack/{id}")),
            sha256: "a".repeat(64),
        }
    }

    fn pack(
        id: &str,
        version: &str,
        resources: Vec<ValidatedCapabilityResource>,
    ) -> ValidatedCapabilityPack {
        ValidatedCapabilityPack {
            schema_version: 1,
            id: id.to_string(),
            version: version.to_string(),
            description: None,
            provenance: CapabilityPackProvenance {
                source_kind: CapabilityPackSourceKind::LocalDirectory,
                pack_root: PathBuf::from("/pack"),
                manifest_path: PathBuf::from("/pack/capability-pack.toml"),
                manifest_sha256: "f".repeat(64),
            },
            resources,
        }
    }

    fn committed_record(
        journal: &CapabilityPackTransactionJournal,
        pack: &ValidatedCapabilityPack,
    ) -> crate::capability_pack::CapabilityPackTransactionRecord {
        let mut record = journal.begin(pack).expect("begin transaction");
        record.status = CapabilityPackTransactionStatus::Committed;
        record.ownership_managed = true;
        journal.save(&record).expect("save committed record");
        record
    }

    #[test]
    fn ownership_commit_is_idempotent_and_reopens_deterministically() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let journal = CapabilityPackTransactionJournal::new(temp.path().join("transactions"));
        let store = CapabilityPackOwnershipStore::new(temp.path().join("ownership.json"));
        let pack = pack(
            "rust-team",
            "1.0.0",
            vec![resource(CapabilityResourceKind::Skill, "rust-review")],
        );
        let record = committed_record(&journal, &pack);

        store.commit(&record).expect("first ownership commit");
        store.commit(&record).expect("idempotent ownership commit");

        let reopened = store.load().expect("reopen ownership");
        assert_eq!(reopened.generation, 1);
        assert_eq!(
            reopened
                .packs
                .get("rust-team")
                .expect("installed pack")
                .current_transaction_id,
            record.id
        );
        assert_eq!(
            reopened
                .resources
                .get("skill:rust-review")
                .expect("resource owner")
                .pack_version,
            "1.0.0"
        );
    }

    #[test]
    fn ownership_validation_rejects_cross_pack_takeover() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let journal = CapabilityPackTransactionJournal::new(temp.path().join("transactions"));
        let store = CapabilityPackOwnershipStore::new(temp.path().join("ownership.json"));
        let first = pack(
            "rust-team",
            "1.0.0",
            vec![resource(CapabilityResourceKind::Skill, "rust-review")],
        );
        store
            .commit(&committed_record(&journal, &first))
            .expect("initial ownership");
        let takeover = pack(
            "other-team",
            "1.0.0",
            vec![resource(CapabilityResourceKind::Skill, "rust-review")],
        );

        let error = store
            .validate_install(&takeover)
            .expect_err("cross-pack takeover");

        assert!(matches!(
            error,
            CapabilityPackOwnershipError::ResourceOwnedByAnotherPack {
                resource,
                owner_pack,
                ..
            } if resource == "skill:rust-review" && owner_pack == "rust-team"
        ));
    }

    #[test]
    fn update_requires_newer_version_and_same_resource_identity_set() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let journal = CapabilityPackTransactionJournal::new(temp.path().join("transactions"));
        let store = CapabilityPackOwnershipStore::new(temp.path().join("ownership.json"));
        let initial_resources = vec![
            resource(CapabilityResourceKind::Agent, "reviewer"),
            resource(CapabilityResourceKind::Skill, "rust-review"),
        ];
        let initial = pack("rust-team", "1.0.0", initial_resources.clone());
        let initial_record = committed_record(&journal, &initial);
        store.commit(&initial_record).expect("initial ownership");

        let update = pack("rust-team", "1.1.0", initial_resources);
        assert!(matches!(
            store.validate_install(&update).expect("valid update"),
            CapabilityPackInstallIntent::Update {
                previous_transaction_id
            } if previous_transaction_id == initial_record.id
        ));

        let same_version = pack(
            "rust-team",
            "1.0.0",
            vec![
                resource(CapabilityResourceKind::Agent, "reviewer"),
                resource(CapabilityResourceKind::Skill, "rust-review"),
            ],
        );
        assert!(matches!(
            store.validate_install(&same_version),
            Err(CapabilityPackOwnershipError::VersionNotNewer { .. })
        ));

        let dropped_resource = pack(
            "rust-team",
            "1.1.0",
            vec![resource(CapabilityResourceKind::Skill, "rust-review")],
        );
        assert!(matches!(
            store.validate_install(&dropped_resource),
            Err(CapabilityPackOwnershipError::ResourceSetChanged { .. })
        ));
    }

    #[test]
    fn ownership_uncommit_restores_previous_version_then_removes_pack() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let journal = CapabilityPackTransactionJournal::new(temp.path().join("transactions"));
        let store = CapabilityPackOwnershipStore::new(temp.path().join("ownership.json"));
        let resources = vec![resource(CapabilityResourceKind::Skill, "rust-review")];
        let first = committed_record(&journal, &pack("rust-team", "1.0.0", resources.clone()));
        store.commit(&first).expect("first commit");
        let second = committed_record(&journal, &pack("rust-team", "1.1.0", resources));
        store.commit(&second).expect("second commit");

        let rolled_back = store.uncommit(&second).expect("uncommit second");
        let installed = rolled_back
            .packs
            .get("rust-team")
            .expect("previous version");
        assert_eq!(installed.current_version, "1.0.0");
        assert_eq!(installed.current_transaction_id, first.id);
        assert_eq!(rolled_back.generation, 3);
        assert_eq!(
            rolled_back
                .resources
                .get("skill:rust-review")
                .expect("previous owner")
                .pack_version,
            "1.0.0"
        );
        assert_eq!(
            store
                .uncommit(&second)
                .expect("idempotent uncommit")
                .generation,
            3
        );

        let removed = store.uncommit(&first).expect("uncommit first");
        assert!(!removed.packs.contains_key("rust-team"));
        assert!(!removed.resources.contains_key("skill:rust-review"));
        assert_eq!(removed.generation, 4);
    }
}
