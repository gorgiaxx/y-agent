//! Multi-dimensional document metadata extracted by LLM.
//!
//! Provides the [`DocumentMetadata`] struct for structured classification
//! of knowledge entries across multiple dimensions: document type, industry,
//! sub-category, and LLM-interpreted title.
//!
//! All fields are `Option` for graceful degradation when LLM is unavailable.

use serde::{Deserialize, Serialize};

/// Multi-dimensional document metadata extracted by LLM.
///
/// Replaces the single-dimension `tags: Vec<String>` model with structured
/// classification across several orthogonal axes. Populated by the
/// `knowledge-metadata` sub-agent during ingestion.
///
/// # Dimensions
///
/// | Field              | Description                                              | Examples                                      |
/// |--------------------|----------------------------------------------------------|-----------------------------------------------|
/// | `document_type`    | What kind of document this is                            | standards, paper, manual, novel, tutorial      |
/// | `industry`         | Broad industry/domain classification                     | cybersecurity, finance, medicine, law          |
/// | `subcategory`      | Fine-grained category within the industry                | cryptography, fuzzing, pentest                 |
/// | `interpreted_title`| LLM's understanding of the document's actual title       | (language matches document content)            |
/// | `title_language`   | ISO 639-1 code for the interpreted title                 | en, zh, ja, ko, de, fr                         |
/// | `original_filename`| Preserved original filename from ingestion source        | ISO-26262-Part6.pdf                            |
/// | `topics`           | Free-form topic tags (replaces old `tags` field)         | error-handling, async-runtime, tokio           |
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DocumentMetadata {
    /// Document type classification.
    ///
    /// Examples: standards, presentations, papers, notes, manuals, datasheets,
    /// novels, tutorials, specifications, reports, `reference_guides`,
    /// whitepapers, `case_studies`, `technical_documentation`, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_type: Option<String>,

    /// Industry/domain classification.
    ///
    /// Examples: finance, biology, medicine, architecture, `computer_science`,
    /// cybersecurity, design, law, sociology, psychology, engineering,
    /// education, linguistics, physics, chemistry, mathematics, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub industry: Option<String>,

    /// Fine-grained sub-category within the industry.
    ///
    /// Examples (for cybersecurity): cryptography, fuzzing, pentest,
    /// `malware_analysis`, `threat_intelligence`, `vulnerability_research`, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcategory: Option<String>,

    /// LLM-interpreted document title.
    ///
    /// May differ from the original filename. The language is determined
    /// by the LLM based on the document's primary content language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpreted_title: Option<String>,

    /// ISO 639-1 language code for the interpreted title.
    ///
    /// Examples: "en", "zh", "ja", "ko", "de", "fr", "es".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_language: Option<String>,

    /// Original filename preserved from the ingestion source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_filename: Option<String>,

    /// Free-form topic tags extracted by the metadata agent.
    ///
    /// Replaces the old single-dimension `tags` field semantics.
    /// Tags are lowercase, hyphen-separated (e.g., "error-handling").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
}

impl DocumentMetadata {
    /// Check if all metadata fields are empty / unset.
    pub fn is_empty(&self) -> bool {
        self.document_type.is_none()
            && self.industry.is_none()
            && self.subcategory.is_none()
            && self.interpreted_title.is_none()
            && self.title_language.is_none()
            && self.original_filename.is_none()
            && self.topics.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_empty() {
        let meta = DocumentMetadata::default();
        assert!(meta.is_empty());
    }

    #[test]
    fn test_serde_roundtrip() {
        let meta = DocumentMetadata {
            document_type: Some("standards".to_string()),
            industry: Some("cybersecurity".to_string()),
            subcategory: Some("cryptography".to_string()),
            interpreted_title: Some("Applied Cryptography Guide".to_string()),
            title_language: Some("en".to_string()),
            original_filename: Some("crypto-guide.md".to_string()),
            topics: vec!["aes".to_string(), "rsa".to_string(), "tls".to_string()],
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let deserialized: DocumentMetadata = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(meta, deserialized);
        assert!(!meta.is_empty());
    }

    #[test]
    fn test_serde_empty_fields_skipped() {
        let meta = DocumentMetadata::default();
        let json = serde_json::to_string(&meta).expect("serialize");
        // Empty fields should be skipped.
        assert!(!json.contains("document_type"));
        assert!(!json.contains("industry"));
    }

    #[test]
    fn test_deserialize_with_missing_fields() {
        // Backward compatibility: missing fields default to None/empty.
        let json = r#"{"document_type": "paper"}"#;
        let meta: DocumentMetadata = serde_json::from_str(json).expect("deserialize");
        assert_eq!(meta.document_type, Some("paper".to_string()));
        assert!(meta.industry.is_none());
        assert!(meta.topics.is_empty());
    }

    #[test]
    fn test_partial_metadata_not_empty() {
        let meta = DocumentMetadata {
            industry: Some("finance".to_string()),
            ..Default::default()
        };
        assert!(!meta.is_empty());
    }
}
