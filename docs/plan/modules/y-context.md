# R&D Plan: y-context

**Module**: `crates/y-context`
**Phase**: 3.4 (Execution Layer)
**Priority**: High — assembles the LLM prompt from all context sources
**Design References**: `context-session-design.md`, `prompt-design.md`, `input-enrichment-design.md`
**Depends On**: `y-core`, `y-hooks` (ContextMiddleware chain), `y-session`
**Last Audited**: 2026-03-10

---

## 1. Module Purpose

`y-context` assembles the LLM prompt by orchestrating the Context middleware chain. It manages token budget enforcement, context compaction strategies, prompt section assembly, and user input enrichment. The pipeline is a sequence of `ContextMiddleware` instances (defined in `y-hooks`) that build the prompt incrementally.

---

## 2. Dependency Map

```
y-context
  ├── y-core (traits: MemoryClient, SessionStore, ToolRegistry for injection)
  ├── y-hooks (ContextMiddleware chain execution)
  ├── y-session (session messages for history loading)
  ├── tokio (async)
  ├── serde_json (payload assembly)
  ├── thiserror (errors)
  └── tracing (pipeline step spans)
```

---

## 3. Module Structure

```
y-context/src/
  lib.rs              — Public API: ContextPipeline, ContextWindowGuard, CompactionEngine, RecallStore
  pipeline.rs         — ContextPipeline: drives ordered ContextProvider list        ✅ implemented
  guard.rs            — ContextWindowGuard: token budget with 3 trigger modes       ✅ implemented (≈ planned budget.rs)
  compaction.rs       — CompactionEngine: message summarization, sliding window     ✅ implemented
  recall.rs           — RecallStore: memory recall via hybrid text/vector search    ✅ implemented (new)
  repair.rs           — repair_history: session history repair                      ✅ implemented (new)
  memory/
    mod.rs            — Memory integration submodule                                ✅ implemented (new)
    deduplication.rs  — Content-hash + semantic dedup                               ✅ implemented
    ltm_client.rs     — Long-term memory client                                     ✅ implemented
    stm_client.rs     — Short-term memory client                                    ✅ implemented
    working_memory.rs — Pipeline-scoped working memory                              ✅ implemented
    query.rs          — Memory query construction                                   ✅ implemented
    recall_middleware.rs — Memory recall as context middleware                       ✅ implemented
    search_orchestrator.rs — Multi-strategy search fallback                         ✅ implemented
```

> **Audit note (2026-03-10):** The original plan specified 8 separate middleware files under `middleware/`. The implementation instead uses a `ContextProvider` trait with ordered providers, which achieves the same 7-stage pipeline semantics but with a simpler architecture. `budget.rs` was merged into `guard.rs` as `ContextWindowGuard` with `TokenBudget`. `section.rs` was merged into `pipeline.rs` as `ContextItem`. Memory modules (LTM/STM/WM, dedup, search orchestrator) were co-located under `memory/` rather than in a separate `y-memory` crate.
>
> **Planned but not yet independently implemented middleware files:**
> `build_system.rs`, `inject_memory.rs`, `inject_knowledge.rs`, `inject_skills.rs`, `inject_tools.rs`, `load_history.rs`, `inject_status.rs`, `enrich_input.rs`

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-CTX-001 — TokenBudget

