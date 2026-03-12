//! Filesystem-backed skill store for persistent skill storage.
//!
//! Stores skills on disk in a proprietary directory structure:
//!
//! ```text
//! <store_path>/
//!   <skill-name>/
//!     skill.toml        # Manifest
//!     root.md           # Root content
//!     details/          # Sub-documents
//!       <sub-doc-id>.md
//!     lineage.toml      # Transformation lineage (created empty)
//! ```
//!
//! Writes are atomic (write to temp, rename) to avoid partial files.

use std::path::{Path, PathBuf};

use y_core::skill::SkillManifest;

use crate::error::SkillModuleError;
use crate::manifest::ManifestParser;

/// Filesystem-backed persistent store for skills.
#[derive(Debug)]
pub struct FilesystemSkillStore {
    /// Root path where all skills are stored.
    base_path: PathBuf,
}

impl FilesystemSkillStore {
    /// Create a new filesystem skill store at the given base path.
    ///
    /// Creates the directory if it doesn't exist.
    pub fn new(base_path: impl Into<PathBuf>) -> Result<Self, SkillModuleError> {
        let base_path = base_path.into();
        std::fs::create_dir_all(&base_path).map_err(|e| SkillModuleError::Other {
            message: format!(
                "failed to create store directory {}: {e}",
                base_path.display()
            ),
        })?;
        Ok(Self { base_path })
    }

    /// Save a skill manifest to the filesystem.
    ///
    /// Uses atomic write: writes to a temp directory, then renames.
    pub fn save_skill(&self, manifest: &SkillManifest) -> Result<(), SkillModuleError> {
        let skill_dir = self.base_path.join(&manifest.name);
        let tmp_dir = self.base_path.join(format!(".{}.tmp", manifest.name));

        // Clean up any leftover temp directory
        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to clean temp dir: {e}"),
            })?;
        }

        // Create temp directory structure
        std::fs::create_dir_all(&tmp_dir).map_err(|e| SkillModuleError::Other {
            message: format!("failed to create temp dir: {e}"),
        })?;

        // Write skill.toml
        let toml_content =
            ManifestParser::to_toml(manifest).map_err(|e| SkillModuleError::Other {
                message: format!("failed to serialize manifest: {e}"),
            })?;
        std::fs::write(tmp_dir.join("skill.toml"), &toml_content).map_err(|e| {
            SkillModuleError::Other {
                message: format!("failed to write skill.toml: {e}"),
            }
        })?;

        // Write root.md
        std::fs::write(tmp_dir.join("root.md"), &manifest.root_content).map_err(|e| {
            SkillModuleError::Other {
                message: format!("failed to write root.md: {e}"),
            }
        })?;

        // Create details directory and write sub-document placeholders
        if !manifest.sub_documents.is_empty() {
            let details_dir = tmp_dir.join("details");
            std::fs::create_dir_all(&details_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to create details dir: {e}"),
            })?;

            for sub_doc in &manifest.sub_documents {
                let filename = sanitize_filename(&sub_doc.id);
                let sub_doc_path = details_dir.join(format!("{filename}.md"));
                // Write placeholder — actual content is stored in the registry
                std::fs::write(
                    &sub_doc_path,
                    format!("# {}\n\n(sub-document placeholder)", sub_doc.title),
                )
                .map_err(|e| SkillModuleError::Other {
                    message: format!("failed to write sub-doc {}: {e}", sub_doc.id),
                })?;
            }
        }

        // Create empty lineage.toml
        std::fs::write(tmp_dir.join("lineage.toml"), "# Transformation lineage\n").map_err(
            |e| SkillModuleError::Other {
                message: format!("failed to write lineage.toml: {e}"),
            },
        )?;

        // Atomic swap: remove old, rename temp to final
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to remove old skill dir: {e}"),
            })?;
        }
        std::fs::rename(&tmp_dir, &skill_dir).map_err(|e| SkillModuleError::Other {
            message: format!("failed to rename temp dir to final: {e}"),
        })?;

        Ok(())
    }

    /// Load a single skill from the filesystem by name.
    pub fn load_skill(&self, name: &str) -> Result<SkillManifest, SkillModuleError> {
        let skill_dir = self.base_path.join(name);
        Self::load_from_dir(&skill_dir)
    }

    /// Load all skills from the store directory.
    pub fn load_all(&self) -> Result<Vec<SkillManifest>, SkillModuleError> {
        let mut manifests = Vec::new();

        let entries = std::fs::read_dir(&self.base_path).map_err(|e| SkillModuleError::Other {
            message: format!(
                "failed to read store directory {}: {e}",
                self.base_path.display()
            ),
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| SkillModuleError::Other {
                message: format!("failed to read directory entry: {e}"),
            })?;

            let path = entry.path();

            // Skip temp directories and non-directories
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }

            // Try to load the skill; skip if it fails (corrupt data)
            match Self::load_from_dir(&path) {
                Ok(manifest) => manifests.push(manifest),
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "skipping corrupt skill directory"
                    );
                }
            }
        }

        Ok(manifests)
    }

    /// Delete a skill from the filesystem.
    pub fn delete_skill(&self, name: &str) -> Result<(), SkillModuleError> {
        let skill_dir = self.base_path.join(name);
        if skill_dir.exists() {
            std::fs::remove_dir_all(&skill_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to delete skill dir: {e}"),
            })?;
        }
        Ok(())
    }

    /// Load a skill manifest from a directory.
    fn load_from_dir(skill_dir: &Path) -> Result<SkillManifest, SkillModuleError> {
        let toml_path = skill_dir.join("skill.toml");
        let root_path = skill_dir.join("root.md");

        // Read skill.toml
        let toml_content =
            std::fs::read_to_string(&toml_path).map_err(|e| SkillModuleError::Other {
                message: format!("failed to read {}: {e}", toml_path.display()),
            })?;

        // Read root.md (optional — content may be inline in the TOML)
        let root_content = if root_path.exists() {
            std::fs::read_to_string(&root_path).map_err(|e| SkillModuleError::Other {
                message: format!("failed to read {}: {e}", root_path.display()),
            })?
        } else {
            String::new()
        };

        // Parse the manifest using ManifestParser
        let parser = ManifestParser::new(crate::config::SkillConfig::default());
        let mut manifest = parser.parse(&toml_content)?;

        // Override root_content with the file content if it was loaded from root.md
        if !root_content.is_empty() && manifest.root_content.is_empty() {
            manifest.root_content = root_content;
            manifest.token_estimate = crate::manifest::estimate_tokens(&manifest.root_content);
        }

        Ok(manifest)
    }

    /// Get the base path of this store.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }
}

