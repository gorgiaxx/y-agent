//! Skill validation: enforces registration rules from the design.
//!
//! The validator checks 7 rules before a skill can be registered:
//! 1. Format validation: `skill.toml` + `root.md` must exist
//! 2. Schema validation: parse `skill.toml`, required fields present
//! 3. Root token limit: `root.md` ≤ `max_root_tokens`
//! 4. Security constraints: all security flags `false` unless approved
//! 5. Unique name: no duplicate names in registry
//! 6. Lineage required: `lineage.toml` must exist
//! 7. Reference resolution: `[tool:X]`, `[skill:X]`, `[knowledge:X]` refs resolve

use std::collections::HashSet;
use std::path::Path;

use y_core::skill::SkillManifest;

use crate::config::SkillConfig;
use crate::manifest::estimate_tokens;

/// A validation error with a descriptive message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Which rule was violated.
    pub rule: ValidationRule,
    /// Human-readable description.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)
    }
}

/// The validation rules that can be violated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationRule {
    /// `skill.toml` + `root.md` must exist.
    FormatValidation,
    /// `skill.toml` must parse and have required fields.
    SchemaValidation,
    /// `root.md` must not exceed token limit.
    RootTokenLimit,
    /// Security flags must all be `false` unless approved.
    SecurityConstraints,
    /// Skill name must be unique in the registry.
    UniqueName,
    /// `lineage.toml` must exist.
    LineageRequired,
    /// All `[tool:X]`, `[skill:X]`, `[knowledge:X]` references must resolve.
    ReferenceResolution,
}

impl std::fmt::Display for ValidationRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::FormatValidation => "format",
            Self::SchemaValidation => "schema",
            Self::RootTokenLimit => "root_token_limit",
            Self::SecurityConstraints => "security",
            Self::UniqueName => "unique_name",
            Self::LineageRequired => "lineage_required",
            Self::ReferenceResolution => "reference",
        };
        f.write_str(s)
    }
}

/// Validates skills against registration rules.
#[derive(Debug)]
pub struct SkillValidator {
    config: SkillConfig,
}

impl SkillValidator {
    /// Create a new validator with the given configuration.
    pub fn new(config: SkillConfig) -> Self {
        Self { config }
    }

    /// Validate a skill directory on disk (checks format, lineage, etc.).
    pub fn validate_directory(&self, skill_dir: &Path) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Rule 1: Format validation
        let toml_path = skill_dir.join("skill.toml");
        let root_path = skill_dir.join("root.md");

        if !toml_path.exists() {
            errors.push(ValidationError {
                rule: ValidationRule::FormatValidation,
                message: format!("skill.toml not found in {}", skill_dir.display()),
            });
        }
        if !root_path.exists() {
            errors.push(ValidationError {
                rule: ValidationRule::FormatValidation,
                message: format!("root.md not found in {}", skill_dir.display()),
            });
        }

