//! Skill registry: `SkillRegistry` trait implementation.
//!
//! Combines the version store, search index, and sub-document storage
//! into a complete `SkillRegistry` implementation with interior mutability
//! via `tokio::sync::RwLock`.

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use tokio::sync::RwLock;
use y_core::skill::{
    SkillError, SkillManifest, SkillRegistry, SkillSummary, SkillVersion, SubDocumentContent,
};
use y_core::types::SkillId;

use crate::error::SkillModuleError;
use crate::search::SkillSearch;
use crate::store::FilesystemSkillStore;
use crate::version::VersionStore;

/// Inner state guarded by `RwLock`.
#[derive(Debug, Default)]
struct RegistryInner {
    /// All manifests keyed by skill ID.
    manifests: HashMap<String, SkillManifest>,
    /// Sub-document content storage: (`skill_id`, `doc_id`) → content.
    sub_documents: HashMap<(String, String), SubDocumentContent>,
    /// Content-addressable version store.
    version_store: VersionStore,
    /// Search index.
    search: SkillSearch,
    /// Set of disabled skill names.
    disabled: std::collections::HashSet<String>,
}

/// Full implementation of the `SkillRegistry` trait.
///
/// Uses interior mutability (`RwLock`) so all async trait methods work
/// through `&self` as required by the trait contract.
#[derive(Debug)]
pub struct SkillRegistryImpl {
    inner: RwLock<RegistryInner>,
    /// Optional filesystem store for persistence.
    store: Option<FilesystemSkillStore>,
}

impl Default for SkillRegistryImpl {
    fn default() -> Self {
        Self {
            inner: RwLock::new(RegistryInner::default()),
            store: None,
        }
    }
}

impl SkillRegistryImpl {
    /// Create a new empty skill registry (in-memory only).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a skill registry backed by a filesystem store.
    ///
    /// Pre-loads all skills (manifests + sub-document content) from the store.
    pub async fn with_store(store: FilesystemSkillStore) -> Result<Self, SkillModuleError> {
        let registry = Self {
            inner: RwLock::new(RegistryInner::default()),
            store: Some(store),
        };

        // Load existing skills from the filesystem
        if let Some(ref store) = registry.store {
            let manifests = store.load_all()?;
            let mut inner = registry.inner.write().await;

            // Load disabled-skills state from the store directory.
            inner.disabled = Self::load_disabled_from_path(store.base_path());

            for manifest in manifests {
                let skill_id = manifest.id.to_string();
                let skill_name = manifest.name.clone();
                let content = serde_json::to_vec(&manifest).unwrap_or_default();
                let version = inner.version_store.register_version(&skill_id, &content);

                let mut versioned = manifest;
                versioned.version = version;
                inner.search.index(versioned.clone());
                inner.manifests.insert(skill_id.clone(), versioned);

                // Load sub-document content from disk into the in-memory cache.
                if let Ok(sub_docs) = store.load_sub_documents(&skill_name) {
                    for (doc_path, doc_content) in sub_docs {
                        let key = (skill_id.clone(), doc_path.clone());
                        inner.sub_documents.insert(
                            key,
                            SubDocumentContent {
                                id: doc_path.clone(),
                                title: doc_path,
                                content: doc_content,
                                token_estimate: 0,
                            },
                        );
                    }
                }
            }
        }

        Ok(registry)
    }

