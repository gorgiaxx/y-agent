# R&D Plan: y-storage

**Module**: `crates/y-storage`
**Phase**: 2.3 (Core Runtime)
**Priority**: High — persistence foundation for checkpoints, sessions, file journal
**Design References**: `orchestrator-design.md`, `DATABASE_SCHEMA.md`
**Depends On**: `y-core`

---

## 1. Module Purpose

`y-storage` provides the persistence layer for all operational data. It manages SQLite connection pools (WAL mode), migration execution, and concrete implementations of `CheckpointStorage`, `SessionStore`, and generic repository patterns. PostgreSQL support for diagnostics is gated behind the `diagnostics_pg` feature flag.

---

## 2. Dependency Map

```
y-storage
  ├── y-core (traits: CheckpointStorage, SessionStore, TranscriptStore)
  ├── sqlx (SQLite + optional PostgreSQL)
  ├── tokio (async runtime)
  ├── serde / serde_json (serialization)
  ├── thiserror (error types)
  ├── tracing (instrumentation)
  └── uuid, chrono (ID generation, timestamps)
```

---

## 3. Module Structure

```
y-storage/src/
  lib.rs              — Public API re-exports
  error.rs            — StorageError enum
  config.rs           — StorageConfig (paths, pool size, WAL settings)
  pool.rs             — SQLite connection pool setup with WAL pragmas
  migration.rs        — Migration runner (sqlx-based)
  checkpoint.rs       — SqliteCheckpointStorage impl of CheckpointStorage
  session_store.rs    — SqliteSessionStore impl of SessionStore
  transcript.rs       — JsonlTranscriptStore impl of TranscriptStore
  repository.rs       — Generic repository trait and base helpers
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-STOR-001 — StorageConfig validation

```
FILE: crates/y-storage/src/config.rs
TEST_LOCATION: crates/y-storage/src/config.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-001-01 | `test_config_default_values` | `StorageConfig::default()` | WAL mode enabled, pool_size > 0 |
| T-STOR-001-02 | `test_config_validate_empty_path_fails` | `validate()` with empty db_path | Returns error |
| T-STOR-001-03 | `test_config_validate_valid_config` | `validate()` with valid config | Returns Ok |
| T-STOR-001-04 | `test_config_deserialization_from_toml` | TOML → `StorageConfig` | All fields parsed |

#### Task: T-STOR-002 — SQLite pool setup

```
FILE: crates/y-storage/src/pool.rs
TEST_LOCATION: crates/y-storage/src/pool.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-002-01 | `test_pool_create_in_memory` | Create pool with `:memory:` | Pool is usable |
| T-STOR-002-02 | `test_pool_wal_mode_enabled` | Check PRAGMA after connect | `journal_mode = wal` |
| T-STOR-002-03 | `test_pool_foreign_keys_enabled` | Check PRAGMA after connect | `foreign_keys = 1` |
| T-STOR-002-04 | `test_pool_busy_timeout_set` | Check PRAGMA after connect | `busy_timeout = 5000` |

#### Task: T-STOR-003 — Migration runner

```
FILE: crates/y-storage/src/migration.rs
TEST_LOCATION: crates/y-storage/src/migration.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-003-01 | `test_migration_run_creates_tables` | Run all migrations on fresh DB | All expected tables exist |
| T-STOR-003-02 | `test_migration_idempotent` | Run migrations twice | No error on second run |
| T-STOR-003-03 | `test_migration_version_tracking` | Check migration metadata | Version numbers recorded |

#### Task: T-STOR-004 — CheckpointStorage implementation

```
FILE: crates/y-storage/src/checkpoint.rs
TEST_LOCATION: crates/y-storage/src/checkpoint.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-004-01 | `test_checkpoint_write_pending_then_read_committed_is_none` | Write pending, read committed | Returns `None` |
| T-STOR-004-02 | `test_checkpoint_commit_makes_state_durable` | Write pending → commit → read committed | Returns committed state |
| T-STOR-004-03 | `test_checkpoint_overwrite_pending` | Write pending twice | Second write overwrites first |
| T-STOR-004-04 | `test_checkpoint_set_interrupted` | `set_interrupted()` | Status changes to `Interrupted`, interrupt_data set |
| T-STOR-004-05 | `test_checkpoint_set_completed` | `set_completed()` | Status changes to `Completed` |
| T-STOR-004-06 | `test_checkpoint_set_failed` | `set_failed()` | Status changes to `Failed`, error stored |
| T-STOR-004-07 | `test_checkpoint_prune_old_steps` | `prune()` | Steps before threshold deleted, returns count |
| T-STOR-004-08 | `test_checkpoint_stale_detection` | Concurrent version conflict | Returns `StaleCheckpoint` error |
| T-STOR-004-09 | `test_checkpoint_not_found` | Read non-existent workflow | Returns `None` (not error) |
| T-STOR-004-10 | `test_checkpoint_crash_safety_pending_lost` | Write pending, simulate crash (drop pool), reconnect | Pending data is gone, committed intact |

#### Task: T-STOR-005 — SessionStore implementation

```
FILE: crates/y-storage/src/session_store.rs
TEST_LOCATION: crates/y-storage/src/session_store.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-005-01 | `test_session_create_root` | Create main session with no parent | Root ID equals session ID, depth=0 |
| T-STOR-005-02 | `test_session_create_child` | Create child of existing session | parent_id set, depth=1, path includes parent |
| T-STOR-005-03 | `test_session_create_branch` | Create branch from session | session_type=Branch, shares root_id |
| T-STOR-005-04 | `test_session_get_by_id` | `get()` existing session | Returns correct node |
| T-STOR-005-05 | `test_session_get_not_found` | `get()` non-existent ID | Returns `NotFound` error |
| T-STOR-005-06 | `test_session_list_by_state` | `list()` with state filter | Returns only matching sessions |
| T-STOR-005-07 | `test_session_list_by_agent` | `list()` with agent_id filter | Returns only agent's sessions |
| T-STOR-005-08 | `test_session_set_state` | `set_state()` Active → Paused | State updated |
| T-STOR-005-09 | `test_session_set_state_invalid_transition` | `set_state()` Tombstone → Active | Returns `InvalidStateTransition` error |
| T-STOR-005-10 | `test_session_update_metadata` | `update_metadata()` | title, token_count, message_count updated |
| T-STOR-005-11 | `test_session_children` | `children()` of parent | Returns all direct children |
| T-STOR-005-12 | `test_session_ancestors` | `ancestors()` of deep node | Returns full path from root |

