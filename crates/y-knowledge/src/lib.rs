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
//! - [`chunking::ChunkingStrategy`] — L0/L1/L2 multi-resolution chunking
//! - [`progressive::ProgressiveLoader`] — on-demand resolution escalation
//! - [`retrieval::HybridRetriever`] — vector + keyword search with fallback
//! - [`indexer::VectorIndexer`] — Qdrant collection management (feature-gated)

pub mod chunking;
pub mod config;
pub mod error;
pub mod indexer;
pub mod progressive;
pub mod retrieval;

// Re-export primary types.
pub use chunking::{Chunk, ChunkLevel, ChunkingStrategy};
pub use config::KnowledgeConfig;
pub use error::KnowledgeError;
pub use progressive::ProgressiveLoader;
pub use retrieval::HybridRetriever;
