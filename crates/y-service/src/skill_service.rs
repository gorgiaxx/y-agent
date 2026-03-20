//! Skill service — list, get, uninstall, enable/disable skills.
//!
//! Wraps [`y_skills::FilesystemSkillStore`] and [`y_skills::SkillRegistryImpl`]
//! so that presentation layers do not construct registry instances directly.

use std::path::{Path, PathBuf};

use y_skills::{FilesystemSkillStore, SkillRegistryImpl};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Skill summary info.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub enabled: bool,
}

/// Full skill detail.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub root_content: String,
    pub author: Option<String>,
    pub classification_type: Option<String>,
    pub dir_path: String,
}

// ---------------------------------------------------------------------------
// SkillService
// ---------------------------------------------------------------------------

/// Service for skill CRUD operations.
///
/// Each method opens the skill store from disk, performs the operation, and
/// returns. This is stateless by design -- the source of truth is the
/// filesystem.
pub struct SkillService {
    store_path: PathBuf,
}

impl SkillService {
    /// Create a new `SkillService` rooted at the given skills directory.
    pub fn new(store_path: &Path) -> Self {
        Self {
            store_path: store_path.to_path_buf(),
        }
    }

    /// List all installed skills with their enabled status.
    pub async fn list(&self) -> Result<Vec<SkillInfo>, String> {
        if !self.store_path.exists() {
            return Ok(vec![]);
        }

        let store = FilesystemSkillStore::new(&self.store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;

        let registry = SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?;

        let disabled = registry.read_disabled_set().await;

        let store2 = FilesystemSkillStore::new(&self.store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;
        let manifests = store2
            .load_all()
            .map_err(|e| format!("Failed to load skills: {e}"))?;

        let mut infos: Vec<SkillInfo> = manifests
            .into_iter()
            .map(|m| SkillInfo {
                name: m.name.clone(),
                description: m.description.clone(),
                version: m.version.0.clone(),
                tags: m.tags.clone(),
                enabled: !disabled.contains(&m.name),
            })
            .collect();

        infos.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(infos)
    }

    /// Get full detail for a single skill.
    pub async fn get(&self, name: &str) -> Result<SkillDetail, String> {
        let store = FilesystemSkillStore::new(&self.store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;

        let manifest = store
            .load_skill(name)
            .map_err(|e| format!("Skill not found: {e}"))?;

        let registry = SkillRegistryImpl::with_store(
            FilesystemSkillStore::new(&self.store_path)
                .map_err(|e| format!("Failed to open skill store: {e}"))?,
        )
        .await
        .map_err(|e| format!("Failed to create registry: {e}"))?;

        let enabled = registry.is_enabled(name).await;
        let classification_type = manifest
            .classification
            .as_ref()
            .map(|c| c.skill_type.to_string());

        Ok(SkillDetail {
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            version: manifest.version.0.clone(),
            tags: manifest.tags.clone(),
            enabled,
            root_content: manifest.root_content.clone(),
            author: manifest.author.clone(),
            classification_type,
            dir_path: self
                .store_path
                .join(&manifest.name)
                .to_string_lossy()
                .to_string(),
        })
    }

    /// Uninstall (delete) a skill.
    pub async fn uninstall(&self, name: &str) -> Result<(), String> {
        let store = FilesystemSkillStore::new(&self.store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;

        store
            .delete_skill(name)
            .map_err(|e| format!("Failed to uninstall skill: {e}"))?;

        // Also remove from disabled list if present.
        let registry = SkillRegistryImpl::with_store(
            FilesystemSkillStore::new(&self.store_path)
                .map_err(|e| format!("Failed to open skill store: {e}"))?,
        )
        .await
        .map_err(|e| format!("Failed to create registry: {e}"))?;
        let _ = registry.set_enabled(name, true).await;

        Ok(())
    }

    /// Enable or disable a skill.
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        let store = FilesystemSkillStore::new(&self.store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;

        let registry = SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?;

        registry
            .set_enabled(name, enabled)
            .await
            .map_err(|e| format!("{e}"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_list_skills_empty() {
        let dir = TempDir::new().unwrap();
        let svc = SkillService::new(dir.path());
        let skills = svc.list().await.unwrap();
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn test_list_skills_nonexistent_dir() {
        let svc = SkillService::new(Path::new("/nonexistent/path"));
        let skills = svc.list().await.unwrap();
        assert!(skills.is_empty());
    }
}
