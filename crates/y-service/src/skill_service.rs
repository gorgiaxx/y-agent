//! Skill service — list, get, uninstall, enable/disable skills.
//!
//! Wraps [`y_skills::FilesystemSkillStore`] and [`y_skills::SkillRegistryImpl`]
//! so that presentation layers do not construct registry instances directly.

use std::path::{Component, Path, PathBuf};

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

/// A file or directory within an installed skill.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillFileEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<SkillFileEntry>>,
}

/// Validation result for one installed skill.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillValidationResult {
    pub name: String,
    pub valid: bool,
    pub errors: Vec<String>,
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

    /// Validate a skill identifier before it is used in filesystem paths.
    pub fn validate_name(name: &str) -> Result<(), String> {
        validate_skill_name(name)
    }

    /// Resolve an installed skill directory after validating its identifier.
    pub fn skill_directory(&self, name: &str) -> Result<PathBuf, String> {
        validate_skill_name(name)?;
        Ok(self.store_path.join(name))
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
        validate_skill_name(name)?;
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
        validate_skill_name(name)?;
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
        validate_skill_name(name)?;
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

    /// Return the recursively sorted file tree for an installed skill.
    pub async fn file_tree(&self, name: &str) -> Result<Vec<SkillFileEntry>, String> {
        let skill_dir = self.skill_directory(name)?;
        if !skill_dir.exists() {
            return Err(format!(
                "Skill directory not found: {}",
                skill_dir.display()
            ));
        }
        tokio::task::spawn_blocking(move || build_file_tree(&skill_dir, &skill_dir))
            .await
            .map_err(|error| format!("Task join error: {error}"))
    }

    /// Read a UTF-8 file within an installed skill directory.
    pub async fn read_file(&self, name: &str, relative_path: &Path) -> Result<String, String> {
        let skill_dir = self.skill_directory(name)?;
        let target = crate::skill_files::resolve_skill_read_path(&skill_dir, relative_path)?;
        tokio::fs::read_to_string(target)
            .await
            .map_err(|error| format!("Failed to read file: {error}"))
    }

    /// Write a file within an installed skill directory.
    pub async fn write_file(
        &self,
        name: &str,
        relative_path: &Path,
        content: &str,
    ) -> Result<(), String> {
        let skill_dir = self.skill_directory(name)?;
        let target = crate::skill_files::resolve_skill_write_path(&skill_dir, relative_path)?;
        tokio::fs::write(target, content)
            .await
            .map_err(|error| format!("Failed to write file: {error}"))
    }

    /// Validate every installed skill and return per-skill diagnostics.
    pub fn validate_all(&self) -> Result<Vec<SkillValidationResult>, String> {
        let store = FilesystemSkillStore::new(&self.store_path)
            .map_err(|error| format!("Failed to open skill store: {error}"))?;
        let manifests = store
            .load_all()
            .map_err(|error| format!("Failed to load skills: {error}"))?;
        let validator = y_skills::SkillValidator::new(y_skills::SkillConfig::default());
        let existing_names = manifests
            .iter()
            .map(|manifest| manifest.name.clone())
            .collect::<std::collections::HashSet<_>>();
        let empty_set = std::collections::HashSet::new();

        Ok(manifests
            .iter()
            .map(|manifest| {
                let skill_dir = self.store_path.join(&manifest.name);
                let errors = validator
                    .validate_directory(&skill_dir)
                    .into_iter()
                    .chain(validator.validate_manifest(
                        manifest,
                        &existing_names,
                        &empty_set,
                        &empty_set,
                        &empty_set,
                    ))
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>();
                SkillValidationResult {
                    name: manifest.name.clone(),
                    valid: errors.is_empty(),
                    errors,
                }
            })
            .collect())
    }
}

fn validate_skill_name(name: &str) -> Result<(), String> {
    let is_plain_name = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if is_plain_name {
        Ok(())
    } else {
        Err(format!("Invalid skill name: {name}"))
    }
}

fn build_file_tree(dir: &Path, relative_base: &Path) -> Vec<SkillFileEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let absolute_path = entry.path();
        let path = absolute_path
            .strip_prefix(relative_base)
            .unwrap_or(&absolute_path)
            .to_string_lossy()
            .into_owned();
        let name = entry.file_name().to_string_lossy().into_owned();
        let (size, children) = if metadata.is_dir() {
            (0, Some(build_file_tree(&absolute_path, relative_base)))
        } else {
            (metadata.len(), None)
        };
        entries.push(SkillFileEntry {
            path,
            name,
            is_dir: metadata.is_dir(),
            size,
            children,
        });
    }

    entries.sort_by(|left, right| match (left.is_dir, right.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
    });
    entries
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

    #[tokio::test]
    async fn test_file_tree_rejects_skill_name_traversal() {
        let dir = TempDir::new().unwrap();
        let svc = SkillService::new(dir.path());

        let error = svc.file_tree("../outside").await.unwrap_err();

        assert!(error.contains("Invalid skill name"));
    }

    #[test]
    fn test_skill_file_write_target_allows_new_file_under_skill_dir() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("writer");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::create_dir_all(skill_dir.join("details")).unwrap();

        let target =
            crate::skill_files::resolve_skill_write_path(&skill_dir, Path::new("details/guide.md"))
                .unwrap();

        assert_eq!(target, skill_dir.join("details/guide.md"));
    }

    #[cfg(unix)]
    #[test]
    fn test_skill_file_write_target_rejects_symlink_escape() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("writer");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, skill_dir.join("link.md")).unwrap();

        let error = crate::skill_files::resolve_skill_write_path(&skill_dir, Path::new("link.md"))
            .unwrap_err();

        assert!(error.contains("symlink"));
    }
}
