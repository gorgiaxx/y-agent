# R&D Plan: y-knowledge

**Module**: `crates/y-knowledge`
**Phase**: 4.2 (Intelligence Layer)
**Priority**: Medium — external knowledge augments agent reasoning
**Design References**: `knowledge-base-design.md`
**Depends On**: `y-core`, `y-hooks`

---

## 1. Module Purpose

`y-knowledge` manages external knowledge ingestion (PDF, web, API), domain-classified vector indexing (Qdrant), hybrid retrieval, and L0/L1/L2 multi-resolution progressive loading. Knowledge is distinct from LTM: knowledge is external/domain-classified; LTM is conversation-extracted.

---

## 2. Dependency Map

```
y-knowledge
  ├── y-core (MemoryClient interface for retrieval pattern)
  ├── y-hooks (InjectKnowledge middleware at priority 350)
  ├── qdrant-client (vector store — feature: vector_qdrant)
  ├── reqwest (web fetching)
  ├── tokio (async I/O)
  ├── serde / serde_json (document parsing)
  ├── thiserror (errors)
  └── tracing (domain, source_type, chunk_count spans)
```

---

## 3. Module Structure

```
y-knowledge/src/
  lib.rs              — Public API: KnowledgeManager
  error.rs            — KnowledgeError
  config.rs           — KnowledgeConfig (collections, chunk sizes, embedding model)
  ingestion/
    mod.rs            — IngestionPipeline
    pdf.rs            — PDF document parser
    web.rs            — Web page fetcher and parser
    api.rs            — API response ingester
  chunking.rs         — ChunkingStrategy: L0/L1/L2 multi-resolution
  indexer.rs          — VectorIndexer: Qdrant collection management
  retrieval.rs        — HybridRetriever: vector + keyword search
  progressive.rs      — ProgressiveLoader: L0 summary → L1 sections → L2 full
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-KB-001 — Chunking strategy

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-KB-001-01 | `test_chunking_l0_produces_summary` | Document → L0 | Single summary chunk |
| T-KB-001-02 | `test_chunking_l1_produces_sections` | Document → L1 | Section-level chunks |
| T-KB-001-03 | `test_chunking_l2_produces_full` | Document → L2 | Paragraph-level chunks |
| T-KB-001-04 | `test_chunking_respects_max_tokens` | Chunk with limit | Each chunk within limit |
| T-KB-001-05 | `test_chunking_preserves_metadata` | Source URL, domain | Metadata on each chunk |

#### Task: T-KB-002 — Progressive loader

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-KB-002-01 | `test_progressive_l0_first` | Initial query | Returns L0 summaries |
| T-KB-002-02 | `test_progressive_l1_on_demand` | User requests detail | Returns L1 sections |
| T-KB-002-03 | `test_progressive_l2_full` | Deep dive | Returns L2 full content |
| T-KB-002-04 | `test_progressive_token_budget` | Budget=500 tokens | Stays within budget across levels |

#### Task: T-KB-003 — Hybrid retrieval

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-KB-003-01 | `test_retrieval_vector_search` | Semantic query | Returns similar documents |
| T-KB-003-02 | `test_retrieval_keyword_fallback` | No vector matches | Falls back to keyword |
| T-KB-003-03 | `test_retrieval_domain_filter` | Query with domain="rust" | Only rust domain results |
| T-KB-003-04 | `test_retrieval_freshness_filter` | Filter by freshness | Excludes stale documents |
| T-KB-003-05 | `test_retrieval_respects_limit` | Limit=5 | At most 5 results |

### 4.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-KB-INT-01 | `ingestion_integration_test.rs` | `test_ingest_and_retrieve` | Ingest document → index → query → retrieve |
| T-KB-INT-02 | `ingestion_integration_test.rs` | `test_multi_resolution_query` | Ingest → L0 query → L1 drill-down |
| T-KB-INT-03 | `ingestion_integration_test.rs` | `test_domain_classification` | Ingest docs in 3 domains → filter by domain |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-KB-001 | `ChunkingStrategy` | L0/L1/L2 multi-resolution chunking | High |
| I-KB-002 | `VectorIndexer` | Qdrant collection management (feature-gated) | High |
| I-KB-003 | `HybridRetriever` | Vector + keyword search with fallback | High |
| I-KB-004 | `ProgressiveLoader` | L0 → L1 → L2 on-demand loading | High |
| I-KB-005 | PDF ingestion | PDF document parsing | Medium |
| I-KB-006 | Web ingestion | Web page fetching and parsing | Medium |
| I-KB-007 | API ingestion | API response processing | Low |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 75% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-knowledge` |
| Chunk quality | L0 summaries < 200 tokens | Validation test |

---

## 7. Acceptance Criteria

- [ ] Documents chunked at 3 resolution levels (L0/L1/L2)
- [ ] Qdrant indexing with domain classification and freshness tracking
- [ ] Hybrid retrieval with vector → keyword fallback
- [ ] Progressive loading respects token budgets
- [ ] At least one ingestion source (web) functional
- [ ] Coverage >= 75%
