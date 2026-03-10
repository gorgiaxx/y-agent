# R&D Plan: y-session

**Module**: `crates/y-session`
**Phase**: 2.2 (Core Runtime)
**Priority**: High — session tree is the backbone of conversation state
**Design References**: `context-session-design.md`
**Depends On**: `y-core`, `y-storage`

---

## 1. Module Purpose

`y-session` manages the session tree lifecycle: creating, branching, and persisting sessions. Metadata lives in SQLite (via `y-storage`); message transcripts live in JSONL files. It provides the `SessionManager` high-level facade that coordinates both stores and enforces state machine transitions.

---

## 2. Dependency Map

```
y-session
  ├── y-core (traits: SessionStore, TranscriptStore, SessionNode, SessionType/State)
  ├── y-storage (SqliteSessionStore, JsonlTranscriptStore — injected via trait)
  ├── tokio (async, filesystem operations)
  ├── serde / serde_json (JSONL serialization)
  ├── thiserror (errors)
  ├── tracing (session_id spans)
  └── uuid, chrono
```

---

## 3. Module Structure

```
y-session/src/
  lib.rs              — Public API: SessionManager
  error.rs            — SessionManagerError
  config.rs           — SessionConfig (transcript dir, max depth, etc.)
  manager.rs          — SessionManager: facade over store + transcript
  state_machine.rs    — Valid state transitions enforcement
  tree.rs             — Tree traversal utilities (subtree, merge detection)
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-SESS-001 — State machine

```
FILE: crates/y-session/src/state_machine.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SESS-001-01 | `test_active_to_paused_allowed` | Transition Active → Paused | Ok |
| T-SESS-001-02 | `test_active_to_archived_allowed` | Transition Active → Archived | Ok |
| T-SESS-001-03 | `test_paused_to_active_allowed` | Transition Paused → Active | Ok |
| T-SESS-001-04 | `test_archived_to_active_not_allowed` | Transition Archived → Active | Error |
| T-SESS-001-05 | `test_tombstone_to_any_not_allowed` | Transition Tombstone → * | Error for all targets |
| T-SESS-001-06 | `test_any_to_tombstone_allowed` | Transition * → Tombstone | Ok for all sources |
| T-SESS-001-07 | `test_merged_to_active_not_allowed` | Transition Merged → Active | Error |

#### Task: T-SESS-002 — SessionManager facade

```
FILE: crates/y-session/src/manager.rs
TEST_LOCATION: #[cfg(test)] in same file (with mock stores)
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SESS-002-01 | `test_manager_create_session` | `create()` main session | Calls store.create, initializes transcript |
| T-SESS-002-02 | `test_manager_branch_session` | `branch()` from existing | Creates branch with correct parent, copies no messages |
| T-SESS-002-03 | `test_manager_add_message` | `add_message()` | Appends to transcript, updates metadata counts |
| T-SESS-002-04 | `test_manager_get_messages` | `get_messages()` | Returns transcript messages |
| T-SESS-002-05 | `test_manager_pause_resume` | `pause()` then `resume()` | State transitions correctly |
| T-SESS-002-06 | `test_manager_archive` | `archive()` | State → Archived |
| T-SESS-002-07 | `test_manager_depth_limit` | Create session at max_depth | Error or enforced |
| T-SESS-002-08 | `test_manager_get_context_window` | Query token count | Returns stored token_count |

#### Task: T-SESS-003 — Tree traversal

```
FILE: crates/y-session/src/tree.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SESS-003-01 | `test_tree_subtree_from_root` | Get subtree of root | Returns all descendants |
| T-SESS-003-02 | `test_tree_subtree_from_mid_node` | Get subtree of middle node | Returns only descendants |
| T-SESS-003-03 | `test_tree_depth_calculation` | Depth of leaf node | Correct depth count |
| T-SESS-003-04 | `test_tree_path_from_root_to_leaf` | Full path traversal | Ordered from root to leaf |

### 4.2 Integration Tests

```
FILE: crates/y-session/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-SESS-INT-01 | `session_lifecycle_test.rs` | `test_full_session_lifecycle` | Create → add messages → pause → resume → archive |
| T-SESS-INT-02 | `session_lifecycle_test.rs` | `test_branch_and_diverge` | Create main → branch → add different messages to each |
| T-SESS-INT-03 | `session_lifecycle_test.rs` | `test_session_tree_with_multiple_levels` | Root → child → grandchild, verify tree queries |
| T-SESS-INT-04 | `session_lifecycle_test.rs` | `test_session_recovery_from_disk` | Create, persist, reload from SQLite + JSONL |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-SESS-001 | `SessionConfig` | Transcript directory, max depth, cleanup policy | High |
| I-SESS-002 | `StateMachine` | Valid transition table, `can_transition()`, `validate_transition()` | High |
| I-SESS-003 | `SessionManager` | High-level facade coordinating store + transcript | High |
| I-SESS-004 | Tree traversal utilities | subtree, ancestor path, depth calculation | Medium |
| I-SESS-005 | Transcript directory management | Auto-create directories, file naming convention | Medium |

---

## 6. Performance Benchmarks

```
FILE: crates/y-session/benches/session_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Session recovery (1000 messages) | < 5 seconds | `criterion` |
| Session create | P95 < 10ms | `criterion` |
| Message append | P95 < 1ms | `criterion` |
| Tree traversal (100 nodes) | P95 < 5ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-session` |
| Clippy clean | 0 warnings | `cargo clippy -p y-session` |
| State machine | 100% transition coverage | Manual review |

---

## 8. Acceptance Criteria

- [ ] State machine enforces all valid/invalid transitions
- [ ] Sessions can be created, branched, paused, resumed, archived
- [ ] Transcript append is atomic (no partial messages on crash)
- [ ] Tree queries (children, ancestors, subtree) return correct results
- [ ] Session recovery from SQLite + JSONL works after restart
- [ ] 1000-message session recovers in < 5 seconds
- [ ] Coverage >= 80%
