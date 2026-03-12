# R&D Plan: y-agent-core

**Module**: `crates/y-agent-core`
**Phase**: 3.1 (Execution Layer)
**Priority**: Critical — the orchestrator is the central execution engine
**Design References**: `orchestrator-design.md`
**Depends On**: `y-core`, `y-hooks`, `y-provider`, `y-storage`, `y-context`
**Last Audited**: 2026-03-10

---

## 1. Module Purpose

`y-agent-core` is the orchestrator: it drives the DAG-based task execution engine with typed state channels, checkpoint-based recovery, and interrupt/resume protocol. It coordinates provider calls, tool execution, context assembly, and hook invocation into a coherent agent loop.

---

## 2. Dependency Map

```
y-agent-core
  ├── y-core (traits: CheckpointStorage, ProviderPool, ToolRegistry, Middleware)
  ├── y-hooks (all 3 middleware chains: Context, Tool, LLM)
  ├── y-provider (LLM communication via ProviderPool)
  ├── y-storage (CheckpointStorage for persistence)
  ├── y-context (context assembly pipeline)
  ├── tokio (task spawning, channels, select, JoinSet)
  ├── serde_json (state serialization)
  ├── thiserror (errors)
  └── tracing (workflow_id, step_number, task spans)
```

---

## 3. Module Structure

```
y-agent-core/src/
  lib.rs              — Public API: WorkflowExecutor, TaskDag, Channel, CheckpointStore, InterruptManager
  dag.rs              — TaskDag: task dependency graph, topological execution          ✅ implemented
  channel.rs          — Channel / WorkflowContext: typed state channels with reducers  ✅ implemented
  checkpoint.rs       — CheckpointStore: committed/pending checkpoint persistence      ✅ implemented
  executor.rs         — WorkflowExecutor: DAG execution with checkpointing             ⚠️ placeholder (synchronous)
  interrupt.rs        — InterruptManager: pause/resume, HITL integration               ✅ implemented
  orchestrator.rs     — Orchestrator: main execution coordinator                       ❌ NOT YET IMPLEMENTED
  agent_loop.rs       — AgentLoop: LLM ↔ Tool loop with turn management               ❌ NOT YET IMPLEMENTED
  expression.rs       — ExpressionDSL: basic expression parser for workflow defs       ❌ NOT YET IMPLEMENTED
  compensation.rs     — CompensationManager: rollback for side-effect operations       ❌ NOT YET IMPLEMENTED
  error.rs            — OrchestratorError                                              ❌ NOT YET IMPLEMENTED
  config.rs           — OrchestratorConfig (max_steps, timeout, retry policy)          ❌ NOT YET IMPLEMENTED
```

> **Audit note (2026-03-10):** The original `checkpoint_mgr.rs` was implemented as `checkpoint.rs` with `CheckpointStore`. The planned `orchestrator.rs` and `agent_loop.rs` are subsumed by the current `executor.rs`, which is a placeholder (synchronous, no actual LLM/tool loop). Full async orchestration is deferred to a later phase.

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-ORCH-001 — DAG engine

```
FILE: crates/y-agent-core/src/dag.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-001-01 | `test_dag_linear_execution_order` | A → B → C | Executes in order |
| T-ORCH-001-02 | `test_dag_parallel_execution` | A → [B, C] → D | B and C run in parallel |
| T-ORCH-001-03 | `test_dag_diamond_dependency` | A → [B, C] → D (B,C both → D) | D waits for both |
| T-ORCH-001-04 | `test_dag_cycle_detection` | A → B → A | Error: cycle detected |
| T-ORCH-001-05 | `test_dag_single_node` | Just A | Executes correctly |
| T-ORCH-001-06 | `test_dag_empty` | No nodes | No-op |
| T-ORCH-001-07 | `test_dag_topological_sort` | Complex graph | Correct topological order |
| T-ORCH-001-08 | `test_dag_task_failure_stops_dependents` | B fails in A → [B, C] → D | D not executed, C may run |

#### Task: T-ORCH-002 — Typed channels

```
FILE: crates/y-agent-core/src/channel.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-002-01 | `test_channel_write_read` | Write value, read | Returns written value |
| T-ORCH-002-02 | `test_channel_reducer_accumulates` | Sum reducer, write 1,2,3 | Read returns 6 |
| T-ORCH-002-03 | `test_channel_version_tracking` | 3 writes | Version incremented to 3 |
| T-ORCH-002-04 | `test_channel_stale_read_detection` | Read with old version | `StaleRead` error |
| T-ORCH-002-05 | `test_channel_type_safety` | Write string, read as int | Type error |
| T-ORCH-002-06 | `test_channel_serialization_for_checkpoint` | Serialize channel state | JSON captures value + version |

#### Task: T-ORCH-003 — CheckpointStore