/// Sanitize a filename by replacing path separators and other problematic chars.
fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::skill::{SkillVersion, SubDocumentRef};
    use y_core::types::{now, SkillId};

    fn test_manifest(name: &str) -> SkillManifest {
        let now = now();
        SkillManifest {
            id: SkillId::from_string(format!("skill-{name}")),
            name: name.to_string(),
            description: format!("Test skill: {name}"),
            version: SkillVersion(String::new()),
            tags: vec!["test".to_string()],
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: "Root content for testing persistence.".to_string(),
            sub_documents: vec![SubDocumentRef {
                id: "sub-1".to_string(),
                path: "details/sub-1.md".to_string(),
                title: "Sub Document 1".to_string(),
                load_condition: "when needed".to_string(),
                token_estimate: 50,
            }],
            token_estimate: 10,
            created_at: now,
            updated_at: now,
            classification: None,
            constraints: None,
            safety: None,
            references: None,
            author: None,
            source_format: None,
            source_hash: None,
            state: None,
            root_path: None,
        }
    }

    /// T-SK-S1-05: `FilesystemSkillStore` write + read roundtrip.
    #[test]
    fn test_filesystem_store_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(tmp.path()).unwrap();

        let manifest = test_manifest("roundtrip-test");
        store.save_skill(&manifest).unwrap();

        // Verify files exist
        let skill_dir = tmp.path().join("roundtrip-test");
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("root.md").exists());
        assert!(skill_dir.join("lineage.toml").exists());
        assert!(skill_dir.join("details").join("sub-1.md").exists());

        // Read back and verify
        let loaded = store.load_skill("roundtrip-test").unwrap();
        assert_eq!(loaded.name, "roundtrip-test");
        assert_eq!(loaded.description, "Test skill: roundtrip-test");
        assert_eq!(loaded.tags, vec!["test"]);
    }

    /// T-SK-S1-06: Registry loads skills from filesystem store on construction.
    #[test]
    fn test_filesystem_store_load_all() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(tmp.path()).unwrap();

        store.save_skill(&test_manifest("skill-a")).unwrap();
        store.save_skill(&test_manifest("skill-b")).unwrap();
        store.save_skill(&test_manifest("skill-c")).unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 3);

        let names: Vec<&str> = all.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"skill-a"));
        assert!(names.contains(&"skill-b"));
        assert!(names.contains(&"skill-c"));
    }

    /// T-SK-S1-07: Atomic write doesn't leave partial files on simulated failure.
    ///
    /// Verifies that the temp directory is cleaned up if it exists from a
    /// previous interrupted write.
    #[test]
    fn test_filesystem_store_atomic_write() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(tmp.path()).unwrap();

        // Simulate a leftover temp dir from a crashed write
        let leftover_tmp = tmp.path().join(".crash-skill.tmp");
        std::fs::create_dir_all(&leftover_tmp).unwrap();
        std::fs::write(leftover_tmp.join("partial.txt"), "partial data").unwrap();

        // A normal write should clean up the leftover and succeed
        let manifest = test_manifest("crash-skill");
        store.save_skill(&manifest).unwrap();

        // Temp dir should be gone
        assert!(!leftover_tmp.exists());

        // Skill dir should exist and be complete
        let skill_dir = tmp.path().join("crash-skill");
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("root.md").exists());
    }

    /// Overwriting an existing skill replaces resources completely.
    #[test]
    fn test_filesystem_store_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(tmp.path()).unwrap();

        let mut manifest = test_manifest("overwrite-test");
        store.save_skill(&manifest).unwrap();

        manifest.root_content = "Updated root content.".to_string();
        store.save_skill(&manifest).unwrap();

        let root_content =
            std::fs::read_to_string(tmp.path().join("overwrite-test").join("root.md")).unwrap();
        assert_eq!(root_content, "Updated root content.");
    }

    /// Delete removes the skill directory.
    #[test]
    fn test_filesystem_store_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(tmp.path()).unwrap();

        store.save_skill(&test_manifest("delete-me")).unwrap();
        assert!(tmp.path().join("delete-me").exists());

        store.delete_skill("delete-me").unwrap();
        assert!(!tmp.path().join("delete-me").exists());
    }
}