    /// List all registered skill names.
    pub async fn list_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.manifests.values().map(|m| m.name.clone()).collect()
    }

    // -----------------------------------------------------------------------
    // Enabled / disabled state
    // -----------------------------------------------------------------------

    /// Check whether a skill is enabled (i.e. not in the disabled set).
    pub async fn is_enabled(&self, name: &str) -> bool {
        let inner = self.inner.read().await;
        !inner.disabled.contains(name)
    }

    /// Return a snapshot of all disabled skill names.
    pub async fn read_disabled_set(&self) -> std::collections::HashSet<String> {
        let inner = self.inner.read().await;
        inner.disabled.clone()
    }

    /// Enable or disable a skill by name.
    ///
    /// Persists the change to `disabled_skills.json` if a filesystem store
    /// is configured.
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), SkillError> {
        let mut inner = self.inner.write().await;
        if enabled {
            inner.disabled.remove(name);
        } else {
            inner.disabled.insert(name.to_string());
        }

        // Persist to filesystem.
        if let Some(ref store) = self.store {
            Self::save_disabled_to_path(store.base_path(), &inner.disabled).map_err(|e| {
                SkillError::StorageError {
                    message: format!("failed to persist disabled state: {e}"),
                }
            })?;
        }
        Ok(())
    }

    /// Load the disabled set from a `disabled_skills.json` file next to the store.
    fn load_disabled_from_path(base_path: &Path) -> std::collections::HashSet<String> {
        let path = base_path.join("disabled_skills.json");
        if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str::<Vec<String>>(&content)
                .unwrap_or_default()
                .into_iter()
                .collect()
        } else {
            std::collections::HashSet::new()
        }
    }

    /// Persist the disabled set to `disabled_skills.json`.
    fn save_disabled_to_path(
        base_path: &Path,
        disabled: &std::collections::HashSet<String>,
    ) -> Result<(), String> {
        let path = base_path.join("disabled_skills.json");
        let list: Vec<&String> = disabled.iter().collect();
        let content =
            serde_json::to_string_pretty(&list).map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write disabled_skills.json: {e}"))
    }

    /// Store sub-document content for a registered skill.
    ///
    /// Updates the in-memory cache and, if a filesystem store is configured,
    /// also persists the content to `<skill-dir>/<doc_path>`.
    pub async fn store_sub_document(
        &self,
        skill_id: &str,
        doc_id: &str,
        content: &str,
    ) -> Result<(), SkillError> {
        let mut inner = self.inner.write().await;
        let key = (skill_id.to_string(), doc_id.to_string());

        // Update existing entry or insert new one.
        if let Some(existing) = inner.sub_documents.get_mut(&key) {
            existing.content = content.to_string();
        } else {
            inner.sub_documents.insert(
                key,
                SubDocumentContent {
                    id: doc_id.to_string(),
                    title: doc_id.to_string(),
                    content: content.to_string(),
                    token_estimate: u32::try_from(content.len() / 4).unwrap_or(0),
                },
            );
        }

        // Persist to filesystem if a store is configured.
        if let Some(ref store) = self.store {
            // Look up the skill name from the manifest (filesystem uses name as dir).
            if let Some(manifest) = inner.manifests.get(skill_id) {
                store
                    .write_sub_document(&manifest.name, doc_id, content)
                    .map_err(|e| SkillError::StorageError {
                        message: format!("failed to persist sub-document: {e}"),
                    })?;
            }
        }

        Ok(())
    }

    /// Returns the filesystem store's base path, if a store is configured.
    pub fn store_base_path(&self) -> Option<&Path> {
        self.store.as_ref().map(super::store::FilesystemSkillStore::base_path)
    }
}

#[async_trait]
impl SkillRegistry for SkillRegistryImpl {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SkillSummary>, SkillError> {
        let inner = self.inner.read().await;
        Ok(inner.search.search(query, limit))
    }

    async fn get_manifest(&self, id: &SkillId) -> Result<SkillManifest, SkillError> {
        let inner = self.inner.read().await;
        inner
            .manifests
            .get(id.as_str())
            .cloned()
            .ok_or_else(|| SkillError::NotFound { id: id.to_string() })
    }

    async fn load_sub_document(
        &self,
        skill_id: &SkillId,
        doc_id: &str,
    ) -> Result<SubDocumentContent, SkillError> {
        let inner = self.inner.read().await;
        let key = (skill_id.to_string(), doc_id.to_string());
        inner
            .sub_documents
            .get(&key)
            .cloned()
            .ok_or_else(|| SkillError::SubDocumentNotFound {
                skill_id: skill_id.to_string(),
                doc_id: doc_id.to_string(),
            })
    }

    async fn register(&self, manifest: SkillManifest) -> Result<SkillVersion, SkillError> {
        let mut inner = self.inner.write().await;
        let skill_id = manifest.id.to_string();
        let content = serde_json::to_vec(&manifest).map_err(|e| SkillError::Other {
            message: format!("serialization error: {e}"),
        })?;

        let version = inner.version_store.register_version(&skill_id, &content);

        // Store sub-document content
        for sub_doc in &manifest.sub_documents {
            let key = (skill_id.clone(), sub_doc.id.clone());
            inner.sub_documents.insert(
                key,
                SubDocumentContent {
                    id: sub_doc.id.clone(),
                    title: sub_doc.title.clone(),
                    content: String::new(), // Content loaded from ingestion
                    token_estimate: sub_doc.token_estimate,
                },
            );
        }

        // Update manifest with version
        let mut versioned = manifest;
        versioned.version = version.clone();

        // Persist to filesystem if a store is configured
        if let Some(ref store) = self.store {
            store
                .save_skill(&versioned)
                .map_err(|e| SkillError::StorageError {
                    message: format!("failed to persist skill: {e}"),
                })?;
        }

        // Update search index
        inner.search.index(versioned.clone());
        inner.manifests.insert(skill_id, versioned);

        Ok(version)
    }

