//! y-knowledge: Knowledge Base — external ingestion, domain-classified indexing.
//!
//! Manages external knowledge ingestion, domain-classified vector indexing,
//! hybrid retrieval, and L0/L1/L2 multi-resolution progressive loading.
//!
//! Knowledge is distinct from LTM: knowledge is external/domain-classified;
//! LTM is conversation-extracted.
//!
//! # Components
//!
//! - [`bm25`] — BM25 inverted index for keyword search
//! - [`chunking::ChunkingStrategy`] — L0/L1/L2 multi-resolution chunking
//! - [`classifier`] — Domain classification (rule-based + LLM placeholder)
//! - [`ingestion`] — Source connectors and ingestion pipeline
//! - [`maintenance`] — Staleness detection, TTL expiry, hit tracking
//! - [`middleware`] — `InjectKnowledge` context injection (priority 350)
//! - [`models`] — Core data models (`KnowledgeEntry`, `KnowledgeCollection`, etc.)
//! - [`normalize`] — Text normalization for embedding preprocessing
//! - [`observability`] — Events, metrics, and hook points
//! - [`progressive::ProgressiveLoader`] — on-demand resolution escalation
//! - [`quality`] — Quality filtering and deduplication
//! - [`retrieval::HybridRetriever`] — blend search (vector + BM25) with dedup
//! - [`tools`] — Built-in knowledge tools for Agent use
//! - [`indexer::VectorIndexer`] — Qdrant collection management (feature-gated)
//! - [`tokenizer`] — English/Chinese text segmentation

pub mod bm25;
pub mod chunking;
pub mod classifier;
pub mod config;
pub mod error;
pub mod indexer;
pub mod ingestion;
pub mod maintenance;
pub mod middleware;
pub mod models;
pub mod normalize;
pub mod observability;
pub mod progressive;
pub mod quality;
pub mod retrieval;
pub mod tokenizer;
pub mod tools;

// Re-export primary types.
pub use bm25::Bm25Index;
pub use chunking::{Chunk, ChunkLevel, ChunkerType, ChunkingStrategy};
pub use classifier::{Classifier, RuleBasedClassifier};
pub use config::KnowledgeConfig;
pub use error::KnowledgeError;
pub use ingestion::{IngestionPipeline, RawDocument, SourceConnector};
pub use maintenance::{HitTracker, MaintenanceManager};
pub use middleware::{EntryMetadata, InjectKnowledge};
pub use models::{EntryState, KnowledgeCollection, KnowledgeEntry, L1Section, SourceRef};
pub use normalize::normalize_for_embedding;
pub use observability::{KnowledgeEvent, KnowledgeMetrics, MetricsCollector};
pub use progressive::ProgressiveLoader;
pub use quality::QualityFilter;
pub use retrieval::{HybridRetriever, SearchStrategy, SummaryGenerator};
pub use tokenizer::{AutoTokenizer, ChineseTokenizer, SimpleTokenizer, Tokenizer};

