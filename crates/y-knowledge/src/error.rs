//! Error types for the knowledge module.

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeError {
    #[error("document not found: {id}")]
    NotFound { id: String },

    #[error("ingestion failed: {message}")]
    IngestionError { message: String },

    #[error("indexing error: {message}")]
    IndexingError { message: String },

    #[error("retrieval error: {message}")]
    RetrievalError { message: String },

    #[error("chunk error: {message}")]
    ChunkError { message: String },

    #[error("{message}")]
    Other { message: String },
}
