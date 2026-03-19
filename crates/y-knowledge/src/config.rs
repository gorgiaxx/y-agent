//! Configuration for the knowledge module.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeConfig {
    /// Maximum tokens per L0 chunk (summary).
    #[serde(default = "default_l0_max_tokens")]
    pub l0_max_tokens: u32,

    /// Maximum tokens per L1 chunk (section).
    #[serde(default = "default_l1_max_tokens")]
    pub l1_max_tokens: u32,

    /// Maximum tokens per L2 chunk (paragraph).
    /// When embedding is enabled, chunks are further capped by `embedding_max_tokens`.
    #[serde(default = "default_l2_max_tokens")]
    pub l2_max_tokens: u32,

    /// Default collection name for entries without explicit collection.
    #[serde(default = "default_collection")]
    pub default_collection: String,

    /// Minimum similarity threshold for retrieval results (0.0-1.0).
    /// Results below this score are discarded. Inspired by `MaxKB` default.
    #[serde(default = "default_min_similarity_threshold")]
    pub min_similarity_threshold: f32,

    /// Maximum number of chunks per entry. If chunking produces more than
    /// this limit, adjacent chunks are merged to stay within budget.
    /// Prevents excessive chunk counts from large documents (e.g. novels).
    #[serde(default = "default_max_chunks_per_entry")]
    pub max_chunks_per_entry: usize,

    // -- Embedding configuration --
    /// Whether embedding-based semantic search is enabled.
    #[serde(default)]
    pub embedding_enabled: bool,

    /// Embedding model name (e.g. "text-embedding-3-small").
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,

    /// Embedding vector dimensions.
    #[serde(default = "default_embedding_dimensions")]
    pub embedding_dimensions: usize,

    /// Base URL for the embedding API.
    #[serde(default = "default_embedding_base_url")]
    pub embedding_base_url: String,

    /// Environment variable holding the embedding API key.
    #[serde(default = "default_embedding_api_key_env")]
    pub embedding_api_key_env: String,

    /// Direct API key value (takes precedence over `embedding_api_key_env`).
    /// Useful for local servers (LM Studio, Ollama) that accept any key.
    #[serde(default)]
    pub embedding_api_key: String,

    /// Maximum tokens per chunk that the embedding model can accept.
    /// When set (> 0), chunk sizes are capped to this value before embedding.
    /// Set this to your embedding model's context window (e.g. 512 for many
    /// local GGUF models, 8192 for `OpenAI` text-embedding-3-small).
    /// Default: 0 (uses `l2_max_tokens` as the limit).
    #[serde(default)]
    pub embedding_max_tokens: u32,

    // -- Retrieval tuning --
    /// Retrieval strategy: "hybrid", "keyword", or "semantic".
    #[serde(default = "default_retrieval_strategy")]
    pub retrieval_strategy: String,

    /// BM25 weight in blend fusion.
    #[serde(default = "default_weight")]
    pub bm25_weight: f64,

    /// Vector similarity weight in blend fusion.
    #[serde(default = "default_weight")]
    pub vector_weight: f64,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            l0_max_tokens: 200,
            l1_max_tokens: 500,
            l2_max_tokens: default_l2_max_tokens(),
            default_collection: default_collection(),
            min_similarity_threshold: default_min_similarity_threshold(),
            max_chunks_per_entry: default_max_chunks_per_entry(),
            embedding_enabled: false,
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dimensions(),
            embedding_base_url: default_embedding_base_url(),
            embedding_api_key_env: default_embedding_api_key_env(),
            embedding_api_key: String::new(),
            embedding_max_tokens: 0,
            retrieval_strategy: default_retrieval_strategy(),
            bm25_weight: default_weight(),
            vector_weight: default_weight(),
        }
    }
}

impl KnowledgeConfig {
    /// Effective maximum tokens per chunk for embedding.
    ///
    /// Returns `embedding_max_tokens` if explicitly set (> 0),
    /// otherwise falls back to `l2_max_tokens`.
    pub fn effective_chunk_max_tokens(&self) -> u32 {
        if self.embedding_max_tokens > 0 {
            self.embedding_max_tokens
        } else {
            self.l2_max_tokens
        }
    }
}

const fn default_l0_max_tokens() -> u32 {
    200
}
const fn default_l1_max_tokens() -> u32 {
    500
}
const fn default_l2_max_tokens() -> u32 {
    450
}
fn default_collection() -> String {
    "default".to_string()
}
const fn default_min_similarity_threshold() -> f32 {
    0.65
}
const fn default_max_chunks_per_entry() -> usize {
    5000
}
fn default_embedding_model() -> String {
    "text-embedding-3-small".to_string()
}
const fn default_embedding_dimensions() -> usize {
    1536
}
fn default_embedding_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_embedding_api_key_env() -> String {
    "OPENAI_API_KEY".to_string()
}
fn default_retrieval_strategy() -> String {
    "hybrid".to_string()
}
const fn default_weight() -> f64 {
    1.0
}
