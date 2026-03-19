# Qdrant + Embedding Semantic Retrieval Integration

## Status: IMPLEMENTED

All 6 phases have been implemented and pass compilation + tests.

## Summary

| Component | Before | After |
|-----------|--------|-------|
| **Retrieval** | `HybridRetriever` text similarity (substring + word overlap) | Real cosine similarity + BM25 hybrid search |
| **Embedding** | `EmbeddingProvider` trait only, no implementation | `OpenAiEmbeddingProvider` in `y-provider` (OpenAI-compatible) |
| **Vector Store** | `VectorIndexer` Qdrant (feature-gated, no-op default) | In-memory `HashMap<String, Vec<f32>>` in `HybridRetriever`; Qdrant optional |
| **Ingestion** | BM25 index only | BM25 + embedding generation + vector index |
| **Persistence** | Chunks in JSON only | Embeddings in `knowledge_embeddings.bin` (binary format) |
| **Config** | No embedding config | Full embedding + retrieval tuning in `KnowledgeConfig` |
| **Context** | Keyword-only retrieval | Query embedding + cosine similarity retrieval |

## Design Decisions

### In-Memory Vector Store (default) + Optional Qdrant

Qdrant is not mandatory. Reasons:
1. Most knowledge bases will not exceed 10K chunks; in-memory is fast enough
2. No external dependency (Qdrant server) required
3. Qdrant retained as `vector_qdrant` feature for large-scale deployment

### Embedding Persistence

Binary format (`knowledge_embeddings.bin`) instead of JSON:
- Format: `[count: u32][key_len: u32][key][vec_len: u32][f32 * vec_len]...`
- No external dependency (no bincode / protobuf)
- Loaded on startup and fed into `HybridRetriever`

### Graceful Degradation

All embedding operations are optional:
- No embedding provider -> falls back to text similarity (existing behavior)
- Embedding API failure during ingestion -> logs warning, indexes without vectors
- Embedding API failure during query -> falls back to keyword search

## Files Changed

| File | Change |
|------|--------|
| `crates/y-provider/src/embedding.rs` | **NEW** -- `OpenAiEmbeddingProvider` + `EmbeddingConfig` |
| `crates/y-provider/src/lib.rs` | Added `pub mod embedding` + re-exports |
| `crates/y-knowledge/src/retrieval.rs` | Added `embeddings` map, `cosine_similarity()`, `search_with_embedding()`, `index_with_embedding()` |
| `crates/y-knowledge/src/config.rs` | Added embedding + retrieval tuning fields |
| `crates/y-knowledge/src/middleware.rs` | `retrieve_for_context()` now accepts `query_embedding: Option<&[f32]>` |
| `crates/y-service/src/knowledge_service.rs` | Added embedding provider, embedding pipeline in ingestion, binary persistence |
| `crates/y-context/src/knowledge_provider.rs` | Added embedding provider, query embedding before retrieval |
| `crates/y-tools/src/builtin/knowledge_search.rs` | Updated `retrieve_for_context` call signature |

## Implementation Phases

### Phase 1: OpenAI Embedding Provider (`y-provider`)

`crates/y-provider/src/embedding.rs`:
- `OpenAiEmbeddingProvider` implements `y_core::embedding::EmbeddingProvider`
- `POST /embeddings` on any OpenAI-compatible API
- Configurable `base_url`, `model`, `dimensions`, `api_key`
- Native batch embedding (single API call for multiple inputs, up to 2048)
- `EmbeddingConfig` with serde support for TOML config files

### Phase 2: In-Memory Vector Store (`y-knowledge`)

`crates/y-knowledge/src/retrieval.rs`:
- `embeddings: HashMap<String, Vec<f32>>` added to `HybridRetriever`
- `index_with_embedding(chunk, embedding, quality_score)` stores embedding
- `search_with_embedding(query, query_embedding, filter)` uses cosine when available
- `cosine_similarity(a, b)` free function for vector comparison
- Falls back to text similarity when embeddings are absent

### Phase 3: Embedding Pipeline (`y-service`)

`crates/y-service/src/knowledge_service.rs`:
- `embedding_provider: Option<Arc<dyn EmbeddingProvider>>` field
- `set_embedding_provider()` setter
- Ingestion: `embed_batch()` on chunk texts, then `index_with_embedding()`
- Graceful fallback: log warning and use keyword-only if embedding fails

### Phase 4: Embedding Persistence (`y-service`)

`crates/y-service/src/knowledge_service.rs`:
- `save_embeddings()` writes `knowledge_embeddings.bin`
- `load_embeddings()` reads on startup, feeds into retriever
- Called during `reindex_all_entries()` to restore embeddings after restart

### Phase 5: Configuration (`y-knowledge`)

`crates/y-knowledge/src/config.rs` -- new `KnowledgeConfig` fields:
- `embedding_enabled: bool` (default: false)
- `embedding_model: String` (default: "text-embedding-3-small")
- `embedding_dimensions: usize` (default: 1536)
- `embedding_base_url: String` (default: "https://api.openai.com/v1")
- `embedding_api_key_env: String` (default: "OPENAI_API_KEY")
- `retrieval_strategy: String` (default: "hybrid")
- `bm25_weight: f64` (default: 1.0)
- `vector_weight: f64` (default: 1.0)

### Phase 6: Context Provider Upgrade (`y-context`)

`crates/y-context/src/knowledge_provider.rs`:
- `embedding_provider: Option<Arc<dyn EmbeddingProvider>>` field
- `with_embedding()` constructor
- `provide()` embeds query before retrieval when provider is available
- Graceful fallback: log warning and skip embedding on failure

## Verification

```
cargo build              # Full workspace compiles
cargo test -p y-provider # Embedding provider tests (5 tests)
cargo test -p y-knowledge # Retrieval + cosine similarity tests (17+ tests)
cargo test -p y-service  # Knowledge service tests
cargo test -p y-context  # Context provider tests
```