```
FILE: crates/y-agent-core/src/checkpoint.rs
TEST_LOCATION: #[cfg(test)] in same file (with mock CheckpointStorage)
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-003-01 | `test_checkpoint_save_pending` | Save pending state | Calls `write_pending` on storage |
| T-ORCH-003-02 | `test_checkpoint_commit` | Commit step | Calls `commit` on storage |
| T-ORCH-003-03 | `test_checkpoint_recover_from_committed` | Load committed state | Channel state restored |
| T-ORCH-003-04 | `test_checkpoint_pending_lost_on_crash` | Write pending, simulate crash | Only committed state survives |
| T-ORCH-003-05 | `test_checkpoint_step_number_monotonic` | Commit steps 1, 2, 3 | Step number always increases |
| T-ORCH-003-06 | `test_checkpoint_prune_old` | Prune steps before 5 | Steps 1-4 removed |

#### Task: T-ORCH-004 — Interrupt protocol

```
FILE: crates/y-agent-core/src/interrupt.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-004-01 | `test_interrupt_pauses_execution` | Trigger interrupt | Execution pauses, state saved |
| T-ORCH-004-02 | `test_interrupt_resume_continues` | Interrupt then resume | Execution continues from saved state |
| T-ORCH-004-03 | `test_interrupt_with_hitl_data` | HITL interrupt | Interrupt data contains user prompt |
| T-ORCH-004-04 | `test_interrupt_timeout` | Interrupt with no resume | Configurable timeout, then fail/escalate |
| T-ORCH-004-05 | `test_interrupt_checkpoint_saved` | Trigger interrupt | Checkpoint status = Interrupted |

#### Task: T-ORCH-005 — AgentLoop ❌ NOT YET IMPLEMENTED

```
FILE: crates/y-agent-core/src/agent_loop.rs (planned)
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-005-01 | `test_agent_loop_single_turn` | User message → LLM response (no tools) | Single LLM call, response returned |
| T-ORCH-005-02 | `test_agent_loop_tool_call` | LLM requests tool → tool executes → LLM responds | 2 LLM calls, tool executed between |
| T-ORCH-005-03 | `test_agent_loop_multi_tool_parallel` | LLM requests 3 tools | All 3 executed (potentially parallel) |
| T-ORCH-005-04 | `test_agent_loop_max_steps_enforced` | Loop exceeds max_steps | Stopped with error |
| T-ORCH-005-05 | `test_agent_loop_context_assembled_each_turn` | Multi-turn | Context pipeline invoked each turn |
| T-ORCH-005-06 | `test_agent_loop_llm_error_handled` | LLM returns error | Error propagated, no infinite retry |
| T-ORCH-005-07 | `test_agent_loop_checkpoint_per_step` | 3 turns | 3 checkpoints saved |

#### Task: T-ORCH-006 — Expression DSL (basic) ❌ NOT YET IMPLEMENTED

```
FILE: crates/y-agent-core/src/expression.rs (planned)
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-ORCH-006-01 | `test_expr_literal_string` | `"hello"` | Evaluates to "hello" |
| T-ORCH-006-02 | `test_expr_channel_reference` | `$channel.output` | Resolves channel value |
| T-ORCH-006-03 | `test_expr_conditional` | `if $status == "ok" then $a else $b` | Correct branch |
| T-ORCH-006-04 | `test_expr_parse_error` | Invalid syntax | ParseError |

### 4.2 Integration Tests

```
FILE: crates/y-agent-core/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-ORCH-INT-01 | `orchestrator_integration_test.rs` | `test_simple_dag_execution` | Linear DAG with mock tools, verify ordering |
| T-ORCH-INT-02 | `orchestrator_integration_test.rs` | `test_dag_with_checkpoint_recovery` | Execute 3 steps, crash, recover, continue |
| T-ORCH-INT-03 | `orchestrator_integration_test.rs` | `test_agent_loop_multi_turn` | 3-turn conversation with tool calls, mock LLM |
| T-ORCH-INT-04 | `orchestrator_integration_test.rs` | `test_interrupt_resume_flow` | HITL interrupt, provide input, resume |
| T-ORCH-INT-05 | `orchestrator_integration_test.rs` | `test_parallel_task_execution` | Diamond DAG, verify parallel execution |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority | Status |
|---------|------|-------------|----------|--------|
| I-ORCH-001 | `TaskDag` | Topological sort, parallel execution, dependency tracking | High | ✅ Done |
| I-ORCH-002 | `Channel` / `WorkflowContext` | State channels with reducers, versioning, serialization | High | ✅ Done |
| I-ORCH-003 | `CheckpointStore` | Committed/pending checkpoint persistence | High | ✅ Done |
| I-ORCH-004 | `AgentLoop` | LLM ↔ Tool loop, turn management, max_steps | High | ❌ Planned |
| I-ORCH-005 | `InterruptManager` | Pause/resume, HITL data, timeout | High | ✅ Done |
| I-ORCH-006 | `WorkflowExecutor` | DAG execution coordinator (currently placeholder) | High | ⚠️ Placeholder |
| I-ORCH-007 | `ExpressionDSL` | Basic expression parser for workflow definitions | Medium | ❌ Planned |
| I-ORCH-008 | `CompensationManager` | Rollback for failed side-effect operations | Medium | ❌ Planned |

---

## 6. Performance Benchmarks

```
FILE: crates/y-agent-core/benches/orchestrator_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| DAG topological sort (50 nodes) | P95 < 1ms | `criterion` |
| Channel write + read | P95 < 100us | `criterion` |
| Checkpoint save/commit | P95 < 10ms | `criterion` |
| Agent loop single turn (mock LLM) | P95 < 50ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 75% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-agent-core` |
| Clippy clean | 0 warnings | `cargo clippy -p y-agent-core` |
| No deadlock | Verified | Code review: channel/lock ordering |

---

## 8. Acceptance Criteria

- [ ] DAG engine executes tasks in topological order with parallel support
- [ ] Cycle detection prevents invalid DAGs
- [ ] Typed channels track versions and detect stale reads
- [ ] Checkpoint committed/pending separation works correctly
- [ ] Crash recovery restores from last committed checkpoint
- [ ] Agent loop handles multi-turn conversations with tool calls
- [ ] Interrupt/resume protocol works for HITL scenarios
- [ ] Max steps limit prevents infinite loops
- [ ] Coverage >= 75%