    async fn rollback(
        &self,
        id: &SkillId,
        target_version: &SkillVersion,
    ) -> Result<(), SkillError> {
        let mut inner = self.inner.write().await;

        // Verify the version exists
        let content = inner.version_store.get(&target_version.0).ok_or_else(|| {
            SkillError::VersionNotFound {
                version: target_version.to_string(),
            }
        })?;

        // Deserialize the manifest from the stored content
        let manifest: SkillManifest =
            serde_json::from_slice(content).map_err(|e| SkillError::Other {
                message: format!("failed to deserialize version: {e}"),
            })?;

        // Update version store
        inner
            .version_store
            .rollback(id.as_str(), target_version)
            .map_err(|e| SkillError::Other {
                message: e.to_string(),
            })?;

        // Update manifest and search
        inner.search.index(manifest.clone());
        inner.manifests.insert(id.to_string(), manifest);

        Ok(())
    }

    async fn version_history(&self, id: &SkillId) -> Result<Vec<SkillVersion>, SkillError> {
        let inner = self.inner.read().await;
        Ok(inner.version_store.history(id.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::skill::{SkillManifest, SkillVersion, SubDocumentRef};
    use y_core::types::{now, SkillId};

    fn test_manifest(name: &str) -> SkillManifest {
        let now = now();
        SkillManifest {
            id: SkillId::from_string("skill-test-1"),
            name: name.to_string(),
            description: format!("{name} skill"),
            version: SkillVersion(String::new()),
            tags: vec!["rust".to_string()],
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: "root content for testing".to_string(),
            sub_documents: vec![SubDocumentRef {
                id: "sub-1".to_string(),
                path: "details/sub-1.md".to_string(),
                title: "Sub Doc 1".to_string(),
                load_condition: "when needed".to_string(),
                token_estimate: 50,
            }],
            token_estimate: 10,
            created_at: now,
            updated_at: now,
            classification: None,
            constraints: None,
            security: None,
            references: None,
            author: None,
            source_format: None,
            source_hash: None,
            state: None,
            root_path: None,
        }
    }

    /// T-SK-S1-03 / T-SKILL-004-01: Register new skill via async trait → retrievable, version created.
    #[tokio::test]
    async fn test_registry_register_new_skill() {
        let registry = SkillRegistryImpl::new();
        let manifest = test_manifest("test-skill");
        let id = manifest.id.clone();

        let version = registry.register(manifest).await.unwrap();
        assert!(!version.0.is_empty());

        let retrieved = registry.get_manifest(&id).await.unwrap();
        assert_eq!(retrieved.name, "test-skill");
        assert_eq!(retrieved.version, version);
    }

    /// T-SKILL-004-02: Register same ID → new version, old in history.
    #[tokio::test]
    async fn test_registry_update_creates_new_version() {
        let registry = SkillRegistryImpl::new();

        let mut m1 = test_manifest("version-1");
        let id = m1.id.clone();
        let v1 = registry.register(m1.clone()).await.unwrap();

        m1.root_content = "updated root content".to_string();
        let v2 = registry.register(m1).await.unwrap();

        assert_ne!(v1, v2);

        let history = registry.version_history(&id).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    /// T-SKILL-004-03: `get_manifest()` returns full manifest with root_content.
    #[tokio::test]
    async fn test_registry_get_manifest() {
        let registry = SkillRegistryImpl::new();
        let manifest = test_manifest("full-manifest");
        let id = manifest.id.clone();
        registry.register(manifest).await.unwrap();

        let retrieved = registry.get_manifest(&id).await.unwrap();
        assert_eq!(retrieved.root_content, "root content for testing");
    }

    /// T-SKILL-004-04: `load_sub_document()` returns content for sub-doc ID.
    #[tokio::test]
    async fn test_registry_load_sub_document() {
        let registry = SkillRegistryImpl::new();
        let manifest = test_manifest("with-subdocs");
        let id = manifest.id.clone();
        registry.register(manifest).await.unwrap();

        let sub_doc = registry.load_sub_document(&id, "sub-1").await.unwrap();
        assert_eq!(sub_doc.id, "sub-1");
        assert_eq!(sub_doc.title, "Sub Doc 1");
    }

    /// T-SK-S1-04 / T-SKILL-004-05: Rollback via async trait changes active version to target.
    #[tokio::test]
    async fn test_registry_rollback() {
        let registry = SkillRegistryImpl::new();

        let mut m1 = test_manifest("rollback-test");
        let id = m1.id.clone();
        let v1 = registry.register(m1.clone()).await.unwrap();

        m1.root_content = "v2 content".to_string();
        let _v2 = registry.register(m1).await.unwrap();

        registry.rollback(&id, &v1).await.unwrap();

        let current = registry.get_manifest(&id).await.unwrap();
        assert_eq!(current.root_content, "root content for testing");
    }
}