        // Rule 3: Root token limit (if root.md exists)
        if root_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&root_path) {
                let tokens = estimate_tokens(&content);
                if tokens > self.config.max_root_tokens {
                    errors.push(ValidationError {
                        rule: ValidationRule::RootTokenLimit,
                        message: format!(
                            "root.md is {} tokens, max is {}",
                            tokens, self.config.max_root_tokens
                        ),
                    });
                }
            }
        }

        // Rule 6: Lineage required
        let lineage_path = skill_dir.join("lineage.toml");
        if !lineage_path.exists() {
            errors.push(ValidationError {
                rule: ValidationRule::LineageRequired,
                message: format!("lineage.toml not found in {}", skill_dir.display()),
            });
        }

        errors
    }

    /// Validate a manifest in memory (checks schema, security, references).
    pub fn validate_manifest(
        &self,
        manifest: &SkillManifest,
        existing_names: &HashSet<String>,
        registered_tools: &HashSet<String>,
        registered_skills: &HashSet<String>,
        registered_knowledge: &HashSet<String>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Rule 2: Schema validation — required fields
        if manifest.name.is_empty() {
            errors.push(ValidationError {
                rule: ValidationRule::SchemaValidation,
                message: "skill name is empty".to_string(),
            });
        }
        if manifest.description.is_empty() {
            errors.push(ValidationError {
                rule: ValidationRule::SchemaValidation,
                message: "skill description is empty".to_string(),
            });
        }

        // Rule 3: Root token limit
        let tokens = estimate_tokens(&manifest.root_content);
        if tokens > self.config.max_root_tokens {
            errors.push(ValidationError {
                rule: ValidationRule::RootTokenLimit,
                message: format!(
                    "root content is {} tokens, max is {}",
                    tokens, self.config.max_root_tokens
                ),
            });
        }

        // Rule 4: Security constraints
        if let Some(ref security) = manifest.security {
            if security.allows_external_calls {
                errors.push(ValidationError {
                    rule: ValidationRule::SecurityConstraints,
                    message: "allows_external_calls is true (requires explicit approval)"
                        .to_string(),
                });
            }
            if security.allows_file_operations {
                errors.push(ValidationError {
                    rule: ValidationRule::SecurityConstraints,
                    message: "allows_file_operations is true (requires explicit approval)"
                        .to_string(),
                });
            }
            if security.allows_code_execution {
                errors.push(ValidationError {
                    rule: ValidationRule::SecurityConstraints,
                    message: "allows_code_execution is true (requires explicit approval)"
                        .to_string(),
                });
            }
        }

        // Rule 5: Unique name
        if existing_names.contains(&manifest.name) {
            errors.push(ValidationError {
                rule: ValidationRule::UniqueName,
                message: format!("skill name '{}' already exists in registry", manifest.name),
            });
        }

        // Rule 7: Reference resolution
        if let Some(ref refs) = manifest.references {
            for tool_ref in &refs.tools {
                if !registered_tools.contains(tool_ref) {
                    errors.push(ValidationError {
                        rule: ValidationRule::ReferenceResolution,
                        message: format!("unresolved tool reference: [tool:{tool_ref}]"),
                    });
                }
            }
            for skill_ref in &refs.skills {
                if !registered_skills.contains(skill_ref) {
                    errors.push(ValidationError {
                        rule: ValidationRule::ReferenceResolution,
                        message: format!("unresolved skill reference: [skill:{skill_ref}]"),
                    });
                }
            }
            for kb_ref in &refs.knowledge_bases {
                if !registered_knowledge.contains(kb_ref) {
                    errors.push(ValidationError {
                        rule: ValidationRule::ReferenceResolution,
                        message: format!("unresolved knowledge reference: [knowledge:{kb_ref}]"),
                    });
                }
            }
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::skill::{SkillManifest, SkillReferences, SkillSecurityConfig, SkillVersion};
    use y_core::types::{now, SkillId};

    fn test_manifest(name: &str) -> SkillManifest {
        let now = now();
        SkillManifest {
            id: SkillId::new(),
            name: name.to_string(),
            description: "A test skill".to_string(),
            version: SkillVersion(String::new()),
            tags: vec![],
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: "Short root content.".to_string(),
            sub_documents: vec![],
            token_estimate: 5,
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

    fn empty_sets() -> (
        HashSet<String>,
        HashSet<String>,
        HashSet<String>,
        HashSet<String>,
    ) {
        (
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
            HashSet::new(),
        )
    }

    /// T-SK-S2-01: Validator rejects missing root.md.
    #[test]
    fn test_validator_rejects_missing_root_md() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // Only create skill.toml, not root.md
        std::fs::write(skill_dir.join("skill.toml"), "name = \"test\"").unwrap();

        let validator = SkillValidator::new(SkillConfig::default());
        let errors = validator.validate_directory(&skill_dir);

        let format_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::FormatValidation)
            .collect();
        assert!(
            format_errors.iter().any(|e| e.message.contains("root.md")),
            "expected root.md error, got: {format_errors:?}"
        );
    }

    /// T-SK-S2-02: Validator rejects duplicate skill name.
    #[test]
    fn test_validator_rejects_duplicate_name() {
        let validator = SkillValidator::new(SkillConfig::default());
        let manifest = test_manifest("duplicate-skill");

        let mut existing = HashSet::new();
        existing.insert("duplicate-skill".to_string());

        let (tools, skills, kb) = (HashSet::new(), HashSet::new(), HashSet::new());
        let errors = validator.validate_manifest(&manifest, &existing, &tools, &skills, &kb);

        assert!(
            errors.iter().any(|e| e.rule == ValidationRule::UniqueName),
            "expected unique name error, got: {errors:?}"
        );
    }

    /// T-SK-S2-03: Validator rejects oversized root document.
    #[test]
    fn test_validator_rejects_oversized_root() {
        let validator = SkillValidator::new(SkillConfig::default());
        let mut manifest = test_manifest("big-skill");
        manifest.root_content = "x".repeat(10_000); // ~2500 tokens > 2000

        let (names, tools, skills, kb) = empty_sets();
        let errors = validator.validate_manifest(&manifest, &names, &tools, &skills, &kb);

        assert!(
            errors
                .iter()
                .any(|e| e.rule == ValidationRule::RootTokenLimit),
            "expected root token limit error, got: {errors:?}"
        );
    }

    /// T-SK-S2-04: Validator detects broken [tool:X] references.
    #[test]
    fn test_validator_detects_broken_references() {
        let validator = SkillValidator::new(SkillConfig::default());
        let mut manifest = test_manifest("ref-skill");
        manifest.references = Some(SkillReferences {
            tools: vec!["nonexistent-tool".to_string()],
            skills: vec!["nonexistent-skill".to_string()],
            knowledge_bases: vec![],
        });

        let (names, tools, skills, kb) = empty_sets();
        let errors = validator.validate_manifest(&manifest, &names, &tools, &skills, &kb);

        let ref_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::ReferenceResolution)
            .collect();
        assert_eq!(
            ref_errors.len(),
            2,
            "expected 2 ref errors, got: {ref_errors:?}"
        );
        assert!(ref_errors
            .iter()
            .any(|e| e.message.contains("nonexistent-tool")));
        assert!(ref_errors
            .iter()
            .any(|e| e.message.contains("nonexistent-skill")));
    }

    /// Security constraints: `allows_external_calls` triggers error.
    #[test]
    fn test_validator_rejects_insecure_flags() {
        let validator = SkillValidator::new(SkillConfig::default());
        let mut manifest = test_manifest("insecure-skill");
        manifest.security = Some(SkillSecurityConfig {
            allows_external_calls: true,
            allows_file_operations: false,
            allows_code_execution: true,
            max_delegation_depth: 0,
        });

        let (names, tools, skills, kb) = empty_sets();
        let errors = validator.validate_manifest(&manifest, &names, &tools, &skills, &kb);

        let security_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::SecurityConstraints)
            .collect();
        assert_eq!(
            security_errors.len(),
            2,
            "expected 2 security errors, got: {security_errors:?}"
        );
    }

    /// A valid manifest with no issues produces zero errors.
    #[test]
    fn test_validator_accepts_valid_manifest() {
        let validator = SkillValidator::new(SkillConfig::default());
        let manifest = test_manifest("valid-skill");

        let (names, tools, skills, kb) = empty_sets();
        let errors = validator.validate_manifest(&manifest, &names, &tools, &skills, &kb);

        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }
}