```
FILE: crates/y-context/src/budget.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-001-01 | `test_budget_allocate_within_limit` | Allocate 500 of 1000 budget | Ok, remaining = 500 |
| T-CTX-001-02 | `test_budget_reject_oversized` | Allocate 1500 of 1000 budget | Error: exceeded |
| T-CTX-001-03 | `test_budget_category_allocation` | System: 200, History: 500, Tools: 300 | Each category tracked independently |
| T-CTX-001-04 | `test_budget_total_cannot_exceed_context_window` | Sum of categories > window | Error on construction |
| T-CTX-001-05 | `test_budget_remaining_after_multiple_allocations` | 3 sequential allocations | `remaining()` correct |
| T-CTX-001-06 | `test_budget_release_frees_tokens` | Allocate then release | Tokens available again |

> **Audit note:** `budget.rs` was merged into `guard.rs`. Tests should reference `guard.rs` (`ContextWindowGuard`, `TokenBudget`).

#### Task: T-CTX-002 — ContextItem (was PromptSection)

```
FILE: crates/y-context/src/pipeline.rs
TEST_LOCATION: #[cfg(test)] in same file
```

> **Audit note:** `section.rs` was merged into `pipeline.rs`. `PromptSection` is now `ContextItem`.

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-002-01 | `test_section_creation` | New section with category and content | Fields accessible |
| T-CTX-002-02 | `test_section_token_estimate` | Section with known content | Token estimate reasonable |
| T-CTX-002-03 | `test_section_priority_ordering` | Sort sections by priority | Lower priority first |
| T-CTX-002-04 | `test_section_serialization` | Section to JSON | Roundtrip preserves fields |
| T-CTX-002-05 | `test_section_truncation` | Truncate to token limit | Content shortened, truncated flag set |

#### Task: T-CTX-003 — CompactionStrategy

```
FILE: crates/y-context/src/compaction.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-003-01 | `test_compaction_sliding_window` | 100 messages, window=50 | Returns last 50 |
| T-CTX-003-02 | `test_compaction_preserves_system_messages` | System message at position 0 | Always retained |
| T-CTX-003-03 | `test_compaction_summary_generation` | Summarize 50 dropped messages | Summary within token budget |
| T-CTX-003-04 | `test_compaction_triggered_at_threshold` | Messages exceed threshold | Compaction invoked |
| T-CTX-003-05 | `test_compaction_not_triggered_below_threshold` | Messages below threshold | No compaction |

#### Task: T-CTX-004 — ContextPipeline

```
FILE: crates/y-context/src/pipeline.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-004-01 | `test_pipeline_assembles_prompt` | Full pipeline execution | Prompt contains system, history, tools sections |
| T-CTX-004-02 | `test_pipeline_enforces_token_budget` | History exceeds budget | Compaction triggered, total within limit |
| T-CTX-004-03 | `test_pipeline_middleware_order` | 7 middleware in chain | Executed in priority order (50→100→...→700) |
| T-CTX-004-04 | `test_pipeline_skips_empty_sections` | No memories available | Memory section omitted |
| T-CTX-004-05 | `test_pipeline_abort_on_budget_exceeded` | Cannot fit within budget even after compaction | Error |

#### Task: T-CTX-005 — Built-in context providers (was middleware)

```
FILE: crates/y-context/src/pipeline.rs (ContextProvider implementations)
TEST_LOCATION: #[cfg(test)] in each file, or integration tests
```

> **Audit note:** The 8 planned middleware files under `middleware/` are not independently implemented. The `ContextProvider` trait in `pipeline.rs` serves the same role. Individual providers (BuildSystemPrompt, InjectMemory, etc.) are planned as future ContextProvider implementations.

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-005-01 | `test_build_system_prompt_adds_section` | BuildSystemPrompt middleware | System section added to payload |
| T-CTX-005-02 | `test_inject_memory_queries_client` | InjectMemory middleware | Calls `MemoryClient::recall()`, injects results |
| T-CTX-005-03 | `test_inject_memory_respects_budget` | InjectMemory with limited budget | Truncates to budget |
| T-CTX-005-04 | `test_inject_tools_adds_active_definitions` | InjectTools middleware | Active tools injected as function definitions |
| T-CTX-005-05 | `test_inject_tools_uses_index_not_full` | InjectTools with lazy loading | Injects ToolIndex, not full definitions |
| T-CTX-005-06 | `test_load_history_respects_budget` | LoadHistory with 1000 messages | Loads what fits in budget |
| T-CTX-005-07 | `test_inject_status_adds_context_metadata` | InjectContextStatus | Token usage, session info in context |

#### Task: T-CTX-006 — EnrichInput middleware (feature-gated)

```
FILE: crates/y-context/src/middleware/enrich_input.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CTX-006-01 | `test_enrich_ambiguous_input_triggers_analysis` | Vague user input | TaskIntentAnalyzer invoked |
| T-CTX-006-02 | `test_enrich_clear_input_skipped` | Clear, specific input | Heuristic pre-filter skips LLM call |
| T-CTX-006-03 | `test_enrich_policy_never_skips` | `EnrichmentPolicy::Never` | Always skips enrichment |
| T-CTX-006-04 | `test_enrich_policy_always_runs` | `EnrichmentPolicy::Always` | Always runs regardless of input clarity |
| T-CTX-006-05 | `test_enrich_replaces_original_input` | Enriched input produced | Original replaced in payload, original in audit |
| T-CTX-006-06 | `test_enrich_interactive_clarification` | Ambiguity detected | Interrupt triggered for user clarification |

