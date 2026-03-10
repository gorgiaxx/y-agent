# y-agent R&D Plan — Master Index

**Version**: v0.1
**Created**: 2026-03-09
**Status**: Draft
**Author**: Claude and Gorgias

---

## 1. Overview

This document is the master index for the y-agent R&D plan. It provides an AI-automatable structure for developing and testing all 20 crates in the workspace. Each module has a dedicated plan document with:

- Exact file paths and module structure
- Test-first development tasks with unique test IDs
- Implementation tasks with unique task IDs
- Performance benchmark targets
- Quality gates and acceptance criteria

**Methodology**: All development follows strict TDD (Red-Green-Refactor). Tests are written before production code. See `docs/standards/TEST_STRATEGY.md` for the full methodology.

---

## 2. Module Plan Documents

### Phase 1 — Foundation

| Module | Plan Document | Priority | Key Deliverables |
|--------|--------------|----------|-----------------|
| y-core | [y-core.md](modules/y-core.md) | Critical | 10 test groups (50+ tests), trait contract validation, serde roundtrips |
| y-test-utils | [y-remaining.md](modules/y-remaining.md#6-y-test-utils-phase-1) | High | Mock providers, mock storage, fixtures, assertion helpers |

### Phase 2 — Core Runtime

| Module | Plan Document | Priority | Key Deliverables |
|--------|--------------|----------|-----------------|
| y-storage | [y-storage.md](modules/y-storage.md) | High | SQLite pool, migrations, CheckpointStorage, SessionStore, TranscriptStore |
| y-provider | [y-provider.md](modules/y-provider.md) | High | Tag-based routing, freeze/thaw, concurrency, OpenAI/Anthropic clients |
| y-session | [y-session.md](modules/y-session.md) | High | Session tree, state machine, JSONL transcripts, tree traversal |
| y-hooks | [y-hooks.md](modules/y-hooks.md) | High | Middleware chains, hook registry, event bus |

### Phase 3 — Execution Layer

| Module | Plan Document | Priority | Key Deliverables |
|--------|--------------|----------|-----------------|
| y-agent-core | [y-agent-core.md](modules/y-agent-core.md) | Critical | DAG engine, typed channels, checkpoint recovery, agent loop, interrupt/resume |
| y-tools | [y-tools.md](modules/y-tools.md) | High | Tool registry, lazy loading, JSON Schema validation, LRU activation |
| y-runtime | [y-runtime.md](modules/y-runtime.md) | High | Docker/Native adapters, capability enforcement, image whitelist |
| y-context | [y-context.md](modules/y-context.md) | High | Context pipeline, token budgets, compaction, 7 middleware stages |
| y-prompt | [y-remaining.md](modules/y-remaining.md#1-y-prompt-phase-34) | Medium | PromptSection, templates, mode overlays, TOML store |
| y-mcp | [y-remaining.md](modules/y-remaining.md#2-y-mcp-phase-32) | Medium | MCP client, tool adapter, memory adapter |
| y-journal | [y-remaining.md](modules/y-remaining.md#3-y-journal-phase-33) | Medium | File journal middleware, three-tier storage, rollback |
| y-scheduler | [y-remaining.md](modules/y-remaining.md#4-y-scheduler-phase-33) | Medium | Cron/interval scheduling, workflow triggering |

### Phase 4 — Intelligence Layer

| Module | Plan Document | Priority | Key Deliverables |
|--------|--------------|----------|-----------------|
| Memory System | [y-memory.md](modules/y-memory.md) | High | 3-tier memory (LTM/STM/WM), dedup, search orchestrator |
| y-skills | [y-skills.md](modules/y-skills.md) | Medium | Skill registry, version control, TOML manifests, lazy sub-docs |
| y-knowledge | [y-knowledge.md](modules/y-knowledge.md) | Medium | L0/L1/L2 chunking, Qdrant indexing, hybrid retrieval |
| y-guardrails | [y-guardrails.md](modules/y-guardrails.md) | High | Permission model, LoopGuard, taint tracking, HITL |
| y-multi-agent | [y-multi-agent.md](modules/y-multi-agent.md) | Medium | Agent pool, delegation, Sequential/Hierarchical patterns |

### Phase 5 — Integration and Release

| Module | Plan Document | Priority | Key Deliverables |
|--------|--------------|----------|-----------------|
| y-cli | [y-cli.md](modules/y-cli.md) | Medium | CLI commands, config loading, dependency wiring |
| y-diagnostics | [y-remaining.md](modules/y-remaining.md#5-y-diagnostics-phase-5) | Low | PostgreSQL traces, cost intelligence, trace replay |

---

## 3. Test ID Convention

All test IDs follow the pattern: `T-{MODULE}-{GROUP}-{SEQ}`

| Prefix | Module |
|--------|--------|
| T-CORE | y-core |
| T-STOR | y-storage |
| T-PROV | y-provider |
| T-SESS | y-session |
| T-HOOK | y-hooks |
| T-TOOL | y-tools |
| T-RT | y-runtime |
| T-CTX | y-context |
| T-ORCH | y-agent-core |
| T-GUARD | y-guardrails |
| T-MA | y-multi-agent |
| T-SKILL | y-skills |
| T-KB | y-knowledge |
| T-MEM | y-memory |
| T-CLI | y-cli |
| T-PROMPT | y-prompt |
| T-MCP | y-mcp |
| T-JRNL | y-journal |
| T-SCHED | y-scheduler |
| T-DIAG | y-diagnostics |
| T-UTIL | y-test-utils |

Integration tests use the suffix `-INT-{SEQ}` (e.g., `T-STOR-INT-01`).

---

## 4. Implementation Task ID Convention

All implementation task IDs follow: `I-{MODULE}-{SEQ}`

| Prefix | Module |
|--------|--------|
| I-CORE | y-core |
| I-STOR | y-storage |
| I-PROV | y-provider |
| I-SESS | y-session |
| I-HOOK | y-hooks |
| I-TOOL | y-tools |
| I-RT | y-runtime |
| I-CTX | y-context |
| I-ORCH | y-agent-core |
| I-GUARD | y-guardrails |
| I-MA | y-multi-agent |
| I-SKILL | y-skills |
| I-KB | y-knowledge |
| I-MEM | y-memory |
| I-CLI | y-cli |
| I-PROMPT | y-prompt |
| I-MCP | y-mcp |
| I-JRNL | y-journal |
| I-SCHED | y-scheduler |
| I-DIAG | y-diagnostics |
| I-UTIL | y-test-utils |

---

## 5. Test Statistics Summary

| Module | Unit Tests | Integration Tests | Total | Benchmark Targets |
|--------|-----------|------------------|-------|-------------------|
| y-core | ~50 | 0 | ~50 | N/A |
| y-storage | ~28 | 6 | ~34 | 7 benchmarks |
| y-provider | ~28 | 5 | ~33 | 4 benchmarks |
| y-session | ~15 | 4 | ~19 | 4 benchmarks |
| y-hooks | ~25 | 5 | ~30 | 5 benchmarks |
| y-tools | ~22 | 4 | ~26 | 5 benchmarks |
| y-runtime | ~23 | 5 | ~28 | 4 benchmarks |
| y-context | ~25 | 4 | ~29 | 4 benchmarks |
| y-agent-core | ~27 | 5 | ~32 | 4 benchmarks |
| y-guardrails | ~22 | 4 | ~26 | N/A |
| y-multi-agent | ~14 | 3 | ~17 | N/A |
| y-skills | ~15 | 3 | ~18 | N/A |
| y-knowledge | ~12 | 3 | ~15 | N/A |
| y-memory | ~20 | 5 | ~25 | N/A |
| y-cli | ~9 | 4 | ~13 | N/A |
| y-prompt | ~7 | 0 | ~7 | N/A |
| y-mcp | ~7 | 0 | ~7 | N/A |
| y-journal | ~8 | 0 | ~8 | N/A |
| y-scheduler | ~7 | 0 | ~7 | N/A |
| y-diagnostics | ~8 | 0 | ~8 | N/A |
| y-test-utils | ~6 | 0 | ~6 | N/A |
| **Total** | **~378** | **~60** | **~438** | **37 benchmarks** |

---

## 6. Dependency Execution Order

The following order respects all crate dependencies. Modules at the same indent level can be developed in parallel.

```
Phase 1 (Foundation):
  ├── y-core              ← All traits (MUST be first)
  └── y-test-utils        ← Mock implementations (parallel with y-core tests)

Phase 2 (Core Runtime — all parallel, depend only on y-core):
  ├── y-storage           ← SQLite pool, migrations, checkpoint/session stores
  ├── y-provider          ← Provider pool, routing, freeze/thaw
  ├── y-session           ← Session tree, state machine, transcripts
  └── y-hooks             ← Middleware chains, hook registry, event bus

Phase 3 (Execution — depends on Phase 2):
  ├── y-tools             ← Tool registry, lazy loading (depends: y-hooks)
  ├── y-runtime           ← Docker/Native adapters (depends: y-core only)
  ├── y-prompt            ← Prompt sections and templates (depends: y-core)
  ├── y-mcp              ← MCP protocol adapters (depends: y-core)
  ├── y-journal           ← File journal (depends: y-hooks, y-storage)
  ├── y-scheduler         ← Scheduled tasks (depends: y-storage)
  ├── y-context           ← Context pipeline (depends: y-hooks, y-session)
  └── y-agent-core        ← Orchestrator (depends: y-provider, y-storage, y-context, y-hooks)

Phase 4 (Intelligence — depends on Phase 3):
  ├── y-memory (STM)      ← Experience store (depends: y-storage)
  ├── y-memory (LTM)      ← Qdrant client (depends: y-core)
  ├── y-memory (WM)       ← Working memory (depends: y-core)
  ├── y-skills            ← Skill registry (depends: y-storage, y-hooks)
  ├── y-knowledge         ← Knowledge base (depends: y-hooks)
  ├── y-guardrails        ← Guardrail middleware (depends: y-hooks)
  └── y-multi-agent       ← Multi-agent (depends: y-agent-core, y-session)

Phase 5 (Integration):
  ├── y-cli               ← CLI binary (depends: all)
  ├── y-diagnostics       ← PostgreSQL diagnostics (depends: y-hooks, y-storage)
  └── E2E tests           ← Full system tests
```

---

## 7. AI Automation Protocol

This R&D plan is structured for AI-driven development. Each module plan provides:

### 7.1 For Test Generation (Red Phase)

1. **Test ID**: Unique identifier for tracking
2. **Test Name**: Rust function name (`test_*`)
3. **File Location**: Exact file path and test location (`#[cfg(test)]` or `tests/`)
4. **Behavior Under Test**: What the test validates
5. **Assertion**: Expected outcome

AI agents should:
- Read the module plan
- Generate the test code following the naming convention
- Place tests in the specified locations
- Run `cargo test` to confirm the test fails (Red)

### 7.2 For Implementation (Green Phase)

1. **Task ID**: Unique identifier
2. **Description**: What to implement
3. **Priority**: Implementation order within the module
4. **File Location**: From the module structure section

AI agents should:
- Read the failing tests
- Write the minimal code to pass
- Run `cargo test` to confirm green
- Run `cargo clippy` to verify lint-clean

### 7.3 For Refactoring

After Green, AI agents should:
- Check for code duplication within the module
- Extract common patterns into helper functions
- Verify all tests still pass
- Verify benchmarks don't regress (if applicable)

### 7.4 For Quality Verification

After each module is complete:
- Run `cargo test -p {crate}` — all tests pass
- Run `cargo clippy -p {crate}` — 0 warnings
- Run `cargo llvm-cov -p {crate}` — meets coverage target
- Run `cargo bench -p {crate}` — meets performance targets (if applicable)
- Run `cargo doc -p {crate} --no-deps` — 0 warnings

---

## 8. Coverage Targets Summary

| Module | Minimum | Aspirational |
|--------|---------|-------------|
| y-core | 90% | 95% |
| y-storage | 80% | 90% |
| y-provider | 80% | 90% |
| y-session | 80% | 90% |
| y-hooks | 80% | 90% |
| y-tools | 80% | 90% |
| y-runtime | 75% | 85% |
| y-context | 80% | 90% |
| y-agent-core | 75% | 85% |
| y-guardrails | 80% | 90% |
| y-multi-agent | 75% | 85% |
| y-skills | 80% | 90% |
| y-knowledge | 75% | 85% |
| y-cli | 70% | 80% |
| Supporting modules | 70% | 80% |
| **Overall workspace** | **75%** | **85%** |

---

## 9. Performance Targets Summary

| Metric | Target | Module |
|--------|--------|--------|
| Tool dispatch latency (excl. LLM) | P95 < 100ms | y-tools |
| Middleware chain (10 middleware) | P95 < 5ms | y-hooks |
| Checkpoint write | P95 < 10ms | y-storage |
| Session recovery (1000 messages) | < 5 seconds | y-session |
| Context assembly (7 middleware) | P95 < 50ms | y-context |
| Provider routing (10 providers) | P95 < 1ms | y-provider |
| JSON Schema validation | P95 < 1ms | y-tools |
| DAG topological sort (50 nodes) | P95 < 1ms | y-agent-core |
| Native execution (echo) | P95 < 50ms | y-runtime |
| Event bus dispatch (1000 events) | P95 < 10ms | y-hooks |

