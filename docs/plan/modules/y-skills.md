# R&D Plan: y-skills

**Module**: `crates/y-skills`
**Phase**: 4.2 (Intelligence Layer)
**Priority**: Medium — skill-based reasoning enhances agent capability
**Design References**: `skills-knowledge-design.md`, `skill-versioning-evolution-design.md`
**Depends On**: `y-core`, `y-hooks`, `y-storage`

---

## 1. Module Purpose

`y-skills` manages the skill lifecycle: multi-format ingestion, LLM-assisted transformation into tree-indexed proprietary format, atomic registry operations, lazy sub-document loading, Git-like version control, and self-evolution pipeline. Skills are LLM-instruction-only artifacts — no embedded tools or scripts.

---

## 2. Dependency Map

```
y-skills
  ├── y-core (traits: SkillRegistry, SkillManifest, SkillVersion)
  ├── y-hooks (SkillAudit events)
  ├── y-storage (version store persistence)
  ├── toml (skill manifest parsing)
  ├── tokio (async file I/O)
  ├── serde / serde_json (serialization)
  ├── sha2 (content-addressable hashing)
  ├── thiserror (errors)
  └── tracing (skill_id, version spans)
```

---

## 3. Module Structure

```
y-skills/src/
  lib.rs              — Public API: SkillRegistryImpl
  error.rs            — SkillModuleError
  config.rs           — SkillConfig (store path, max root tokens, evolution policy)
  registry.rs         — SkillRegistryImpl: SkillRegistry trait impl
  ingestion.rs        — IngestionPipeline: multi-format input → proprietary format
  transformer.rs      — LlmTransformer: LLM-assisted content restructuring
  version.rs          — VersionStore: content-addressable store, JSONL reflog
  manifest.rs         — ManifestParser: TOML parsing with token estimation
  search.rs           — SkillSearch: tag matching, trigger pattern matching
  evolution.rs        — EvolutionPipeline: experience → pattern → proposal → approval
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-SKILL-001 — Manifest parsing

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SKILL-001-01 | `test_manifest_parse_valid_toml` | Valid TOML manifest | All fields populated |
| T-SKILL-001-02 | `test_manifest_token_estimate` | Root content analysis | Token estimate within 10% of actual |
| T-SKILL-001-03 | `test_manifest_root_exceeds_2000_tokens` | Oversized root | `TokenBudgetExceeded` error |
| T-SKILL-001-04 | `test_manifest_sub_document_refs` | Manifest with sub-docs | References parsed correctly |
| T-SKILL-001-05 | `test_manifest_serialization_roundtrip` | TOML → struct → TOML | Identity |

#### Task: T-SKILL-002 — Version store

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SKILL-002-01 | `test_version_store_creates_hash` | Store content | Returns content-addressable hash |
| T-SKILL-002-02 | `test_version_store_dedup` | Store same content twice | Same hash, single storage |
| T-SKILL-002-03 | `test_version_store_reflog_append` | Register new version | Reflog entry appended |
| T-SKILL-002-04 | `test_version_store_rollback` | Rollback to previous version | Active version changes |
| T-SKILL-002-05 | `test_version_store_history` | 3 versions | History returns all 3 in order |

#### Task: T-SKILL-003 — Skill search

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SKILL-003-01 | `test_search_by_tag` | Search "rust" | Returns skills tagged "rust" |
| T-SKILL-003-02 | `test_search_by_trigger_pattern` | Query matches trigger | Skill returned |
| T-SKILL-003-03 | `test_search_respects_limit` | 10 matches, limit=3 | Returns 3 |
| T-SKILL-003-04 | `test_search_no_match` | Unmatched query | Empty results |
| T-SKILL-003-05 | `test_search_returns_summaries_not_full` | Search results | `SkillSummary` without `root_content` |

#### Task: T-SKILL-004 — Registry operations

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SKILL-004-01 | `test_registry_register_new_skill` | Register skill | Retrievable, version created |
| T-SKILL-004-02 | `test_registry_update_creates_new_version` | Register same ID | New version, old version in history |
| T-SKILL-004-03 | `test_registry_get_manifest` | `get_manifest()` | Returns full manifest with root_content |
| T-SKILL-004-04 | `test_registry_load_sub_document` | `load_sub_document()` | Returns content for sub-doc ID |
| T-SKILL-004-05 | `test_registry_rollback` | `rollback()` | Active version changes to target |

### 4.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-SKILL-INT-01 | `skill_lifecycle_test.rs` | `test_full_skill_lifecycle` | Ingest → register → search → load → update → rollback |
| T-SKILL-INT-02 | `skill_lifecycle_test.rs` | `test_sub_document_lazy_loading` | Register with 3 sub-docs, load each on demand |
| T-SKILL-INT-03 | `skill_lifecycle_test.rs` | `test_version_history_integrity` | 5 updates, verify full history |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-SKILL-001 | `ManifestParser` | TOML parsing, token estimation, validation | High |
| I-SKILL-002 | `VersionStore` | Content-addressable store, JSONL reflog | High |
| I-SKILL-003 | `SkillRegistryImpl` | Full `SkillRegistry` trait impl | High |
| I-SKILL-004 | `SkillSearch` | Tag + trigger pattern matching | High |
| I-SKILL-005 | `IngestionPipeline` | Multi-format input processing | Medium |
| I-SKILL-006 | `LlmTransformer` | LLM-assisted restructuring (requires provider) | Medium |
| I-SKILL-007 | `EvolutionPipeline` | Experience-based skill improvement | Low (deferred) |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-skills` |
| Token constraint | All root docs < 2000 tokens | Validation test |

---

## 7. Acceptance Criteria

- [ ] TOML manifests parse with token estimation
- [ ] Root document enforces < 2,000 token limit
- [ ] Sub-documents load on demand (lazy)
- [ ] Git-like version control with content-addressable hashing
- [ ] Rollback restores previous version atomically
- [ ] Search returns compact summaries, not full content
- [ ] No tools or scripts embedded in skill content
- [ ] Coverage >= 80%
