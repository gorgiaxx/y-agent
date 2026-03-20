//! Embedding provider trait for vector-based memory recall.
//!
//! Design reference: memory-architecture-design.md §Vector Search
//!
//! This module defines the `EmbeddingProvider` trait for generating text
//! embeddings. Implementations may wrap `OpenAI`, local models, or other
//! embedding APIs.

use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single embedding vector.
pub type Embedding = Vec<f32>;

/// Result of embedding a text.
#[derive(Debug, Clone)]
pub struct EmbeddingResult {
    /// The embedding vector.
    pub vector: Embedding,
    /// Dimensionality of the embedding.
    pub dimensions: usize,
    /// Model used to generate the embedding.
    pub model: String,
    /// Tokens consumed by the input text.
    pub token_count: u32,
}

/// Errors from embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("embedding provider error: {message}")]
    ProviderError { message: String },

    #[error("text too long for embedding: {tokens} tokens (max {max_tokens})")]
    TextTooLong { tokens: u32, max_tokens: u32 },

    #[error("embedding model not available: {model}")]
    ModelNotAvailable { model: String },
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Trait for generating text embeddings.
///
/// Implementations handle API calls to embedding providers (`OpenAI`, Ollama,
/// local models, etc.). The trait is intentionally minimal — one method.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding for the given text.
    async fn embed(&self, text: &str) -> Result<EmbeddingResult, EmbeddingError>;

    /// Generate embeddings for multiple texts in a batch.
    ///
    /// Default implementation calls `embed` sequentially. Providers may
    /// override for batch API support.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>, EmbeddingError> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    /// Return the dimensionality of embeddings from this provider.
    fn dimensions(&self) -> usize;

    /// Return the model name used by this provider.
    fn model_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockEmbedding;

    #[async_trait]
    impl EmbeddingProvider for MockEmbedding {
        async fn embed(&self, text: &str) -> Result<EmbeddingResult, EmbeddingError> {
            Ok(EmbeddingResult {
                vector: vec![0.1, 0.2, 0.3],
                dimensions: 3,
                model: "mock".into(),
                token_count: (text.len() / 4) as u32,
            })
        }
        fn dimensions(&self) -> usize {
            3
        }
        fn model_name(&self) -> &'static str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_mock_embed() {
        let provider = MockEmbedding;
        let result = provider.embed("hello world").await.unwrap();
        assert_eq!(result.dimensions, 3);
        assert_eq!(result.vector.len(), 3);
    }

    #[tokio::test]
    async fn test_embed_batch() {
        let provider = MockEmbedding;
        let texts = vec!["hello".into(), "world".into()];
        let results = provider.embed_batch(&texts).await.unwrap();
        assert_eq!(results.len(), 2);
    }
}
