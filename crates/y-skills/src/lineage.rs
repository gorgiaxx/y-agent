//! Lineage tracking: records the provenance of transformed skills.
//!
//! Each proprietary skill has a `lineage.toml` that records where it came from,
//! how it was transformed, and what model was used. This provides full
//! auditability for the skill transformation pipeline.

use serde::{Deserialize, Serialize};

/// A lineage record tracking the transformation provenance of a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageRecord {
    /// Original source file path or URL.
    pub source_path: String,
    /// SHA-256 hash of the original source content.
    pub source_hash: String,
    /// Original source format (e.g., `markdown`, `yaml`, `json`).
    pub source_format: String,
    /// Format detected by the ingestion pipeline.
    pub detected_format: String,
    /// LLM model used for transformation (if applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform_model: Option<String>,
    /// Date/time of transformation.
    pub transform_date: String,
    /// Ordered list of transformation steps applied.
    #[serde(default)]
    pub transform_steps: Vec<TransformStep>,
}

/// A single step in the transformation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformStep {
    /// Step name (e.g., `format_detection`, `content_analysis`, `decomposition`).
    pub name: String,
    /// Step description or summary.
    pub description: String,
    /// Duration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// LLM tokens used (if LLM-assisted step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u64>,
}

impl LineageRecord {
    /// Create a new lineage record for a manually-created skill.
    pub fn manual(source_path: &str, source_format: &str) -> Self {
        let source_content = std::fs::read(source_path).unwrap_or_default();
        let source_hash = format!("sha256:{}", sha2_hex(&source_content));

        Self {
            source_path: source_path.to_string(),
            source_hash,
            source_format: source_format.to_string(),
            detected_format: source_format.to_string(),
            transform_model: None,
            transform_date: chrono::Utc::now().to_rfc3339(),
            transform_steps: vec![TransformStep {
                name: "manual_import".to_string(),
                description: "Skill imported directly without LLM transformation".to_string(),
                duration_ms: None,
                tokens_used: None,
            }],
        }
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserialize from TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }
}

/// Compute SHA-256 hex digest.
fn sha2_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S2-07: Lineage TOML roundtrip serialization.
    #[test]
    fn test_lineage_toml_roundtrip() {
        let record = LineageRecord {
            source_path: "/tmp/humanizer.md".to_string(),
            source_hash: "sha256:abc123def456".to_string(),
            source_format: "markdown".to_string(),
            detected_format: "markdown".to_string(),
            transform_model: Some("gpt-4o".to_string()),
            transform_date: "2026-03-10T12:00:00Z".to_string(),
            transform_steps: vec![
                TransformStep {
                    name: "format_detection".to_string(),
                    description: "Detected Markdown format via extension and heading patterns"
                        .to_string(),
                    duration_ms: Some(5),
                    tokens_used: None,
                },
                TransformStep {
                    name: "content_analysis".to_string(),
                    description: "LLM analyzed content structure and classification".to_string(),
                    duration_ms: Some(3200),
                    tokens_used: Some(1500),
                },
                TransformStep {
                    name: "decomposition".to_string(),
                    description: "Split into root + 3 sub-documents".to_string(),
                    duration_ms: Some(4100),
                    tokens_used: Some(2000),
                },
            ],
        };

        let toml_str = record.to_toml().unwrap();
        assert!(toml_str.contains("source_path"));
        assert!(toml_str.contains("format_detection"));

        let reparsed = LineageRecord::from_toml(&toml_str).unwrap();
        assert_eq!(reparsed.source_path, "/tmp/humanizer.md");
        assert_eq!(reparsed.source_hash, "sha256:abc123def456");
        assert_eq!(reparsed.source_format, "markdown");
        assert_eq!(reparsed.transform_model.as_deref(), Some("gpt-4o"));
        assert_eq!(reparsed.transform_steps.len(), 3);
        assert_eq!(reparsed.transform_steps[0].name, "format_detection");
        assert_eq!(reparsed.transform_steps[1].tokens_used, Some(1500));
    }

    /// Lineage record without optional fields serializes cleanly.
    #[test]
    fn test_lineage_minimal_roundtrip() {
        let record = LineageRecord {
            source_path: "inline".to_string(),
            source_hash: "sha256:000".to_string(),
            source_format: "toml".to_string(),
            detected_format: "toml".to_string(),
            transform_model: None,
            transform_date: "2026-03-10T12:00:00Z".to_string(),
            transform_steps: vec![],
        };

        let toml_str = record.to_toml().unwrap();
        // transform_model should not appear (skip_serializing_if)
        assert!(!toml_str.contains("transform_model"));

        let reparsed = LineageRecord::from_toml(&toml_str).unwrap();
        assert!(reparsed.transform_model.is_none());
        assert!(reparsed.transform_steps.is_empty());
    }
}
