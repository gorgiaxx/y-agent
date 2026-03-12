//! Vector indexer: Qdrant collection management (feature-gated).
//!
//! This module provides the interface for vector indexing. In development,
//! it operates as a no-op. Production use requires the `vector_qdrant` feature.

/// Placeholder for vector indexing operations.
///
/// Production implementation will manage Qdrant collections with
/// domain classification and freshness tracking.
#[derive(Debug, Default)]
pub struct VectorIndexer;

impl VectorIndexer {
    pub fn new() -> Self {
        Self
    }
}
