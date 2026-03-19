//! Manifest parsing: TOML → `SkillManifest` with token estimation.
//!
//! Parses TOML skill manifests in two formats:
//! - **Nested format** (design-aligned): `[skill]`, `[skill.classification]`, etc.
//! - **Legacy flat format**: top-level `name`, `description`, `root_content`
//!
//! The parser auto-detects the format and handles both transparently.

use y_core::skill::{
    SkillClassification, SkillClassificationType, SkillConstraints, SkillError, SkillManifest,
    SkillReferences, SkillSecurityConfig, SkillState, SkillVersion, SubDocumentRef,
};
use y_core::types::{now, SkillId};

use crate::config::SkillConfig;
use crate::error::SkillModuleError;

// ---------------------------------------------------------------------------
// Nested TOML format (design-aligned `skill.toml`)
// ---------------------------------------------------------------------------

/// Top-level wrapper for the nested `[skill]` format.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedTomlWrapper {
    skill: NestedTomlSkill,
}

/// The `[skill]` table in the nested format.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedTomlSkill {
    name: String,
    #[serde(default)]
    version: Option<String>,
    description: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    source_format: Option<String>,
    #[serde(default)]
    source_hash: Option<String>,
    #[serde(default)]
    created: Option<String>,

    #[serde(default)]
    classification: Option<NestedClassification>,
    #[serde(default)]
    constraints: Option<NestedConstraints>,
    #[serde(default)]
    root: Option<NestedRoot>,
    #[serde(default)]
    tree: Option<NestedTree>,
    #[serde(default)]
    references: Option<NestedReferences>,
    #[serde(default)]
    security: Option<NestedSecurity>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedClassification {
    #[serde(rename = "type")]
    skill_type: SkillClassificationType,
    #[serde(default)]
    domain: Vec<String>,
    #[serde(default = "default_true")]
    atomic: bool,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedConstraints {
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    requires_language: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedRoot {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    token_count: Option<u32>,
    /// Inline root content (for self-contained manifests).
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedTree {
    #[serde(default)]
    sub_documents: Vec<NestedSubDoc>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedSubDoc {
    path: String,
    title: String,
    #[serde(default)]
    token_count: Option<u32>,
    #[serde(default)]
    load_condition: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedReferences {
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    knowledge_bases: Vec<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct NestedSecurity {
    #[serde(default)]
    allows_external_calls: bool,
    #[serde(default)]
    allows_file_operations: bool,
    #[serde(default)]
    allows_code_execution: bool,
    #[serde(default)]
    max_delegation_depth: u32,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Legacy flat TOML format
// ---------------------------------------------------------------------------

/// TOML representation of a skill manifest for deserialization (legacy flat format).
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TomlManifest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub trigger_patterns: Vec<String>,
    #[serde(default)]
    pub knowledge_bases: Vec<String>,
    pub root_content: String,
    #[serde(default)]
    pub sub_documents: Vec<TomlSubDoc>,
}

/// TOML sub-document reference.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct TomlSubDoc {
    pub id: String,
    pub title: String,
    pub load_condition: String,
    #[serde(default)]
    pub content: String,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parses TOML manifests and creates `SkillManifest` instances.
#[derive(Debug)]
pub struct ManifestParser {
    config: SkillConfig,
}

impl ManifestParser {
    /// Create a new manifest parser.
    pub fn new(config: SkillConfig) -> Self {
        Self { config }
    }

    /// Parse a TOML string into a `SkillManifest`.
    ///
    /// Auto-detects format: tries nested `[skill]` format first, then legacy flat format.
    pub fn parse(&self, toml_str: &str) -> Result<SkillManifest, SkillModuleError> {
        // Try nested format first
        if let Ok(wrapper) = toml::from_str::<NestedTomlWrapper>(toml_str) {
            return self.parse_nested(wrapper.skill);
        }
        // Fall back to legacy flat format
        let parsed: TomlManifest = toml::from_str(toml_str)?;
        self.parse_flat(parsed)
    }

    /// Parse from the design-aligned nested `[skill]` format.
    fn parse_nested(&self, skill: NestedTomlSkill) -> Result<SkillManifest, SkillModuleError> {
        // Extract root content from the `[skill.root]` section
        let root_content = skill
            .root
            .as_ref()
            .and_then(|r| r.content.clone())
            .unwrap_or_default();

        let token_estimate = skill
            .root
            .as_ref()
            .and_then(|r| r.token_count)
            .unwrap_or_else(|| estimate_tokens(&root_content));

        // Validate token budget
        if token_estimate > self.config.max_root_tokens {
            return Err(SkillModuleError::Core(SkillError::TokenBudgetExceeded {
                actual: token_estimate,
                max: self.config.max_root_tokens,
            }));
        }

        // Extract tags from classification domain or default to empty
        let tags = skill
            .classification
            .as_ref()
            .map(|c| c.domain.clone())
            .unwrap_or_default();

        let now = now();
        let sub_documents: Vec<SubDocumentRef> = skill
            .tree
            .as_ref()
            .map(|t| {
                t.sub_documents
                    .iter()
                    .map(|sd| SubDocumentRef {
                        id: sd.path.clone(),
                        path: sd.path.clone(),
                        title: sd.title.clone(),
                        load_condition: sd
                            .load_condition
                            .clone()
                            .unwrap_or_else(|| "on demand".to_string()),
                        token_estimate: sd.token_count.unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let classification = skill.classification.map(|c| SkillClassification {
            skill_type: c.skill_type,
            domain: c.domain,
            atomic: c.atomic,
        });

        let constraints = skill.constraints.map(|c| SkillConstraints {
            max_input_tokens: c.max_input_tokens,
            max_output_tokens: c.max_output_tokens,
            requires_language: c.requires_language,
        });

        let security = skill.security.map(|s| SkillSecurityConfig {
            allows_external_calls: s.allows_external_calls,
            allows_file_operations: s.allows_file_operations,
            allows_code_execution: s.allows_code_execution,
            max_delegation_depth: s.max_delegation_depth,
        });

        let references = skill.references.map(|r| SkillReferences {
            tools: r.tools,
            skills: r.skills,
            knowledge_bases: r.knowledge_bases,
        });

        let root_path = skill.root.as_ref().and_then(|r| r.path.clone());

        Ok(SkillManifest {
            id: SkillId::new(),
            name: skill.name,
            description: skill.description,
            version: SkillVersion(skill.version.unwrap_or_default()),
            tags,
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content,
            sub_documents,
            token_estimate,
            created_at: now,
            updated_at: now,
            classification,
            constraints,
            security,
            references,
            author: skill.author,
            source_format: skill.source_format,
            source_hash: skill.source_hash,
            state: Some(SkillState::Registered),
            root_path,
        })
    }

    /// Parse from the legacy flat format.
    fn parse_flat(&self, parsed: TomlManifest) -> Result<SkillManifest, SkillModuleError> {
        let token_estimate = estimate_tokens(&parsed.root_content);

        // Validate token budget
        if token_estimate > self.config.max_root_tokens {
            return Err(SkillModuleError::Core(SkillError::TokenBudgetExceeded {
                actual: token_estimate,
                max: self.config.max_root_tokens,
            }));
        }

        let now = now();
        let sub_documents: Vec<SubDocumentRef> = parsed
            .sub_documents
            .iter()
            .map(|sd| SubDocumentRef {
                id: sd.id.clone(),
                path: String::new(),
                title: sd.title.clone(),
                load_condition: sd.load_condition.clone(),
                token_estimate: estimate_tokens(&sd.content),
            })
            .collect();

        Ok(SkillManifest {
            id: SkillId::new(),
            name: parsed.name,
            description: parsed.description,
            version: SkillVersion(String::new()),
            tags: parsed.tags,
            trigger_patterns: parsed.trigger_patterns,
            knowledge_bases: parsed.knowledge_bases,
            root_content: parsed.root_content,
            sub_documents,
            token_estimate,
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
        })
    }

    /// Serialize a `SkillManifest` back to TOML (roundtrip support).
    ///
    /// Uses the design-aligned nested `[skill]` format. Root content is NOT
    /// inlined -- it lives in the separate `root.md` file written by the store.
    pub fn to_toml(manifest: &SkillManifest) -> Result<String, SkillModuleError> {
        let nested = NestedTomlWrapper {
            skill: NestedTomlSkill {
                name: manifest.name.clone(),
                version: Some(manifest.version.0.clone()).filter(|v| !v.is_empty()),
                description: manifest.description.clone(),
                author: manifest.author.clone(),
                source_format: manifest.source_format.clone(),
                source_hash: manifest.source_hash.clone(),
                created: None,
                classification: manifest
                    .classification
                    .as_ref()
                    .map(|c| NestedClassification {
                        skill_type: c.skill_type,
                        domain: c.domain.clone(),
                        atomic: c.atomic,
                    }),
                constraints: manifest.constraints.as_ref().map(|c| NestedConstraints {
                    max_input_tokens: c.max_input_tokens,
                    max_output_tokens: c.max_output_tokens,
                    requires_language: c.requires_language.clone(),
                }),
                root: Some(NestedRoot {
                    path: manifest
                        .root_path
                        .clone()
                        .or_else(|| Some("root.md".to_string())),
                    token_count: Some(manifest.token_estimate),
                    content: None, // Content lives in root.md, not inline
                }),
                tree: if manifest.sub_documents.is_empty() {
                    None
                } else {
                    Some(NestedTree {
                        sub_documents: manifest
                            .sub_documents
                            .iter()
                            .map(|sd| NestedSubDoc {
                                path: sd.path.clone(),
                                title: sd.title.clone(),
                                token_count: Some(sd.token_estimate),
                                load_condition: Some(sd.load_condition.clone()),
                            })
                            .collect(),
                    })
                },
                references: manifest.references.as_ref().map(|r| NestedReferences {
                    tools: r.tools.clone(),
                    skills: r.skills.clone(),
                    knowledge_bases: r.knowledge_bases.clone(),
                }),
                security: manifest.security.as_ref().map(|s| NestedSecurity {
                    allows_external_calls: s.allows_external_calls,
                    allows_file_operations: s.allows_file_operations,
                    allows_code_execution: s.allows_code_execution,
                    max_delegation_depth: s.max_delegation_depth,
                }),
            },
        };

        toml::to_string_pretty(&nested).map_err(|e| SkillModuleError::ManifestParseError {
            message: e.to_string(),
        })
    }
}

/// Estimate token count from text (approximately 4 characters per token).
pub fn estimate_tokens(text: &str) -> u32 {
    // Use the common heuristic: ~4 characters per token for English text
    let chars = u32::try_from(text.len()).unwrap_or(u32::MAX);
    chars.div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> String {
        r#"
name = "rust-error-handling"
description = "Guidelines for Rust error handling patterns"
tags = ["rust", "error-handling"]
trigger_patterns = ["how to handle errors", "error pattern"]
root_content = "Use thiserror for library errors. Use anyhow for applications."

[[sub_documents]]
id = "thiserror-examples"
title = "Thiserror Examples"
load_condition = "When user asks about thiserror"
content = "Example code using thiserror derive macro."

[[sub_documents]]
id = "anyhow-examples"
title = "Anyhow Examples"
load_condition = "When user asks about anyhow"
content = "Example code using anyhow context."
"#
        .to_string()
    }

    /// Design-format nested `skill.toml` used in design documents.
    fn nested_toml() -> String {
        r#"
[skill]
name = "humanizer-zh"
version = "1.0.0"
description = "Remove AI writing artifacts from Chinese text"
author = "y-agent-transform"
source_format = "markdown"
source_hash = "sha256:abc123"

[skill.classification]
type = "llm_reasoning"
domain = ["writing", "chinese", "editing"]
tags = ["humanize", "rewrite", "chinese"]
atomic = true

[skill.constraints]
max_input_tokens = 8000
max_output_tokens = 8000
requires_language = "zh"

[skill.root]
path = "root.md"
token_count = 850
content = "Remove AI writing artifacts from Chinese text while preserving meaning."

[skill.tree]
sub_documents = [
    { path = "details/tone-guidelines.md", title = "Tone and style guidelines", token_count = 600 },
    { path = "details/common-patterns.md", title = "Common AI patterns to detect and remove", token_count = 1200 },
]

[skill.references]
tools = []
skills = []
knowledge_bases = ["chinese-writing"]

[skill.security]
allows_external_calls = false
allows_file_operations = false
allows_code_execution = false
max_delegation_depth = 0
"#
        .to_string()
    }

    /// T-SK-S1-01: Parse design-format `skill.toml` with nested sections.
    #[test]
    fn test_manifest_parse_nested_toml() {
        let parser = ManifestParser::new(SkillConfig::default());
        let manifest = parser.parse(&nested_toml()).unwrap();

        assert_eq!(manifest.name, "humanizer-zh");
        assert_eq!(
            manifest.description,
            "Remove AI writing artifacts from Chinese text"
        );
        assert_eq!(manifest.author.as_deref(), Some("y-agent-transform"));
        assert_eq!(manifest.source_format.as_deref(), Some("markdown"));
        assert_eq!(manifest.source_hash.as_deref(), Some("sha256:abc123"));

        // Classification
        let cls = manifest.classification.as_ref().unwrap();
        assert_eq!(cls.skill_type, SkillClassificationType::LlmReasoning);
        assert_eq!(cls.domain, vec!["writing", "chinese", "editing"]);
        assert!(cls.atomic);

        // Constraints
        let con = manifest.constraints.as_ref().unwrap();
        assert_eq!(con.max_input_tokens, Some(8000));
        assert_eq!(con.max_output_tokens, Some(8000));
        assert_eq!(con.requires_language.as_deref(), Some("zh"));

        // Security
        let saf = manifest.security.as_ref().unwrap();
        assert!(!saf.allows_external_calls);
        assert!(!saf.allows_file_operations);
        assert!(!saf.allows_code_execution);
        assert_eq!(saf.max_delegation_depth, 0);

        // References
        let refs = manifest.references.as_ref().unwrap();
        assert!(refs.tools.is_empty());
        assert_eq!(refs.knowledge_bases, vec!["chinese-writing"]);

        // Sub-documents from tree
        assert_eq!(manifest.sub_documents.len(), 2);
        assert_eq!(manifest.sub_documents[0].id, "details/tone-guidelines.md");
        assert_eq!(manifest.sub_documents[0].token_estimate, 600);

        // Root content + token estimate
        assert!(!manifest.root_content.is_empty());
        assert_eq!(manifest.token_estimate, 850);
    }

    /// T-SK-S1-02: Legacy flat TOML still parses correctly (backward compat).
    /// T-SKILL-001-01: Valid TOML manifest parses with all fields.
    #[test]
    fn test_manifest_parse_valid_toml() {
        let parser = ManifestParser::new(SkillConfig::default());
        let manifest = parser.parse(&valid_toml()).unwrap();

        assert_eq!(manifest.name, "rust-error-handling");
        assert_eq!(manifest.tags.len(), 2);
        assert_eq!(manifest.trigger_patterns.len(), 2);
        assert_eq!(manifest.sub_documents.len(), 2);
        assert!(!manifest.root_content.is_empty());

        // Extended fields should be None for legacy format
        assert!(manifest.classification.is_none());
        assert!(manifest.constraints.is_none());
        assert!(manifest.security.is_none());
        assert!(manifest.references.is_none());
        assert!(manifest.author.is_none());
    }

    /// T-SKILL-001-02: Token estimate is within 10% of actual.
    #[test]
    fn test_manifest_token_estimate() {
        let text = "Use thiserror for library errors. Use anyhow for applications.";
        let estimate = estimate_tokens(text);
        // ~62 chars → ~16 tokens
        assert!((14..=18).contains(&estimate), "estimate was {estimate}");
    }

    /// T-SKILL-001-03: Oversized root produces `TokenBudgetExceeded`.
    #[test]
    fn test_manifest_root_exceeds_2000_tokens() {
        let long_content = "x".repeat(10_000); // ~2500 tokens
        let toml_str = format!(
            r#"
name = "big-skill"
description = "Too large"
root_content = "{long_content}"
"#
        );

        let parser = ManifestParser::new(SkillConfig::default());
        let result = parser.parse(&toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                SkillModuleError::Core(SkillError::TokenBudgetExceeded { .. })
            ),
            "expected TokenBudgetExceeded, got {err:?}"
        );
    }

    /// T-SKILL-001-03b: Oversized root in nested format also produces `TokenBudgetExceeded`.
    #[test]
    fn test_nested_manifest_root_exceeds_2000_tokens() {
        let long_content = "x".repeat(10_000); // ~2500 tokens
        let toml_str = format!(
            r#"
[skill]
name = "big-skill"
description = "Too large"

[skill.root]
content = "{long_content}"
"#
        );

        let parser = ManifestParser::new(SkillConfig::default());
        let result = parser.parse(&toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                SkillModuleError::Core(SkillError::TokenBudgetExceeded { .. })
            ),
            "expected TokenBudgetExceeded, got {err:?}"
        );
    }

    /// T-SKILL-001-04: Sub-document references are parsed correctly.
    #[test]
    fn test_manifest_sub_document_refs() {
        let parser = ManifestParser::new(SkillConfig::default());
        let manifest = parser.parse(&valid_toml()).unwrap();

        assert_eq!(manifest.sub_documents.len(), 2);
        assert_eq!(manifest.sub_documents[0].id, "thiserror-examples");
        assert_eq!(manifest.sub_documents[1].id, "anyhow-examples");
        assert!(!manifest.sub_documents[0].load_condition.is_empty());
    }

    /// T-SKILL-001-05: TOML → struct → TOML roundtrip (identity).
    ///
    /// `to_toml()` uses the nested format and does NOT inline root_content
    /// (content lives in the separate `root.md` file). So after a pure
    /// in-memory roundtrip, `root_content` is expected to be empty.
    /// The full file-backed roundtrip is tested in `store::tests`.
    #[test]
    fn test_manifest_serialization_roundtrip() {
        let parser = ManifestParser::new(SkillConfig::default());
        let manifest = parser.parse(&valid_toml()).unwrap();

        let toml_output = ManifestParser::to_toml(&manifest).unwrap();
        let reparsed = parser.parse(&toml_output).unwrap();

        assert_eq!(manifest.name, reparsed.name);
        assert_eq!(manifest.description, reparsed.description);
        // root_content is NOT inlined in the nested TOML; it lives in root.md.
        // In-memory roundtrip yields empty root_content — the filesystem store
        // merges root.md back in during load_from_dir().
        assert!(reparsed.root_content.is_empty());
    }
}
