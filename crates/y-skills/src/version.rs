//! Version store: content-addressable storage with JSONL reflog.
//!
//! Uses SHA-256 for content-addressable hashing. Version history
//! is tracked via an append-only JSONL reflog per skill.

use std::collections::HashMap;
use std::io::Write;

use sha2::{Digest, Sha256};
use y_core::skill::SkillVersion;

use crate::error::SkillModuleError;

/// A reflog entry tracking a version change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReflogEntry {
    pub version: SkillVersion,
    pub timestamp: String,
    pub action: ReflogAction,
}

/// Actions recorded in the reflog.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflogAction {
    /// Initial registration.
    Created,
    /// Updated to a new version.
    Updated,
    /// Rolled back to a previous version.
    RolledBack,
}

/// In-memory content-addressable version store with reflog.
///
/// Each piece of content is stored under its SHA-256 hash.
/// Deduplication is automatic: storing the same content twice
/// returns the same hash without consuming additional storage.
#[derive(Debug, Default)]
pub struct VersionStore {
    /// Content storage: hash → content bytes.
    objects: HashMap<String, Vec<u8>>,
    /// Per-skill reflog: `skill_id` → ordered entries.
    reflogs: HashMap<String, Vec<ReflogEntry>>,
    /// Per-skill active version: `skill_id` → hash.
    active_versions: HashMap<String, SkillVersion>,
}

impl VersionStore {
    /// Create a new empty version store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store content and return its content-addressable hash.
    ///
    /// If the content already exists, the existing hash is returned
    /// without duplicating storage.
    pub fn store(&mut self, content: &[u8]) -> String {
        let hash = content_hash(content);
        self.objects
            .entry(hash.clone())
            .or_insert_with(|| content.to_vec());
        hash
    }

    /// Retrieve content by hash.
    pub fn get(&self, hash: &str) -> Option<&[u8]> {
        self.objects.get(hash).map(Vec::as_slice)
    }

    /// Register a new version for a skill (appends reflog entry).
    pub fn register_version(&mut self, skill_id: &str, content: &[u8]) -> SkillVersion {
        let hash = self.store(content);
        let version = SkillVersion(hash);

        let action = if self.active_versions.contains_key(skill_id) {
            ReflogAction::Updated
        } else {
            ReflogAction::Created
        };

        self.active_versions
            .insert(skill_id.to_string(), version.clone());

        let entry = ReflogEntry {
            version: version.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            action,
        };

        self.reflogs
            .entry(skill_id.to_string())
            .or_default()
            .push(entry);

        version
    }

    /// Get the active version for a skill.
    pub fn active_version(&self, skill_id: &str) -> Option<&SkillVersion> {
        self.active_versions.get(skill_id)
    }

    /// Get the full version history for a skill.
    pub fn history(&self, skill_id: &str) -> Vec<SkillVersion> {
        self.reflogs
            .get(skill_id)
            .map(|entries| entries.iter().map(|e| e.version.clone()).collect())
            .unwrap_or_default()
    }

