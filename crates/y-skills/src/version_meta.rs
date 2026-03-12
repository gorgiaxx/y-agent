//! Version metadata: tracks provenance and evaluation data per version snapshot.
//!
//! Each version in the CAS has a `version-meta.toml` that records its
//! parent hash, creation context, and evaluation metrics.

use serde::{Deserialize, Serialize};

/// Source type for how a version was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionSourceType {
    /// Created by the transformation pipeline.
    Transformation,
    /// Created by the evolution refiner.
    Evolution,
    /// Rolled back from a later version.
    Rollback,
    /// Manually edited by a user.
    ManualEdit,
}

/// Evaluation metrics collected over a version's lifetime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionEvaluation {
    /// Number of times this version has been used since creation.
    #[serde(default)]
    pub uses_since_creation: u64,
    /// Success rate of tasks that used this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
    /// Average relevance score when this version was selected by search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_relevance_score: Option<f64>,
}

/// Metadata for a single version snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionMeta {
    /// Content-addressable hash of this version.
    pub hash: String,
    /// Hash of the parent version (None for the first version).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_hash: Option<String>,
    /// When this version was created.
    pub created_at: String,
    /// What created this version.
    pub created_by: String,
    /// How this version was produced.
    pub source_type: VersionSourceType,
    /// Evaluation metrics.
    #[serde(default)]
    pub evaluation: VersionEvaluation,
}

impl VersionMeta {
    /// Create metadata for a new version.
    pub fn new(
        hash: &str,
        parent_hash: Option<String>,
        source_type: VersionSourceType,
        created_by: &str,
    ) -> Self {
        Self {
            hash: hash.to_string(),
            parent_hash,
            created_at: chrono::Utc::now().to_rfc3339(),
            created_by: created_by.to_string(),
            source_type,
            evaluation: VersionEvaluation::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S3-04: `VersionMeta` TOML roundtrip.
    #[test]
    fn test_version_meta_toml_roundtrip() {
        let meta = VersionMeta {
            hash: "abc123def456".to_string(),
            parent_hash: Some("parent789".to_string()),
            created_at: "2026-03-10T12:00:00Z".to_string(),
            created_by: "transformation-pipeline".to_string(),
            source_type: VersionSourceType::Transformation,
            evaluation: VersionEvaluation {
                uses_since_creation: 42,
                success_rate: Some(0.95),
                average_relevance_score: Some(0.88),
            },
        };

        let toml_str = meta.to_toml().unwrap();
        assert!(toml_str.contains("abc123def456"));
        assert!(toml_str.contains("parent789"));

        let reparsed = VersionMeta::from_toml(&toml_str).unwrap();
        assert_eq!(reparsed.hash, "abc123def456");
        assert_eq!(reparsed.parent_hash.as_deref(), Some("parent789"));
        assert_eq!(reparsed.source_type, VersionSourceType::Transformation);
        assert_eq!(reparsed.evaluation.uses_since_creation, 42);
        assert_eq!(reparsed.evaluation.success_rate, Some(0.95));
    }

    /// T-SK-S3-05: `parent_hash` chain is correct across versions.
    #[test]
    fn test_version_meta_parent_chain() {
        let v1 = VersionMeta::new(
            "hash_v1",
            None,
            VersionSourceType::Transformation,
            "pipeline",
        );
        assert!(v1.parent_hash.is_none());

        let v2 = VersionMeta::new(
            "hash_v2",
            Some(v1.hash.clone()),
            VersionSourceType::Evolution,
            "refiner",
        );
        assert_eq!(v2.parent_hash.as_deref(), Some("hash_v1"));

        let v3 = VersionMeta::new(
            "hash_v3",
            Some(v2.hash.clone()),
            VersionSourceType::ManualEdit,
            "user",
        );
        assert_eq!(v3.parent_hash.as_deref(), Some("hash_v2"));

        // Rollback to v1 creates a new version pointing at v3 as parent
        let v4_rollback = VersionMeta::new(
            "hash_v1", // same hash as v1
            Some(v3.hash.clone()),
            VersionSourceType::Rollback,
            "user",
        );
        assert_eq!(v4_rollback.parent_hash.as_deref(), Some("hash_v3"));
        assert_eq!(v4_rollback.source_type, VersionSourceType::Rollback);
    }

    /// Minimal version meta without optional fields serializes cleanly.
    #[test]
    fn test_version_meta_minimal() {
        let meta = VersionMeta::new("hash_min", None, VersionSourceType::ManualEdit, "user");

        let toml_str = meta.to_toml().unwrap();
        assert!(!toml_str.contains("parent_hash"));
        assert!(!toml_str.contains("success_rate"));

        let reparsed = VersionMeta::from_toml(&toml_str).unwrap();
        assert!(reparsed.parent_hash.is_none());
        assert_eq!(reparsed.evaluation.uses_since_creation, 0);
    }
}