### 4.2 Integration Tests

```
FILE: crates/y-context/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-CTX-INT-01 | `pipeline_integration_test.rs` | `test_full_context_assembly` | All 7 middleware, mock stores, verify assembled prompt |
| T-CTX-INT-02 | `pipeline_integration_test.rs` | `test_context_fits_in_budget` | Large session, verify compaction + budget enforcement |
| T-CTX-INT-03 | `pipeline_integration_test.rs` | `test_context_with_memories_and_skills` | Memory recall + skill injection, verify ordering |
| T-CTX-INT-04 | `compaction_integration_test.rs` | `test_compaction_preserves_conversation_coherence` | Compacted history still makes sense (structural test) |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority | Status |
|---------|------|-------------|----------|--------|
| I-CTX-001 | `TokenBudget` / `ContextWindowGuard` | Token monitoring with 3 trigger modes | High | ✅ Done (in `guard.rs`) |
| I-CTX-002 | `ContextItem` | Structured context unit (was PromptSection) | High | ✅ Done (in `pipeline.rs`) |
| I-CTX-003 | `ContextPipeline` | Drives ContextProvider chain, assembles final prompt | High | ✅ Done |
| I-CTX-004 | `BuildSystemPrompt` provider | System prompt construction (priority 100) | High | ❌ Planned |
| I-CTX-005 | `InjectMemory` provider | Memory recall and injection (priority 300) | High | ⚠️ Partial (via `memory/recall_middleware.rs`) |
| I-CTX-006 | `InjectTools` provider | Tool index/definition injection (priority 500) | High | ❌ Planned |
| I-CTX-007 | `LoadHistory` provider | Session history loading with compaction (priority 600) | High | ❌ Planned |
| I-CTX-008 | `CompactionEngine` | Sliding window + summary compaction | High | ✅ Done |
| I-CTX-009 | `InjectKnowledge` provider | Knowledge base injection (priority 350) | Medium | ❌ Planned |
| I-CTX-010 | `InjectSkills` provider | Skill injection (priority 400) | Medium | ❌ Planned |
| I-CTX-011 | `InjectContextStatus` provider | Context metadata injection (priority 700) | Medium | ❌ Planned |
| I-CTX-012 | `EnrichInput` provider | Input enrichment (priority 50, feature-gated) | Medium | ❌ Planned |
| I-CTX-013 | `RecallStore` | Hybrid text/vector memory recall | High | ✅ Done (new) |
| I-CTX-014 | `repair_history` | Session history repair utilities | Medium | ✅ Done (new) |
| I-CTX-015 | Memory integration (`memory/`) | LTM/STM/WM clients, dedup, search orchestrator | High | ✅ Done (new) |

---

## 6. Performance Benchmarks

```
FILE: crates/y-context/benches/context_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Context assembly (7 middleware) | P95 < 50ms | `criterion` |
| Token budget allocation | P95 < 100us | `criterion` |
| Compaction (1000 messages) | P95 < 100ms | `criterion` |
| Section truncation | P95 < 1ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-context` |
| Clippy clean | 0 warnings | `cargo clippy -p y-context` |
| Token accuracy | Within 10% of tiktoken | Comparison test |

---

## 8. Acceptance Criteria

- [ ] Context pipeline assembles prompt from all 7 middleware stages
- [ ] Token budget enforced per category and total
- [ ] Compaction triggers when history exceeds threshold
- [ ] System messages never compacted away
- [ ] Tool injection uses ToolIndex (compact), not full definitions
- [ ] Memory injection queries MemoryClient and respects budget
- [ ] Input enrichment (feature-gated) can analyze and replace input
- [ ] Assembled prompt fits within configured context window
- [ ] Coverage >= 80%
