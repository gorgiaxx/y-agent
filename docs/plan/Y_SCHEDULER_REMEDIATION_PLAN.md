# Y-Scheduler Remediation Plan

**Module**: `y-scheduler`
**Design Reference**: [`scheduled-tasks-design.md`](file:///Users/gorgias/Projects/y-agent/docs/design/scheduled-tasks-design.md)
**R&D Plan Reference**: [`y-remaining.md`](file:///Users/gorgias/Projects/y-agent/docs/plan/modules/y-remaining.md#4-y-scheduler-phase-33)
**Date**: 2026-03-11
**Status**: Proposed

---

## Audit Summary

The `y-scheduler` crate has a foundational skeleton — data structures, an in-memory store, simplified trigger types, and a placeholder executor. However, most of the design document's requirements are **not yet implemented**. 18 unit tests pass, all covering basic data-layer correctness.

### Current Files

| File | LoC | Status |
|------|-----|--------|
| `lib.rs` | 28 | Module re-exports only |
| `config.rs` | 84 | ✅ `SchedulerConfig`, `MissedPolicy`, `ConcurrencyPolicy` enums complete |
| `cron.rs` | 126 | ⚠️ Simplified interval-based parser; no real cron parsing |
| `interval.rs` | 60 | ✅ Basic `IntervalSchedule` with `next_fire` |
| `event.rs` | 83 | ⚠️ Data model only; no Hook System bridge |
| `executor.rs` | 168 | ⚠️ Placeholder; no Orchestrator integration, no concurrency policy |
| `store.rs` | 194 | ⚠️ In-memory `Vec`-based; no SQLite persistence |

### Test Summary

18 tests pass across all modules. Tests cover basic struct creation, serialization, store CRUD, simple executor trigger, and cron interval parsing.

---

## Gap Analysis

### Gap G1 — No `SchedulerManager` (Orchestrator Entry Point)

**Design requirement**: A top-level `SchedulerManager` that owns the trigger loop, registry, executor, and recovery manager. Exposes `start()`, `stop()`, `register_schedule()`, `remove_schedule()`.

**Current state**: No `SchedulerManager` exists. `ScheduleStore` and `ScheduleExecutor` are disconnected components.

**Impact**: High — without this, there is no way to run the scheduler as a service.

---

### Gap G2 — No Async Trigger Loop

**Design requirement**: An async loop that evaluates all active triggers (cron, interval, event, one-time) on each tick, enqueues fired triggers into a Trigger Queue, and dispatches to the executor.

**Current state**: No async code exists in the crate. `tokio` is listed as a dependency but unused.

**Impact**: High — the scheduler cannot actually run.

---

### Gap G3 — No Real Cron Parser

**Design requirement**: Standard 5-field cron expression parsing with timezone support (e.g. `"0 9 * * MON"`).

**Current state**: `CronSchedule::parse_simple` handles only 4 trivial patterns (`*/N`, `0 */N`, `0 *`, `0 N`). No day-of-week, month, or day-of-month support. No timezone handling.

**Impact**: Medium — many standard cron expressions will silently fail (return `None`).

---

### Gap G4 — No OneTime Trigger

**Design requirement**: `OneTime { at: DateTime<Utc> }` trigger variant that fires once at a specific time and then auto-disables.

**Current state**: `TriggerConfig::OneTime` exists in `store.rs` as a variant, but there is no `OneTimeSchedule` struct or logic to evaluate it.

**Impact**: Low-Medium — one-time delayed execution is a documented feature.

---

### Gap G5 — No Recovery Manager

**Design requirement**: On startup, load persisted schedules, detect missed fires (last_fire + interval < now), and apply `MissedPolicy` (catch_up / skip / backfill). The state diagram in the design doc shows a full recovery flow.

**Current state**: `MissedPolicy` enum exists in `config.rs`, but no recovery logic exists.

**Impact**: High — the system cannot survive restarts without data loss.

---

### Gap G6 — No SQLite Persistence

**Design requirement**: Schedule definitions and execution history persisted to SQLite with WAL mode. Survive restarts.

**Current state**: `ScheduleStore` is `Vec<Schedule>` in memory. Comment says "In production, persisted to SQLite with WAL mode" but no implementation.

**Impact**: High — all state lost on restart.

---

### Gap G7 — No Concurrency Policy Enforcement

**Design requirement**: When a schedule triggers while a previous execution is still running, enforce `allow` / `skip_if_running` / `queue` / `cancel_previous`.

**Current state**: `ConcurrencyPolicy` enum exists in `config.rs`. `ScheduleExecutor` ignores it entirely and always creates a new execution.

**Impact**: Medium — overlapping executions not guarded.

---

### Gap G8 — No Orchestrator Integration

**Design requirement**: `ScheduleExecutor` translates triggers into standard Orchestrator Workflow executions. Scheduled tasks reuse existing workflow definitions with full workflow features.

**Current state**: Executor is a placeholder that immediately marks execution as `Completed`. Comment says "integrates with Orchestrator in Phase 5".

**Impact**: High — but acceptable to defer since the Orchestrator workflow system (`y-agent-core` DAG engine) is also under development. The executor should at minimum define an `async` trait or callback for workflow dispatch.

---

### Gap G9 — No Parameter Resolution Engine

**Design requirement**: Parameters resolved at trigger time through a resolution chain: (1) defaults from ParameterSchema → (2) static `parameter_values` → (3) dynamic expressions resolved at trigger time (trigger context, event payload). Support `{{ trigger.time }}`, `{{ event.payload.field }}`.

**Current state**: `Schedule.parameter_values` is stored as `serde_json::Value` but never resolved. `ScheduleContext.resolved_parameters` exists but is never populated from values.

**Impact**: Medium — parameterized scheduling is a key design feature.

---

### Gap G10 — No Event Bridge (Hook System Integration)

**Design requirement**: An Event Bridge connects the `y-hooks` event bus to `EventSchedule` triggers.

**Current state**: `EventSchedule` has a `matches_event_type` method but no integration with `y-hooks`. No debounce implementation.

**Impact**: Medium — event-driven scheduling is non-functional.

---

### Gap G11 — No Observability / Metrics

**Design requirement**: Emit `schedule.trigger.fired`, `schedule.trigger.missed`, `schedule.execution.duration`, `schedule.active.count`, `schedule.queue.depth` metrics. Execution history queryable.

**Current state**: No `tracing` spans or metrics. `tracing` is a dependency but unused.

**Impact**: Low-Medium — observability is important but not blocking core functionality.

---

### Gap G12 — No Execution Policies on `Schedule`

**Design requirement**: Each `Schedule` carries per-schedule `missed_policy`, `concurrency_policy`, `max_executions_per_hour`, `description`, `tags`, `metadata`.

**Current state**: `Schedule` struct has no policy fields, no metadata, no description, no tags.

**Impact**: Medium — policies default to global config but cannot be overridden per-schedule.

---

## Remediation Phases

### Phase S1: Core Data Model Enhancement (Priority: High)

Expand `Schedule` struct and add missing fields. Add `OneTimeSchedule` evaluation.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `store.rs` | MODIFY | Add `missed_policy`, `concurrency_policy`, `max_executions_per_hour`, `description`, `tags`, `metadata`, `updated_at` fields to `Schedule`. Add `update()` method. |
| `cron.rs` | MODIFY | Integrate `cron` crate (or `croner`) for full 5-field cron parsing. Keep `timezone` support. Replace `parse_simple`. |
| `onetime.rs` | NEW | `OneTimeSchedule` with `should_fire(now)` logic. |
| `lib.rs` | MODIFY | Add `pub mod onetime`, re-export. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S1-01 | `test_schedule_with_policies` | Per-schedule policy fields stored/retrieved |
| T-S1-02 | `test_cron_parse_complex_expressions` | Day-of-week, month patterns parsed correctly |
| T-S1-03 | `test_cron_next_fire_respects_dow` | `"0 9 * * MON"` → next Monday 9AM |
| T-S1-04 | `test_onetime_fires_at_target` | Fires when `now >= at` |
| T-S1-05 | `test_onetime_does_not_fire_early` | Does not fire when `now < at` |

---

### Phase S2: SchedulerManager & Async Trigger Loop (Priority: High)

Create the central `SchedulerManager` that runs the async trigger evaluation loop.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `manager.rs` | NEW | `SchedulerManager` struct with `start()`, `stop()`, `register()`, `remove()`, `pause()`, `resume()`. Owns `ScheduleStore`, `ScheduleExecutor`. Runs `tokio::spawn` trigger loop with configurable tick interval. |
| `trigger.rs` | NEW | `TriggerEngine` trait + `evaluate_trigger()` function that evaluates all trigger types and returns `Vec<FiredTrigger>`. |
| `queue.rs` | NEW | `TriggerQueue` — async bounded channel (`tokio::sync::mpsc`) for enqueuing fired triggers. |
| `lib.rs` | MODIFY | Add modules, re-export `SchedulerManager`. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S2-01 | `test_manager_start_stop` | Manager starts and shuts down cleanly |
| T-S2-02 | `test_manager_register_schedule` | Schedule registered and appears in store |
| T-S2-03 | `test_trigger_engine_cron_fires` | Cron trigger fires when next_fire <= now |
| T-S2-04 | `test_trigger_engine_interval_fires` | Interval trigger fires correctly |
| T-S2-05 | `test_trigger_queue_enqueue_dequeue` | MPSC channel works correctly |

---

### Phase S3: Concurrency & Missed Policy Enforcement (Priority: Medium-High)

Implement the runtime enforcement of concurrency and missed schedule policies.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `executor.rs` | MODIFY | Make `trigger_execution` async. Add `running_executions: HashMap<String, JoinHandle>` tracking. Enforce `ConcurrencyPolicy` before dispatching. Add `WorkflowDispatcher` trait for Orchestrator integration point. |
| `recovery.rs` | NEW | `RecoveryManager`: on startup, iterate persisted schedules, compute missed fires, apply `MissedPolicy`. |
| `manager.rs` | MODIFY | Call `RecoveryManager::recover()` in `start()`. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S3-01 | `test_skip_if_running_policy` | Second trigger skipped when first still running |
| T-S3-02 | `test_queue_policy` | Second trigger queued, fires after first completes |
| T-S3-03 | `test_allow_policy` | Both executions run in parallel |
| T-S3-04 | `test_recovery_catch_up` | Missed fire → single catch-up execution |
| T-S3-05 | `test_recovery_skip` | Missed fire → skip, next scheduled normally |
| T-S3-06 | `test_recovery_backfill` | Missed fires → all replayed in sequence |

---

### Phase S4: SQLite Persistence (Priority: Medium)

Replace in-memory `Vec<Schedule>` with SQLite-backed store.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `store.rs` | MODIFY | Add `SqliteScheduleStore` implementing a `ScheduleRepository` trait. Keep in-memory `ScheduleStore` for tests. Schema: `schedules` table, `schedule_executions` table. Use `y-storage` connection pool. |
| `Cargo.toml` | MODIFY | Add optional `y-storage` dependency behind feature flag `persistence`. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S4-01 | `test_sqlite_store_crud` | Create/read/update/delete schedule in SQLite |
| T-S4-02 | `test_sqlite_execution_history` | Execution records persisted and queryable |
| T-S4-03 | `test_sqlite_store_survives_reload` | Store data persists across re-instantiation |

---

### Phase S5: Parameter Resolution Engine (Priority: Medium)

Implement the parameter resolution chain with expression support.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `params.rs` | NEW | `ParameterResolver`: applies 3-step resolution (defaults → static values → dynamic expressions). Resolve `{{ trigger.time }}`, `{{ trigger.type }}`, `{{ event.payload.* }}`, `{{ execution.sequence }}`. |
| `executor.rs` | MODIFY | Call `ParameterResolver::resolve()` before dispatching workflow. Populate `ScheduleContext.resolved_parameters`. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S5-01 | `test_param_resolution_defaults` | Schema defaults applied |
| T-S5-02 | `test_param_resolution_static_override` | Static values override defaults |
| T-S5-03 | `test_param_resolution_expression` | `{{ trigger.time }}` resolved to current time |
| T-S5-04 | `test_param_resolution_event_payload` | `{{ event.payload.path }}` resolved from event |

---

### Phase S6: Event Bridge & Hook Integration (Priority: Medium)

Wire `EventSchedule` triggers to the `y-hooks` event bus.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `event_bridge.rs` | NEW | `EventBridge`: implements `y-hooks::EventSubscriber`. On event, match against registered `EventSchedule` triggers, apply debounce window, enqueue matching triggers. |
| `event.rs` | MODIFY | Add debounce state tracking. Add `matches_payload()` for filter evaluation. |
| `Cargo.toml` | MODIFY | Add optional `y-hooks` dependency behind feature flag `events`. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S6-01 | `test_event_bridge_matches_event_type` | Matching event fires trigger |
| T-S6-02 | `test_event_bridge_filter_mismatch` | Non-matching event ignored |
| T-S6-03 | `test_event_bridge_debounce` | Rapid events collapsed within debounce window |

---

### Phase S7: Observability (Priority: Low-Medium)

Add tracing spans and metrics.

#### Files

| File | Action | Changes |
|------|--------|---------|
| `manager.rs` | MODIFY | Add `tracing::instrument` to key operations. Add counter/gauge metrics for fired/missed/active/queue-depth. |
| `executor.rs` | MODIFY | Add execution duration histogram span. |

#### Tests

| ID | Test | Validates |
|----|------|-----------|
| T-S7-01 | `test_tracing_spans_emitted` | Key operations emit tracing spans |

---

## Dependency Graph

```
Phase S1 (Data Model) ──┬──> Phase S2 (Manager + Loop) ──> Phase S3 (Policies)
                        │                                        │
                        └──> Phase S5 (Params)                   ▼
                                                           Phase S4 (SQLite)
                                                                 │
Phase S6 (Events) ─────────────────────────────────────────>     │
                                                                 ▼
                                                           Phase S7 (Observability)
```

---

## Estimated Effort

| Phase | Estimated Files | Estimated LoC | Effort |
|-------|----------------|--------------|--------|
| S1 — Data Model | 4 | ~200 | 1 day |
| S2 — Manager + Loop | 4 | ~400 | 2 days |
| S3 — Policies | 3 | ~350 | 1.5 days |
| S4 — SQLite | 2 | ~300 | 1.5 days |
| S5 — Params | 2 | ~200 | 1 day |
| S6 — Events | 3 | ~250 | 1 day |
| S7 — Observability | 2 | ~100 | 0.5 days |
| **Total** | **~20** | **~1800** | **~8.5 days** |

---

## Verification Plan

### Automated Tests

All tests run via:

```bash
cargo test -p y-scheduler
```

After Phase S4 (SQLite), integration tests require:

```bash
cargo test -p y-scheduler --features persistence
```

After Phase S6 (Events), feature-gated tests:

```bash
cargo test -p y-scheduler --features events
```

Full workspace check:

```bash
cargo clippy -p y-scheduler --all-features -- -D warnings
```

### Quality Gates

| Gate | Target |
|------|--------|
| Test coverage | ≥ 70% |
| All tests pass | 100% |
| Clippy clean | 0 warnings |
| Design conformance | All 12 gaps addressed |
