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

        // Create details directory structure for sub-documents.
        // Actual content is written later via `write_sub_document()`.
        if !manifest.sub_documents.is_empty() {
            let details_dir = tmp_dir.join("details");
            std::fs::create_dir_all(&details_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to create details dir: {e}"),
            })?;
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
    ///
    /// Reads `skill.toml`, merges in `root.md` content, and returns the manifest.
    /// Sub-document content is loaded separately via `load_sub_documents()`.
    fn load_from_dir(skill_dir: &Path) -> Result<SkillManifest, SkillModuleError> {
        let toml_path = skill_dir.join("skill.toml");
        let root_path = skill_dir.join("root.md");

        // Read skill.toml
        let toml_content =
            std::fs::read_to_string(&toml_path).map_err(|e| SkillModuleError::Other {
                message: format!("failed to read {}: {e}", toml_path.display()),
            })?;

        // Read root.md (the canonical location for root content)
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

        // root.md is the canonical source for root content. Use it whenever
        // available, falling back to inline content from the TOML only when
        // no root.md file exists.
        if !root_content.is_empty() {
            manifest.root_content = root_content;
            manifest.token_estimate = crate::manifest::estimate_tokens(&manifest.root_content);
        }

        Ok(manifest)
    }

    /// Write a single sub-document file to a skill's directory.
    ///
    /// `doc_path` is the relative path within the skill directory
    /// (e.g. `"details/tone-guidelines.md"`). Parent directories are
    /// created automatically.
    pub fn write_sub_document(
        &self,
        skill_name: &str,
        doc_path: &str,
        content: &str,
    ) -> Result<(), SkillModuleError> {
        let skill_dir = self.base_path.join(skill_name);
        let file_path = skill_dir.join(doc_path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SkillModuleError::Other {
                message: format!("failed to create sub-doc parent dir: {e}"),
            })?;
        }

        std::fs::write(&file_path, content).map_err(|e| SkillModuleError::Other {
            message: format!("failed to write sub-doc {}: {e}", doc_path),
        })?;

        Ok(())
    }

    /// Load all sub-document contents from a skill directory.
    ///
    /// Returns a map of `doc_path` -> `content` for every file found under
    /// the `details/` subdirectory.
    pub fn load_sub_documents(
        &self,
        skill_name: &str,
    ) -> Result<Vec<(String, String)>, SkillModuleError> {
        let details_dir = self.base_path.join(skill_name).join("details");
        if !details_dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let entries =
            std::fs::read_dir(&details_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to read details dir: {e}"),
            })?;

        for entry in entries {
            let entry = entry.map_err(|e| SkillModuleError::Other {
                message: format!("failed to read dir entry: {e}"),
            })?;
            let path = entry.path();
            if path.is_file() {
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let doc_path = format!("details/{filename}");
                let content =
                    std::fs::read_to_string(&path).map_err(|e| SkillModuleError::Other {
                        message: format!("failed to read sub-doc {}: {e}", doc_path),
                    })?;
                results.push((doc_path, content));
            }
        }

        Ok(results)
    }

    /// Copy companion files from a source directory into an installed skill
    /// directory.
    ///
    /// "Companion files" are everything in `source_dir` **except** the main
    /// file that was already processed by the ingestion agent.  Generated
    /// files (`skill.toml`, `root.md`, `lineage.toml`, `details/`) that
    /// already exist in the target are never overwritten.
    ///
    /// The copy is binary-safe (`std::fs::copy`) and preserves the relative
    /// directory structure.
    pub fn copy_companion_files(
        &self,
        skill_name: &str,
        source_dir: &Path,
        main_file_name: Option<&str>,
    ) -> Result<(), SkillModuleError> {
        let skill_dir = self.base_path.join(skill_name);
        if !skill_dir.exists() {
            return Err(SkillModuleError::Other {
                message: format!(
                    "skill directory does not exist: {}",
                    skill_dir.display()
                ),
            });
        }

        Self::copy_dir_recursive(source_dir, &skill_dir, source_dir, main_file_name)
    }

    /// Recursively copy files from `current` (within `source_root`) into
    /// `target_root`, skipping generated/hidden files.
    fn copy_dir_recursive(
        source_root: &Path,
        target_root: &Path,
        current: &Path,
        main_file_name: Option<&str>,
    ) -> Result<(), SkillModuleError> {
        /// Files/directories generated by the ingestion pipeline that must
        /// never be overwritten by companion copies.
        const SKIP_NAMES: &[&str] = &["skill.toml", "root.md", "lineage.toml", "details"];

        let entries = std::fs::read_dir(current).map_err(|e| SkillModuleError::Other {
            message: format!("failed to read source dir {}: {e}", current.display()),
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| SkillModuleError::Other {
                message: format!("failed to read dir entry: {e}"),
            })?;
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Skip hidden files/dirs.
            if name_str.starts_with('.') {
                continue;
            }

            // Skip the main file that was already processed.
            if let Some(main) = main_file_name {
                if name_str == main {
                    continue;
                }
            }

            let abs_path = entry.path();
            let rel = abs_path
                .strip_prefix(source_root)
                .unwrap_or(&abs_path);

            let target_path = target_root.join(rel);

            // Never overwrite generated files.
            if SKIP_NAMES.contains(&name_str.as_ref()) && target_path.exists() {
                continue;
            }

            let meta = entry.metadata().map_err(|e| SkillModuleError::Other {
                message: format!("failed to read metadata for {}: {e}", abs_path.display()),
            })?;

            if meta.is_dir() {
                std::fs::create_dir_all(&target_path).map_err(|e| SkillModuleError::Other {
                    message: format!(
                        "failed to create companion dir {}: {e}",
                        target_path.display()
                    ),
                })?;
                Self::copy_dir_recursive(source_root, target_root, &abs_path, main_file_name)?;
            } else {
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| SkillModuleError::Other {
                        message: format!(
                            "failed to create parent dir {}: {e}",
                            parent.display()
                        ),
                    })?;
                }
                std::fs::copy(&abs_path, &target_path).map_err(|e| SkillModuleError::Other {
                    message: format!(
                        "failed to copy {} → {}: {e}",
                        abs_path.display(),
                        target_path.display()
                    ),
                })?;
            }
        }

        Ok(())
    }

    /// Get the base path of this store.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }
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
            security: None,
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

        // Verify core files exist
        let skill_dir = tmp.path().join("roundtrip-test");
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("root.md").exists());
        assert!(skill_dir.join("lineage.toml").exists());
        // details/ directory is created but sub-doc files are written
        // separately via write_sub_document().
        assert!(skill_dir.join("details").exists());

        // Write a sub-document and verify it persists.
        store
            .write_sub_document(
                "roundtrip-test",
                "details/sub-1.md",
                "# Sub Document 1\n\nActual content.",
            )
            .unwrap();
        assert!(skill_dir.join("details").join("sub-1.md").exists());

        // Read back and verify
        let loaded = store.load_skill("roundtrip-test").unwrap();
        assert_eq!(loaded.name, "roundtrip-test");
        assert_eq!(loaded.description, "Test skill: roundtrip-test");
        // root_content is loaded from root.md
        assert_eq!(
            loaded.root_content,
            "Root content for testing persistence."
        );

        // Verify sub-document content can be read back
        let sub_docs = store.load_sub_documents("roundtrip-test").unwrap();
        assert_eq!(sub_docs.len(), 1);
        assert_eq!(sub_docs[0].0, "details/sub-1.md");
        assert!(sub_docs[0].1.contains("Actual content."));
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

    /// Companion files from a source directory are copied into the skill
    /// directory without overwriting generated files.
    #[test]
    fn test_copy_companion_files() {
        // Set up store and register a skill.
        let store_tmp = tempfile::tempdir().unwrap();
        let store = FilesystemSkillStore::new(store_tmp.path()).unwrap();
        store.save_skill(&test_manifest("xlsx")).unwrap();

        // Verify generated files exist.
        let skill_dir = store_tmp.path().join("xlsx");
        assert!(skill_dir.join("skill.toml").exists());
        assert!(skill_dir.join("root.md").exists());
        let original_toml = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();

        // Build a source directory resembling the xlsx skill.
        let src_tmp = tempfile::tempdir().unwrap();
        let src = src_tmp.path();
        std::fs::write(src.join("SKILL.md"), "# xlsx skill").unwrap();
        std::fs::write(src.join("LICENSE.txt"), "MIT").unwrap();
        std::fs::create_dir_all(src.join("scripts/office")).unwrap();
        std::fs::write(src.join("scripts/recalc.py"), "print('recalc')").unwrap();
        std::fs::write(src.join("scripts/office/soffice.py"), "print('soffice')").unwrap();

        // Also add a file with a generated name to test skip logic.
        std::fs::write(src.join("skill.toml"), "SHOULD NOT OVERWRITE").unwrap();

        // Copy companions.
        store
            .copy_companion_files("xlsx", src, Some("SKILL.md"))
            .unwrap();

        // Companion files should be present.
        assert!(skill_dir.join("LICENSE.txt").exists());
        assert_eq!(
            std::fs::read_to_string(skill_dir.join("LICENSE.txt")).unwrap(),
            "MIT"
        );
        assert!(skill_dir.join("scripts/recalc.py").exists());
        assert!(skill_dir.join("scripts/office/soffice.py").exists());

        // Main file should NOT be copied.
        assert!(!skill_dir.join("SKILL.md").exists());

        // Generated skill.toml should NOT be overwritten.
        let after_toml = std::fs::read_to_string(skill_dir.join("skill.toml")).unwrap();
        assert_eq!(original_toml, after_toml);
    }
}
