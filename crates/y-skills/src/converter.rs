//! Format converter: produces the proprietary skill directory structure.
//!
//! Converts decomposed + linked content into the canonical format:
//! `skill.toml`, `root.md`, `details/*.md`, `lineage.toml`.

use std::fmt::Write;
use std::path::Path;

use crate::decomposer::DecomposedSkill;
use crate::error::SkillModuleError;
use crate::lineage::LineageRecord;

/// Result of a format conversion.
#[derive(Debug)]
pub struct ConversionResult {
    /// Path to the created skill directory.
    pub skill_dir: std::path::PathBuf,
    /// Number of files written.
    pub files_written: usize,
}

/// Converts decomposed skills into the proprietary directory format.
#[derive(Debug)]
pub struct FormatConverter;

impl FormatConverter {
    /// Create a new converter.
    pub fn new() -> Self {
        Self
    }

    /// Convert a decomposed skill into a proprietary directory.
    ///
    /// Creates the following layout:
    /// ```text
    /// <base>/<name>/
    ///   skill.toml     — manifest metadata
    ///   root.md        — root document
    ///   details/*.md   — sub-documents (if any)
    ///   lineage.toml   — provenance record
    /// ```
    pub fn convert(
        &self,
        base_path: &Path,
        name: &str,
        decomposed: &DecomposedSkill,
        lineage: &LineageRecord,
    ) -> Result<ConversionResult, SkillModuleError> {
        let skill_dir = base_path.join(name);
        std::fs::create_dir_all(&skill_dir).map_err(|e| SkillModuleError::Other {
            message: format!("failed to create skill dir: {e}"),
        })?;

        let mut files_written = 0usize;

        // Write skill.toml manifest
        let manifest_content = Self::build_manifest(name, decomposed);
        std::fs::write(skill_dir.join("skill.toml"), &manifest_content).map_err(|e| {
            SkillModuleError::Other {
                message: format!("failed to write skill.toml: {e}"),
            }
        })?;
        files_written += 1;

        // Write root.md
        std::fs::write(skill_dir.join("root.md"), &decomposed.root_content).map_err(|e| {
            SkillModuleError::Other {
                message: format!("failed to write root.md: {e}"),
            }
        })?;
        files_written += 1;

        // Write sub-documents
        if !decomposed.sub_documents.is_empty() {
            let details_dir = skill_dir.join("details");
            std::fs::create_dir_all(&details_dir).map_err(|e| SkillModuleError::Other {
                message: format!("failed to create details dir: {e}"),
            })?;

            for sub in &decomposed.sub_documents {
                let filename = format!("{}.md", sub.id);
                std::fs::write(details_dir.join(&filename), &sub.content).map_err(|e| {
                    SkillModuleError::Other {
                        message: format!("failed to write {filename}: {e}"),
                    }
                })?;
                files_written += 1;
            }
        }

        // Write lineage.toml
        let lineage_content = lineage.to_toml().map_err(|e| SkillModuleError::Other {
            message: format!("failed to serialize lineage: {e}"),
        })?;
        std::fs::write(skill_dir.join("lineage.toml"), &lineage_content).map_err(|e| {
            SkillModuleError::Other {
                message: format!("failed to write lineage.toml: {e}"),
            }
        })?;
        files_written += 1;

        Ok(ConversionResult {
            skill_dir,
            files_written,
        })
    }

    fn build_manifest(name: &str, decomposed: &DecomposedSkill) -> String {
        let root_tokens = crate::manifest::estimate_tokens(&decomposed.root_content);
        let mut manifest = format!(
            "[skill]\n\
             name = \"{name}\"\n\
             version = \"1.0.0\"\n\
             root_content = \"root.md\"\n\
             root_token_estimate = {root_tokens}\n"
        );

        if !decomposed.sub_documents.is_empty() {
            manifest.push('\n');
            for sub in &decomposed.sub_documents {
                let _ = write!(
                    manifest,
                    "[[skill.sub_documents]]\n\
                     id = \"{}\"\n\
                     title = \"{}\"\n\
                     load_condition = \"on_demand\"\n\
                     token_estimate = {}\n\n",
                    sub.id, sub.title, sub.token_estimate
                );
            }
        }

        manifest
    }
}

impl Default for FormatConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decomposer::{DecomposedSkill, SubDocEntry};

    /// T-SK-S5-06: Converter produces valid proprietary directory.
    #[test]
    fn test_converter_produces_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let converter = FormatConverter::new();

        let decomposed = DecomposedSkill {
            root_content: "# My Skill\n\nRoot content here.".to_string(),
            sub_documents: vec![SubDocEntry {
                id: "sub_01".to_string(),
                title: "Details".to_string(),
                content: "## Details\n\nDetailed content.".to_string(),
                token_estimate: 10,
                level: 1,
            }],
            tree_index: vec![("sub_01".to_string(), "Details".to_string())],
        };

        let lineage = LineageRecord::manual("test", "markdown");
        let result = converter
            .convert(tmp.path(), "my-skill", &decomposed, &lineage)
            .unwrap();

        // Verify files
        assert!(result.skill_dir.join("skill.toml").exists());
        assert!(result.skill_dir.join("root.md").exists());
        assert!(result.skill_dir.join("details/sub_01.md").exists());
        assert!(result.skill_dir.join("lineage.toml").exists());
        assert_eq!(result.files_written, 4);

        // Verify manifest content
        let manifest = std::fs::read_to_string(result.skill_dir.join("skill.toml")).unwrap();
        assert!(manifest.contains("name = \"my-skill\""));
        assert!(manifest.contains("sub_documents"));
    }

    /// Converter handles no sub-documents.
    #[test]
    fn test_converter_no_subdocs() {
        let tmp = tempfile::tempdir().unwrap();
        let converter = FormatConverter::new();

        let decomposed = DecomposedSkill {
            root_content: "Simple skill content.".to_string(),
            sub_documents: vec![],
            tree_index: vec![],
        };

        let lineage = LineageRecord::manual("test", "plaintext");
        let result = converter
            .convert(tmp.path(), "simple", &decomposed, &lineage)
            .unwrap();

        assert_eq!(result.files_written, 3); // skill.toml + root.md + lineage.toml
        assert!(!result.skill_dir.join("details").exists());
    }
}