#### Task: T-STOR-006 — TranscriptStore (JSONL)

```
FILE: crates/y-storage/src/transcript.rs
TEST_LOCATION: crates/y-storage/src/transcript.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-STOR-006-01 | `test_transcript_append_and_read_all` | Append 3 messages, read_all | Returns 3 messages in order |
| T-STOR-006-02 | `test_transcript_read_last_n` | Append 10, read_last(3) | Returns last 3 messages |
| T-STOR-006-03 | `test_transcript_message_count` | Append N messages | `message_count()` returns N |
| T-STOR-006-04 | `test_transcript_empty_session` | Read from empty | Returns empty vec |
| T-STOR-006-05 | `test_transcript_jsonl_format` | Append, read raw file | Each line is valid JSON |
| T-STOR-006-06 | `test_transcript_concurrent_append` | Parallel appends | All messages present (no corruption) |

### 4.2 Integration Tests

```
FILE: crates/y-storage/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-STOR-INT-01 | `checkpoint_integration_test.rs` | `test_full_checkpoint_lifecycle` | write_pending → commit → set_interrupted → resume → set_completed |
| T-STOR-INT-02 | `checkpoint_integration_test.rs` | `test_checkpoint_recovery_after_crash` | Write pending, drop connection, reconnect, verify committed state |
| T-STOR-INT-03 | `session_integration_test.rs` | `test_session_tree_construction` | Create root → child → grandchild → branch, verify tree structure |
| T-STOR-INT-04 | `session_integration_test.rs` | `test_session_with_transcript` | Create session, append messages, verify transcript integrity |
| T-STOR-INT-05 | `migration_integration_test.rs` | `test_all_migrations_up_down` | Run all up, verify tables, run all down, verify clean |
| T-STOR-INT-06 | `migration_integration_test.rs` | `test_migration_data_preservation` | Insert data, run new migration, verify old data intact |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-STOR-001 | `StorageConfig` struct | Config with defaults, TOML deserialization, validation | High |
| I-STOR-002 | `StorageError` enum | Crate-level error with `thiserror`, `ClassifiedError` impl | High |
| I-STOR-003 | SQLite pool factory | `create_pool()` with WAL pragmas, foreign keys, busy timeout | High |
| I-STOR-004 | Migration runner | sqlx-based migration from `migrations/sqlite/` directory | High |
| I-STOR-005 | `SqliteCheckpointStorage` | Full `CheckpointStorage` trait impl with committed/pending | High |
| I-STOR-006 | `SqliteSessionStore` | Full `SessionStore` trait impl with tree operations | High |
| I-STOR-007 | `JsonlTranscriptStore` | JSONL file-based `TranscriptStore` with append + read | High |
| I-STOR-008 | Initial SQLite migrations | All 6 migration files (sessions, checkpoints, journal, tools, schedules, experience) | High |
| I-STOR-009 | PostgreSQL pool (feature-gated) | `diagnostics_pg` feature flag pool setup | Low (Phase 5) |

---

## 6. Performance Benchmarks

```
FILE: crates/y-storage/benches/storage_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Checkpoint write (pending) | P95 < 10ms | `criterion` |
| Checkpoint commit | P95 < 10ms | `criterion` |
| Checkpoint read committed | P95 < 5ms | `criterion` |
| Session create | P95 < 10ms | `criterion` |
| Transcript append (single) | P95 < 1ms | `criterion` |
| Transcript read_all (1000 msgs) | P95 < 50ms | `criterion` |
| Migration run (all 6) | < 500ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-storage` |
| Clippy clean | 0 warnings | `cargo clippy -p y-storage` |
| No unsafe | 0 blocks | Manual review |
| Benchmarks | No regression > 10% P95 | `cargo bench -p y-storage` |

---

## 8. Acceptance Criteria

- [ ] SQLite pool connects with WAL mode and correct pragmas
- [ ] All 6 migration files execute forward and backward cleanly
- [ ] `CheckpointStorage` passes all 10 contract tests including crash safety
- [ ] `SessionStore` passes all 12 tests including tree operations
- [ ] `TranscriptStore` passes all 6 tests including concurrent append
- [ ] All 6 integration tests pass with real in-memory SQLite
- [ ] Benchmark baselines established and committed
- [ ] Coverage >= 80%
