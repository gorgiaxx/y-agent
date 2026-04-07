# TODO

## Knowledge System

### Qdrant Vector Store Integration

Status: Code complete, not wired into runtime.

The `VectorIndexer` in `y-knowledge/src/indexer.rs` has a full Qdrant gRPC client
implementation behind the `vector_qdrant` feature flag. Currently all vector
operations (storage + cosine similarity search) happen in-memory inside
`HybridRetriever`. The Qdrant path needs to be wired as an alternative backend
for large-scale knowledge bases.

Tasks:

- [ ] Wire `VectorIndexer` into `KnowledgeService` as a configurable search backend
- [ ] Add Qdrant search path in `HybridRetriever` or as a parallel retriever
- [ ] Route ingestion to Qdrant upsert when `vector_qdrant` feature + config enabled
- [ ] Route search to Qdrant query when `vector_qdrant` feature + config enabled
- [ ] Update `KnowledgeConfig` with `vector_backend = "memory" | "qdrant"` option
- [ ] Integration test with Qdrant container
- [ ] Update README to clarify that Qdrant is optional (in-memory works by default)

Priority: Low -- in-memory approach works well for small-to-medium knowledge bases.
Qdrant becomes necessary when knowledge exceeds ~10K documents or when persistence
across restarts without the binary embedding file is desired.
