//! Core data models for the knowledge base.
//!
//! Defines the complete knowledge entry, source provenance, collection
//! management, and entry lifecycle state machine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Entry State Machine
// ---------------------------------------------------------------------------

/// Lifecycle state of a knowledge entry.
///
/// ```text
/// Fetched → Parsed → Chunked → Classified → Filtered → Indexed → Active → Stale → Expired
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// Raw content fetched from source.
    Fetched,
    /// Content successfully parsed.
    Parsed,
    /// Content split into chunks.
    Chunked,
    /// Domain classification applied.
    Classified,
    /// Quality filtering passed.
    Filtered,
    /// Vectors indexed in store.
    Indexed,
    /// Available for retrieval.
    Active,
    /// Source may have changed; needs re-check.
    Stale,
    /// Past TTL; pending garbage collection.
    Expired,
}

impl std::fmt::Display for EntryState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetched => write!(f, "fetched"),
            Self::Parsed => write!(f, "parsed"),
            Self::Chunked => write!(f, "chunked"),
            Self::Classified => write!(f, "classified"),
            Self::Filtered => write!(f, "filtered"),
            Self::Indexed => write!(f, "indexed"),
            Self::Active => write!(f, "active"),
            Self::Stale => write!(f, "stale"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

// ---------------------------------------------------------------------------
// Source Provenance
// ---------------------------------------------------------------------------

/// Source type for knowledge entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    /// Local file (text, markdown, etc.)
    File,
    /// Web page.
    Web,
    /// API endpoint.
    Api,
    /// PDF document.
    Pdf,
    /// Manual / user-provided.
    Manual,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File => write!(f, "file"),
            Self::Web => write!(f, "web"),
            Self::Api => write!(f, "api"),
            Self::Pdf => write!(f, "pdf"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

/// Provenance information for a knowledge entry's source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    /// Type of source.
    pub source_type: SourceType,
    /// URI or path of the source.
    pub uri: String,
    /// SHA-256 hash of the raw content (for change detection).
    pub content_hash: String,
    /// Human-readable title.
    pub title: String,
    /// Author or creator (if known).
    pub author: Option<String>,
    /// When the content was fetched.
    pub fetched_at: DateTime<Utc>,
    /// ID of the connector that fetched this.
    pub connector_id: Option<String>,
}

impl SourceRef {
    /// Create a new `SourceRef` for a local file.
    pub fn file(
        uri: impl Into<String>,
        title: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Self {
        Self {
            source_type: SourceType::File,
            uri: uri.into(),
            content_hash: content_hash.into(),
            title: title.into(),
            author: None,
            fetched_at: Utc::now(),
            connector_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// L1 Section
// ---------------------------------------------------------------------------

/// A section-level chunk (L1 resolution) with title and content.
///
/// Generated during ingestion by splitting the document into sections
/// (heading-based, sentence-boundary, or double-newline depending on chunker).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1Section {
    /// Section index within the document.
    pub index: usize,
    /// Extracted section title (from heading or fallback "Section N").
    pub title: String,
    /// Section content text.
    pub content: String,
}

// ---------------------------------------------------------------------------
// Knowledge Entry
// ---------------------------------------------------------------------------

/// A complete knowledge entry with all metadata.
///
/// This is the primary data structure stored in the knowledge base.
/// Each entry represents a single document or content piece that has been
/// ingested, processed, and indexed for retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    /// Unique identifier.
    pub id: Uuid,
    /// Workspace this entry belongs to.
    pub workspace_id: String,
    /// Collection this entry belongs to.
    pub collection: String,

    // -- Content --
    /// Full original content.
    pub content: String,
    /// L0: compact summary (~100 tokens).
    pub summary: Option<String>,
    /// L1: section-level overview (~500 tokens).
    pub overview: Option<String>,
    /// L1: structured section-level chunks.
    #[serde(default)]
    pub l1_sections: Vec<L1Section>,

    // -- Classification --
    /// Domain classifications (e.g., `["rust", "async"]`).
    pub domains: Vec<String>,
    /// Free-form tags.
    pub tags: Vec<String>,

    // -- Source --
    /// Source provenance.
    pub source: SourceRef,

    // -- Quality --
    /// Quality score (0.0–1.0).
    pub quality_score: f32,

    // -- Lifecycle --
    /// Current state in the lifecycle.
    pub state: EntryState,
    /// When this entry was created.
    pub created_at: DateTime<Utc>,
    /// When this entry was last refreshed.
    pub refreshed_at: DateTime<Utc>,
    /// Time-to-live in seconds (None = never expires).
    pub ttl: Option<u64>,

    // -- MaxKB-inspired --
    /// Cached chunk texts (avoid re-chunking).
    pub chunks: Vec<String>,
    /// Persisted content size in bytes (sum of chunk lengths at ingest time).
    /// Used for accurate `total_bytes` accounting on deletion, even when
    /// chunk content may have been cleared.
    #[serde(default)]
    pub content_size: u64,
    /// Whether this entry is active (can be disabled without deletion).
    pub is_active: bool,
    /// Number of retrieval hits (for analytics and ranking).
    pub hit_num: u32,
}

impl KnowledgeEntry {
    /// Create a new entry in `Fetched` state.
    pub fn new(
        workspace_id: impl Into<String>,
        collection: impl Into<String>,
        content: impl Into<String>,
        source: SourceRef,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id: workspace_id.into(),
            collection: collection.into(),
            content: content.into(),
            summary: None,
            overview: None,
            l1_sections: Vec::new(),
            domains: Vec::new(),
            tags: Vec::new(),
            source,
            quality_score: 0.0,
            state: EntryState::Fetched,
            created_at: now,
            refreshed_at: now,
            ttl: None,
            chunks: Vec::new(),
            content_size: 0,
            is_active: true,
            hit_num: 0,
        }
    }

    /// Transition to a new state.
    pub fn transition(&mut self, new_state: EntryState) {
        self.state = new_state;
    }

    /// Record a retrieval hit.
    pub fn record_hit(&mut self) {
        self.hit_num += 1;
    }

    /// Check if the entry has expired based on TTL.
    pub fn is_expired(&self) -> bool {
        if let Some(ttl_secs) = self.ttl {
            let elapsed = Utc::now()
                .signed_duration_since(self.refreshed_at)
                .num_seconds();
            elapsed > 0 && elapsed.unsigned_abs() > ttl_secs
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Knowledge Collection
// ---------------------------------------------------------------------------

/// Refresh policy for knowledge collections.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RefreshPolicy {
    /// Never automatically refresh.
    #[default]
    Manual,
    /// Refresh on a fixed interval (seconds).
    Interval(u64),
    /// Refresh when source hash changes (on access).
    OnChange,
}

/// Configuration for a knowledge collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Target chunk size in characters.
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Overlap between consecutive chunks in characters.
    #[serde(default = "default_overlap")]
    pub overlap: usize,
    /// Embedding model to use for this collection.
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Refresh policy.
    #[serde(default)]
    pub refresh_policy: RefreshPolicy,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            chunk_size: default_chunk_size(),
            overlap: default_overlap(),
            embedding_model: default_embedding_model(),
            refresh_policy: RefreshPolicy::default(),
        }
    }
}

const fn default_chunk_size() -> usize {
    256
}
const fn default_overlap() -> usize {
    32
}
fn default_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}