    /// Rollback a skill to a target version.
    pub fn rollback(
        &mut self,
        skill_id: &str,
        target_version: &SkillVersion,
    ) -> Result<(), SkillModuleError> {
        // Verify the target version exists in object store
        if !self.objects.contains_key(&target_version.0) {
            return Err(SkillModuleError::VersionStoreError {
                message: format!("version {} not found in store", target_version.0),
            });
        }

        self.active_versions
            .insert(skill_id.to_string(), target_version.clone());

        let entry = ReflogEntry {
            version: target_version.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            action: ReflogAction::RolledBack,
        };

        self.reflogs
            .entry(skill_id.to_string())
            .or_default()
            .push(entry);

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Persistent version store (filesystem-backed CAS + reflog)
// ---------------------------------------------------------------------------

/// Filesystem-backed content-addressable store with JSONL reflog.
///
/// Directory layout:
/// ```text
/// <base_path>/
///   objects/<ab>/<full-hash>   — immutable content blobs
///   refs/<skill-name>/HEAD    — current version hash
///   refs/<skill-name>/reflog  — JSONL append-only log
/// ```
#[derive(Debug)]
pub struct PersistentVersionStore {
    /// In-memory store for fast lookups (cache).
    inner: VersionStore,
    /// Base path for the filesystem store.
    base_path: std::path::PathBuf,
}

impl PersistentVersionStore {
    /// Create a new persistent version store at the given path.
    pub fn new(base_path: impl Into<std::path::PathBuf>) -> Result<Self, SkillModuleError> {
        let base_path = base_path.into();
        std::fs::create_dir_all(base_path.join("objects")).map_err(|e| {
            SkillModuleError::VersionStoreError {
                message: format!("failed to create objects dir: {e}"),
            }
        })?;
        std::fs::create_dir_all(base_path.join("refs")).map_err(|e| {
            SkillModuleError::VersionStoreError {
                message: format!("failed to create refs dir: {e}"),
            }
        })?;

        Ok(Self {
            inner: VersionStore::new(),
            base_path,
        })
    }

    /// Store content to both memory and filesystem.
    pub fn store(&mut self, content: &[u8]) -> Result<String, SkillModuleError> {
        let hash = self.inner.store(content);
        let prefix = &hash[..2];
        let obj_dir = self.base_path.join("objects").join(prefix);
        let obj_path = obj_dir.join(&hash);

        if !obj_path.exists() {
            std::fs::create_dir_all(&obj_dir).map_err(|e| SkillModuleError::VersionStoreError {
                message: format!("failed to create object dir: {e}"),
            })?;
            std::fs::write(&obj_path, content).map_err(|e| {
                SkillModuleError::VersionStoreError {
                    message: format!("failed to write object: {e}"),
                }
            })?;
        }

        Ok(hash)
    }

    /// Retrieve content by hash (memory first, then filesystem).
    pub fn get(&self, hash: &str) -> Option<Vec<u8>> {
        if let Some(content) = self.inner.get(hash) {
            return Some(content.to_vec());
        }
        if hash.len() < 2 {
            return None;
        }
        let prefix = &hash[..2];
        let obj_path = self.base_path.join("objects").join(prefix).join(hash);
        std::fs::read(&obj_path).ok()
    }

    /// Register a new version with filesystem persistence.
    pub fn register_version(
        &mut self,
        skill_id: &str,
        content: &[u8],
    ) -> Result<SkillVersion, SkillModuleError> {
        self.store(content)?;
        let version = self.inner.register_version(skill_id, content);
        self.write_head(skill_id, &version)?;

        let action = if self.inner.history(skill_id).len() == 1 {
            ReflogAction::Created
        } else {
            ReflogAction::Updated
        };
        self.append_reflog(skill_id, &version, action)?;
        Ok(version)
    }

    /// Rollback with filesystem persistence.
    pub fn rollback(
        &mut self,
        skill_id: &str,
        target_version: &SkillVersion,
    ) -> Result<(), SkillModuleError> {
        self.inner.rollback(skill_id, target_version)?;
        self.write_head(skill_id, target_version)?;
        self.append_reflog(skill_id, target_version, ReflogAction::RolledBack)?;
        Ok(())
    }

    /// Get version history.
    pub fn history(&self, skill_id: &str) -> Vec<SkillVersion> {
        self.inner.history(skill_id)
    }

    /// Get the active version.
    pub fn active_version(&self, skill_id: &str) -> Option<&SkillVersion> {
        self.inner.active_version(skill_id)
    }

    /// Read the HEAD file for a skill from the filesystem.
    pub fn read_head(&self, skill_id: &str) -> Option<SkillVersion> {
        let head_path = self.base_path.join("refs").join(skill_id).join("HEAD");
        std::fs::read_to_string(&head_path)
            .ok()
            .map(|s| SkillVersion(s.trim().to_string()))
    }

    /// Read the reflog for a skill from the filesystem.
    pub fn read_reflog(&self, skill_id: &str) -> Vec<ReflogEntry> {
        let reflog_path = self.base_path.join("refs").join(skill_id).join("reflog");
        match std::fs::read_to_string(&reflog_path) {
            Ok(c) => c
                .lines()
                .filter(|l| !l.is_empty())
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect(),
            Err(_) => vec![],
        }
    }

    fn write_head(&self, skill_id: &str, version: &SkillVersion) -> Result<(), SkillModuleError> {
        let refs_dir = self.base_path.join("refs").join(skill_id);
        std::fs::create_dir_all(&refs_dir).map_err(|e| SkillModuleError::VersionStoreError {
            message: format!("failed to create refs dir: {e}"),
        })?;
        std::fs::write(refs_dir.join("HEAD"), &version.0).map_err(|e| {
            SkillModuleError::VersionStoreError {
                message: format!("failed to write HEAD: {e}"),
            }
        })?;
        Ok(())
    }

    fn append_reflog(
        &self,
        skill_id: &str,
        version: &SkillVersion,
        action: ReflogAction,
    ) -> Result<(), SkillModuleError> {
        let refs_dir = self.base_path.join("refs").join(skill_id);
        std::fs::create_dir_all(&refs_dir).map_err(|e| SkillModuleError::VersionStoreError {
            message: format!("failed to create refs dir: {e}"),
        })?;

        let entry = ReflogEntry {
            version: version.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            action,
        };

        let mut line =
            serde_json::to_string(&entry).map_err(|e| SkillModuleError::VersionStoreError {
                message: format!("failed to serialize reflog entry: {e}"),
            })?;
        line.push('\n');

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(refs_dir.join("reflog"))
            .map_err(|e| SkillModuleError::VersionStoreError {
                message: format!("failed to open reflog: {e}"),
            })?;
        file.write_all(line.as_bytes())
            .map_err(|e| SkillModuleError::VersionStoreError {
                message: format!("failed to write reflog: {e}"),
            })?;
        Ok(())
    }
}

/// Compute SHA-256 hash of content, returned as hex string.
fn content_hash(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    format!("{result:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SKILL-002-01: Storing content returns a content-addressable hash.
    #[test]
    fn test_version_store_creates_hash() {
        let mut store = VersionStore::new();
        let hash = store.store(b"hello world");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA-256 hex = 64 chars
    }

    /// T-SKILL-002-02: Storing same content twice returns same hash.
    #[test]
    fn test_version_store_dedup() {
        let mut store = VersionStore::new();
        let hash1 = store.store(b"identical content");
        let hash2 = store.store(b"identical content");
        assert_eq!(hash1, hash2);
        // Only one entry in the store
        assert_eq!(store.objects.len(), 1);
    }

    /// T-SKILL-002-03: Registering a version appends a reflog entry.
    #[test]
    fn test_version_store_reflog_append() {
        let mut store = VersionStore::new();
        let _v1 = store.register_version("skill-1", b"version 1 content");

        let history = store.history("skill-1");
        assert_eq!(history.len(), 1);

        let reflog = &store.reflogs["skill-1"];
        assert!(matches!(reflog[0].action, ReflogAction::Created));
    }

    /// T-SKILL-002-04: Rollback changes the active version.
    #[test]
    fn test_version_store_rollback() {
        let mut store = VersionStore::new();
        let v1 = store.register_version("skill-1", b"version 1");
        let _v2 = store.register_version("skill-1", b"version 2");

        assert_ne!(store.active_version("skill-1").unwrap(), &v1);

        store.rollback("skill-1", &v1).unwrap();
        assert_eq!(store.active_version("skill-1").unwrap(), &v1);
    }

    /// T-SKILL-002-05: 3 versions produces history of length 3.
    #[test]
    fn test_version_store_history() {
        let mut store = VersionStore::new();
        store.register_version("skill-1", b"v1");
        store.register_version("skill-1", b"v2");
        store.register_version("skill-1", b"v3");

        let history = store.history("skill-1");
        assert_eq!(history.len(), 3);
    }

    /// T-SK-S3-01: Persistent CAS write + read roundtrip.
    #[test]
    fn test_persistent_cas_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = PersistentVersionStore::new(tmp.path()).unwrap();

        let content = b"hello persistent world";
        let hash = store.store(content).unwrap();

        // Read from memory
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, content);

        // Verify file exists on disk
        let prefix = &hash[..2];
        let obj_path = tmp.path().join("objects").join(prefix).join(&hash);
        assert!(obj_path.exists());
        assert_eq!(std::fs::read(&obj_path).unwrap(), content);
    }

    /// T-SK-S3-02: Reflog JSONL persists Created/Updated/RolledBack.
    #[test]
    fn test_persistent_reflog_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = PersistentVersionStore::new(tmp.path()).unwrap();

        let v1 = store.register_version("skill-r", b"v1 data").unwrap();
        let _v2 = store.register_version("skill-r", b"v2 data").unwrap();
        store.rollback("skill-r", &v1).unwrap();

        let reflog = store.read_reflog("skill-r");
        assert_eq!(reflog.len(), 3);
        assert!(matches!(reflog[0].action, ReflogAction::Created));
        assert!(matches!(reflog[1].action, ReflogAction::Updated));
        assert!(matches!(reflog[2].action, ReflogAction::RolledBack));
    }

    /// T-SK-S3-03: HEAD pointer updated on register/rollback.
    #[test]
    fn test_persistent_head_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = PersistentVersionStore::new(tmp.path()).unwrap();

        let v1 = store.register_version("skill-h", b"head v1").unwrap();
        let head = store.read_head("skill-h").unwrap();
        assert_eq!(head, v1);

        let v2 = store.register_version("skill-h", b"head v2").unwrap();
        let head = store.read_head("skill-h").unwrap();
        assert_eq!(head, v2);

        store.rollback("skill-h", &v1).unwrap();
        let head = store.read_head("skill-h").unwrap();
        assert_eq!(head, v1);
    }
}
