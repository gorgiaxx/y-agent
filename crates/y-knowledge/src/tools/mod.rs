//! Knowledge base tools for Agent use.
//!
//! Provides three built-in tools:
//! - `knowledge_search` — semantic + keyword search with resolution control
//! - `knowledge_lookup` — exact chunk lookup by ID
//! - `knowledge_ingest` — agent-driven content ingestion

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// knowledge_search
// ---------------------------------------------------------------------------

/// Input parameters for `knowledge_search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchParams {
    /// Search query string.
    pub query: String,
    /// Optional domain filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Desired resolution level: `"l0"` (summary), `"l1"` (overview), `"l2"` (full).
    #[serde(default = "default_resolution")]
    pub resolution: String,
    /// Maximum number of results (default: 5).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional collection to search within.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
}

fn default_resolution() -> String {
    "l0".to_string()
}

const fn default_limit() -> usize {
    5
}

/// A single search result item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    /// Chunk ID for follow-up `knowledge_lookup`.
    pub chunk_id: String,
    /// Document ID.
    pub document_id: String,
    /// Content at the requested resolution.
    pub content: String,
    /// Relevance score (0.0–1.0).
    pub relevance: f64,
    /// Domain classifications.
    pub domains: Vec<String>,
    /// Source title.
    pub title: String,
}

/// Output from `knowledge_search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSearchResult {
    /// Matching results.
    pub results: Vec<SearchResultItem>,
    /// Total number of matches (before limit).
    pub total_matches: usize,
    /// Search strategy used.
    pub strategy: String,
}

// ---------------------------------------------------------------------------
// knowledge_lookup
// ---------------------------------------------------------------------------

/// Input parameters for `knowledge_lookup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeLookupParams {
    /// Chunk ID to look up.
    pub chunk_id: String,
    /// Desired resolution: `"l0"`, `"l1"`, `"l2"`.
    #[serde(default = "default_resolution")]
    pub resolution: String,
}

/// Output from `knowledge_lookup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeLookupResult {
    /// Whether the chunk was found.
    pub found: bool,
    /// Chunk content at the requested resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Document metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LookupMetadata>,
}

/// Metadata for a looked-up chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupMetadata {
    pub document_id: String,
    pub title: String,
    pub source_uri: String,
    pub domains: Vec<String>,
    pub quality_score: f32,
}

// ---------------------------------------------------------------------------
// knowledge_ingest
// ---------------------------------------------------------------------------

/// Input parameters for `knowledge_ingest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeIngestParams {
    /// Source URI (file path, URL, etc.).
    pub source: String,
    /// Optional domain hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Target collection (default: "default").
    #[serde(default = "default_collection")]
    pub collection: String,
}

fn default_collection() -> String {
    "default".to_string()
}

/// Output from `knowledge_ingest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeIngestResult {
    /// Whether ingestion succeeded.
    pub success: bool,
    /// ID of the created entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// Number of chunks created.
    pub chunk_count: usize,
    /// Classified domains.
    pub domains: Vec<String>,
    /// Quality score.
    pub quality_score: f32,
    /// Status message.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Tool JSON Schemas
// ---------------------------------------------------------------------------

/// Generate the JSON Schema for `knowledge_search` parameters.
pub fn knowledge_search_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query string"
            },
            "domain": {
                "type": "string",
                "description": "Optional domain filter (e.g., 'rust', 'python')"
            },
            "resolution": {
                "type": "string",
                "enum": ["l0", "l1", "l2"],
                "description": "Content resolution: l0 (summary), l1 (overview), l2 (full)",
                "default": "l0"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of results",
                "default": 5,
                "minimum": 1,
                "maximum": 20
            },
            "collection": {
                "type": "string",
                "description": "Collection to search within"
            }
        },
        "required": ["query"]
    })
}

/// Generate the JSON Schema for `knowledge_lookup` parameters.
pub fn knowledge_lookup_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "chunk_id": {
                "type": "string",
                "description": "Chunk ID to look up"
            },
            "resolution": {
                "type": "string",
                "enum": ["l0", "l1", "l2"],
                "description": "Content resolution",
                "default": "l0"
            }
        },
        "required": ["chunk_id"]
    })
}

/// Generate the JSON Schema for `knowledge_ingest` parameters.
pub fn knowledge_ingest_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "source": {
                "type": "string",
                "description": "Source URI (file path or URL)"
            },
            "domain": {
                "type": "string",
                "description": "Optional domain hint"
            },
            "collection": {
                "type": "string",
                "description": "Target collection name",
                "default": "default"
            }
        },
        "required": ["source"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_params_defaults() {
        let params: KnowledgeSearchParams =
            serde_json::from_str(r#"{"query": "rust error handling"}"#).unwrap();
        assert_eq!(params.query, "rust error handling");
        assert_eq!(params.resolution, "l0");
        assert_eq!(params.limit, 5);
        assert!(params.domain.is_none());
    }

    #[test]
    fn test_search_params_full() {
        let params: KnowledgeSearchParams = serde_json::from_str(
            r#"{"query": "async", "domain": "rust", "resolution": "l1", "limit": 10}"#,
        )
        .unwrap();
        assert_eq!(params.domain, Some("rust".to_string()));
        assert_eq!(params.resolution, "l1");
        assert_eq!(params.limit, 10);
    }

    #[test]
    fn test_lookup_params_defaults() {
        let params: KnowledgeLookupParams =
            serde_json::from_str(r#"{"chunk_id": "abc123"}"#).unwrap();
        assert_eq!(params.chunk_id, "abc123");
        assert_eq!(params.resolution, "l0");
    }

    #[test]
    fn test_ingest_params_defaults() {
        let params: KnowledgeIngestParams =
            serde_json::from_str(r#"{"source": "/path/to/file.md"}"#).unwrap();
        assert_eq!(params.source, "/path/to/file.md");
        assert_eq!(params.collection, "default");
        assert!(params.domain.is_none());
    }

    #[test]
    fn test_search_result_serialization() {
        let result = KnowledgeSearchResult {
            results: vec![SearchResultItem {
                chunk_id: "c1".to_string(),
                document_id: "doc-1".to_string(),
                content: "Rust error handling".to_string(),
                relevance: 0.95,
                domains: vec!["rust".to_string()],
                title: "Error Guide".to_string(),
            }],
            total_matches: 1,
            strategy: "hybrid".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: KnowledgeSearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.results.len(), 1);
        assert_eq!(deserialized.results[0].chunk_id, "c1");
    }

    #[test]
    fn test_schemas_valid_json() {
        let search = knowledge_search_schema();
        assert_eq!(search["type"], "object");
        assert!(search["properties"]["query"].is_object());

        let lookup = knowledge_lookup_schema();
        assert_eq!(lookup["required"][0], "chunk_id");

        let ingest = knowledge_ingest_schema();
        assert_eq!(ingest["required"][0], "source");
    }

    #[test]
    fn test_ingest_result_serialization() {
        let result = KnowledgeIngestResult {
            success: true,
            entry_id: Some("entry-1".to_string()),
            chunk_count: 5,
            domains: vec!["rust".to_string()],
            quality_score: 0.8,
            message: "Ingested successfully".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("entry-1"));
    }
}
