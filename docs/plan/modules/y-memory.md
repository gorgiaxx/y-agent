# R&D Plan: y-memory (Memory System)

**Module**: Spans `y-context` (Working Memory), `y-storage` (STM), Qdrant client (LTM)
**Phase**: 4.1 (Intelligence Layer)
**Priority**: High — memory makes the agent learn and recall across sessions
**Design References**: `memory-architecture-design.md`, `memory-short-term-design.md`, `memory-long-term-design.md`
**Depends On**: `y-core`, `y-hooks`, `y-storage`

---

## 1. Module Purpose

The memory system spans three tiers with distinct lifecycles:

- **Long-Term Memory (LTM)**: Persistent, vector store backed (Qdrant), workspace-scoped. Two-phase deduplication, intent-aware recall, Search Orchestrator with multi-strategy fallback.
- **Short-Term Memory (STM)**: Session-scoped, SQLite-backed. Experience Store for indexed archival.
- **Working Memory (WM)**: Pipeline-scoped, in-memory blackboard with 4 cognitive categories.

Memory middleware in the Context chain injects recalled memories into LLM prompts.

---

## 2. Module Structure

Memory is not a single crate; it spans multiple crates with components:

```
y-core/src/memory.rs           — MemoryClient, ExperienceStore traits (DONE)
y-storage/src/experience.rs    — SqliteExperienceStore (STM)
y-context/src/memory/
  ltm_client.rs                — LtmClient: Qdrant-backed MemoryClient (feature: memory_ltm)
  stm_client.rs                — StmClient: SQLite-backed session memory
  working_memory.rs            — WorkingMemory: in-memory blackboard
  deduplication.rs             — Two-phase dedup (content-hash + LLM 4-action)
  query.rs                     — TypedQuery: intent-aware query decomposition
  search_orchestrator.rs       — SearchOrchestrator: Vector → Hybrid → Keyword fallback
  recall_middleware.rs         — InjectMemory ContextMiddleware (priority 300)
```

---

## 3. Development Tasks

### 3.1 Unit Tests (TDD — Red Phase)

#### Task: T-MEM-001 — Working Memory (blackboard)

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-001-01 | `test_wm_put_get_slot` | Put value in slot | Get returns same value |
| T-MEM-001-02 | `test_wm_cognitive_categories` | Put in each of 4 categories | Each accessible by category |
| T-MEM-001-03 | `test_wm_token_estimate_tracked` | Put value with estimate | Total budget updated |
| T-MEM-001-04 | `test_wm_clear_resets` | Clear all slots | All empty |
| T-MEM-001-05 | `test_wm_pipeline_scoped_lifetime` | Create → use → drop | No persistence |

#### Task: T-MEM-002 — STM Experience Store

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-002-01 | `test_experience_compress_assigns_slot` | `compress()` | Returns monotonic slot_index |
| T-MEM-002-02 | `test_experience_read_by_slot` | `read(session, slot)` | Returns correct record |
| T-MEM-002-03 | `test_experience_list_session` | `list(session)` | Returns all session experiences |
| T-MEM-002-04 | `test_experience_evidence_type_stored` | Compress with `UserCorrection` | Evidence type preserved |
| T-MEM-002-05 | `test_experience_token_estimate_stored` | Compress with estimate | Token estimate retrievable |
| T-MEM-002-06 | `test_experience_cross_session_isolation` | 2 sessions | Each sees only own experiences |

#### Task: T-MEM-003 — LTM Client (feature-gated)

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-003-01 | `test_ltm_remember_and_recall` | Store memory, recall by query | Memory found with relevance |
| T-MEM-003-02 | `test_ltm_recall_respects_limit` | 10 memories, limit=3 | Returns top 3 |
| T-MEM-003-03 | `test_ltm_recall_filter_by_type` | Filter `MemoryType::Tool` | Only tool memories returned |
| T-MEM-003-04 | `test_ltm_recall_min_importance` | Filter importance > 0.5 | Low-importance excluded |
| T-MEM-003-05 | `test_ltm_forget` | `forget(id)` | Memory no longer retrievable |
| T-MEM-003-06 | `test_ltm_get_by_id` | `get(id)` | Returns exact memory |