/// Statistics for a knowledge collection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionStats {
    /// Total number of entries.
    pub entry_count: u64,
    /// Total number of indexed chunks.
    pub chunk_count: u64,
    /// Total content size in bytes.
    pub total_bytes: u64,
}

/// A knowledge collection groups related entries together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCollection {
    /// Unique identifier.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Description of this collection.
    pub description: String,
    /// Workspace this collection belongs to.
    pub workspace_id: String,
    /// Collection-level configuration.
    pub config: CollectionConfig,
    /// Aggregate statistics.
    pub stats: CollectionStats,
    /// When this collection was created.
    pub created_at: DateTime<Utc>,
    /// When this collection was last updated.
    pub updated_at: DateTime<Utc>,
}

impl KnowledgeCollection {
    /// Create a new collection with default config.
    pub fn new(
        workspace_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            workspace_id: workspace_id.into(),
            config: CollectionConfig::default(),
            stats: CollectionStats::default(),
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_source() -> SourceRef {
        SourceRef::file("/path/to/doc.md", "Test Document", "abc123hash")
    }

    #[test]
    fn test_entry_creation() {
        let entry = KnowledgeEntry::new("ws-1", "default", "Hello, world!", test_source());
        assert_eq!(entry.workspace_id, "ws-1");
        assert_eq!(entry.collection, "default");
        assert_eq!(entry.state, EntryState::Fetched);
        assert!(entry.is_active);
        assert_eq!(entry.hit_num, 0);
        assert!(entry.chunks.is_empty());
    }

    #[test]
    fn test_entry_state_transition() {
        let mut entry = KnowledgeEntry::new("ws-1", "default", "content", test_source());
        assert_eq!(entry.state, EntryState::Fetched);

        entry.transition(EntryState::Parsed);
        assert_eq!(entry.state, EntryState::Parsed);

        entry.transition(EntryState::Active);
        assert_eq!(entry.state, EntryState::Active);
    }

    #[test]
    fn test_entry_hit_counter() {
        let mut entry = KnowledgeEntry::new("ws-1", "default", "content", test_source());
        assert_eq!(entry.hit_num, 0);
        entry.record_hit();
        entry.record_hit();
        assert_eq!(entry.hit_num, 2);
    }

    #[test]
    fn test_entry_ttl_expiry() {
        let mut entry = KnowledgeEntry::new("ws-1", "default", "content", test_source());

        // No TTL set — never expires.
        assert!(!entry.is_expired());

        // Set a very large TTL — not expired.
        entry.ttl = Some(999_999);
        assert!(!entry.is_expired());

        // Set TTL to 0 — immediately expired.
        entry.ttl = Some(0);
        // refreshed_at is now, so 0 TTL means expired if any time passed.
        // This may or may not trigger depending on timing;
        // use a past refreshed_at to guarantee.
        entry.refreshed_at = Utc::now() - chrono::Duration::seconds(10);
        assert!(entry.is_expired());
    }

    #[test]
    fn test_collection_creation() {
        let coll = KnowledgeCollection::new("ws-1", "rust-docs", "Rust documentation");
        assert_eq!(coll.name, "rust-docs");
        assert_eq!(coll.workspace_id, "ws-1");
        assert_eq!(coll.config.chunk_size, 256);
        assert_eq!(coll.stats.entry_count, 0);
    }

    #[test]
    fn test_entry_state_display() {
        assert_eq!(EntryState::Fetched.to_string(), "fetched");
        assert_eq!(EntryState::Active.to_string(), "active");
        assert_eq!(EntryState::Expired.to_string(), "expired");
    }

    #[test]
    fn test_source_ref_file_constructor() {
        let source = SourceRef::file("/tmp/test.md", "Test", "hash123");
        assert_eq!(source.source_type, SourceType::File);
        assert_eq!(source.uri, "/tmp/test.md");
        assert_eq!(source.content_hash, "hash123");
        assert!(source.author.is_none());
    }

    #[test]
    fn test_collection_config_defaults() {
        let config = CollectionConfig::default();
        assert_eq!(config.chunk_size, 256);
        assert_eq!(config.overlap, 32);
        assert_eq!(config.embedding_model, "text-embedding-3-small");
        assert!(matches!(config.refresh_policy, RefreshPolicy::Manual));
    }

    #[test]
    fn test_entry_serialization() {
        let entry = KnowledgeEntry::new("ws-1", "default", "test content", test_source());
        let json = serde_json::to_string(&entry).expect("serialize");
        let deserialized: KnowledgeEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.workspace_id, "ws-1");
        assert_eq!(deserialized.collection, "default");
        assert_eq!(deserialized.state, EntryState::Fetched);
    }
}