#### Task: T-MEM-004 — Two-phase deduplication

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-004-01 | `test_dedup_content_hash_fast_path` | Identical content | Deduped without LLM call |
| T-MEM-004-02 | `test_dedup_similar_content_llm_check` | Similar but not identical | LLM 4-action model invoked |
| T-MEM-004-03 | `test_dedup_llm_merge_action` | LLM decides merge | Two memories merged into one |
| T-MEM-004-04 | `test_dedup_llm_keep_both_action` | LLM decides keep both | Both stored |
| T-MEM-004-05 | `test_dedup_dissimilar_content_no_check` | Very different content | No LLM call, both stored |

#### Task: T-MEM-005 — Search Orchestrator

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-005-01 | `test_search_vector_primary` | Clear semantic query | Vector search used first |
| T-MEM-005-02 | `test_search_hybrid_fallback` | Vector returns < min results | Falls back to hybrid |
| T-MEM-005-03 | `test_search_keyword_last_resort` | Hybrid also insufficient | Falls back to keyword |
| T-MEM-005-04 | `test_search_results_deduplicated` | Same doc from vector + keyword | Single result |

#### Task: T-MEM-006 — TypedQuery (intent-aware)

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MEM-006-01 | `test_typed_query_decomposition` | Complex query | Decomposed into sub-queries with types |
| T-MEM-006-02 | `test_typed_query_single_type` | Simple query | Single query, correct type |
| T-MEM-006-03 | `test_typed_query_personal_filter` | "My preferred coding style" | `MemoryType::Personal` filter applied |

### 3.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-MEM-INT-01 | `memory_integration_test.rs` | `test_stm_experience_full_lifecycle` | Compress → read → list → cross-session isolation |
| T-MEM-INT-02 | `memory_integration_test.rs` | `test_ltm_remember_recall_forget` | Store → recall → update → forget |
| T-MEM-INT-03 | `memory_integration_test.rs` | `test_memory_injection_in_context` | InjectMemory middleware recalls and injects into prompt |
| T-MEM-INT-04 | `memory_integration_test.rs` | `test_working_memory_in_pipeline` | WM used during pipeline, cleared after |
| T-MEM-INT-05 | `memory_integration_test.rs` | `test_dedup_end_to_end` | Store 3 similar memories → 2 after dedup |

---

## 4. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-MEM-001 | `WorkingMemory` | In-memory blackboard with 4 categories, token tracking | High |
| I-MEM-002 | `SqliteExperienceStore` | `ExperienceStore` trait impl in y-storage | High |
| I-MEM-003 | `LtmClient` | Qdrant-backed `MemoryClient` (feature: memory_ltm) | High |
| I-MEM-004 | `StmClient` | SQLite-backed session memory | High |
| I-MEM-005 | Two-phase deduplication | Content hash + LLM 4-action model | Medium |
| I-MEM-006 | `SearchOrchestrator` | Multi-strategy fallback (Vector → Hybrid → Keyword) | Medium |
| I-MEM-007 | `TypedQuery` | Intent-aware query decomposition | Medium |
| I-MEM-008 | `InjectMemory` middleware | ContextMiddleware at priority 300 | High |

---

## 5. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test` (relevant crates) |
| Memory recall relevance | > 0.7 avg for top-3 results | Evaluation test |

---

## 6. Acceptance Criteria

- [ ] Working Memory operates as in-memory blackboard with 4 cognitive categories
- [ ] STM Experience Store compresses and indexes experiences per session
- [ ] LTM recalls semantically relevant memories from Qdrant
- [ ] Two-phase dedup prevents duplicate memories (hash fast path + LLM)
- [ ] Search Orchestrator falls back through 3 strategies
- [ ] InjectMemory middleware respects token budget
- [ ] Cross-session memory recall works (store in session A, recall in session B)
- [ ] Coverage >= 80%
