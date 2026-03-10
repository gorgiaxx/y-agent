# R&D Plan: y-hooks

**Module**: `crates/y-hooks`
**Phase**: 2.4 (Core Runtime)
**Priority**: High — middleware chains are the extension backbone
**Design References**: `hooks-plugin-design.md`
**Depends On**: `y-core`

---

## 1. Module Purpose

`y-hooks` implements the hook/middleware/event bus system. It provides 5 middleware chains (Context, Tool, LLM, Compaction, Memory), 21 lifecycle hook points, and an async event bus. Guardrails, file journaling, context assembly, and skill auditing are all implemented as middleware within this system.

---

## 2. Dependency Map

```
y-hooks
  ├── y-core (traits: Middleware, HookHandler, EventSubscriber, Event, ChainType)
  ├── tokio (channels: mpsc, broadcast; task spawning; timers)
  ├── serde_json (payload manipulation)
  ├── thiserror (errors)
  ├── tracing (middleware_name, chain_type, priority spans)
  └── futures (stream utilities)
```

---

## 3. Module Structure

```
y-hooks/src/
  lib.rs              — Public API: HookSystem, MiddlewareChain, EventBus
  error.rs            — HookError enum
  config.rs           — HookConfig (timeouts, channel capacities)
  chain.rs            — MiddlewareChain: priority-sorted execution pipeline
  chain_runner.rs     — ChainRunner: execute chain with timeout and abort handling
  hook_registry.rs    — HookRegistry: register/dispatch hook handlers
  event_bus.rs        — EventBus: broadcast-based async event delivery
  plugin.rs           — PluginLoader: dynamic middleware loading skeleton
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-HOOK-001 — MiddlewareChain

```
FILE: crates/y-hooks/src/chain.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-HOOK-001-01 | `test_chain_executes_in_priority_order` | 3 middleware at priority 100, 200, 300 | Execution order matches priority (ascending) |
| T-HOOK-001-02 | `test_chain_passes_context_between_middleware` | Middleware A modifies payload, B reads it | B sees A's changes |
| T-HOOK-001-03 | `test_chain_short_circuit_stops_execution` | Middleware B returns ShortCircuit | C never executes |
| T-HOOK-001-04 | `test_chain_abort_stops_execution` | Middleware B calls `ctx.abort()` | C never executes, abort_reason preserved |
| T-HOOK-001-05 | `test_chain_empty_is_noop` | Empty chain | Context unchanged |
| T-HOOK-001-06 | `test_chain_single_middleware` | One middleware | Executes correctly |
| T-HOOK-001-07 | `test_chain_same_priority_stable_order` | 2 middleware at same priority | Insertion order preserved |
| T-HOOK-001-08 | `test_chain_register_middleware` | `register()` | Middleware added to chain |
| T-HOOK-001-09 | `test_chain_unregister_middleware` | `unregister()` by name | Middleware removed |

#### Task: T-HOOK-002 — ChainRunner (timeout + error handling)

```
FILE: crates/y-hooks/src/chain_runner.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-HOOK-002-01 | `test_runner_timeout_per_middleware` | Middleware exceeds 5s timeout | `MiddlewareError::Timeout` |
| T-HOOK-002-02 | `test_runner_continues_after_non_fatal_error` | Non-critical middleware error | Chain continues, error logged |
| T-HOOK-002-03 | `test_runner_aborts_on_critical_error` | Critical middleware panics | Chain aborted, error captured |
| T-HOOK-002-04 | `test_runner_tracing_spans` | Normal execution | Spans emitted with middleware_name, chain_type |

#### Task: T-HOOK-003 — HookRegistry

```
FILE: crates/y-hooks/src/hook_registry.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-HOOK-003-01 | `test_hook_register_handler` | Register handler for `PreToolExecute` | Handler discoverable |
| T-HOOK-003-02 | `test_hook_dispatch_to_matching_handlers` | Dispatch `PreToolExecute` event | Only matching handlers invoked |
| T-HOOK-003-03 | `test_hook_dispatch_no_match` | Dispatch event with no handlers | No error, no-op |
| T-HOOK-003-04 | `test_hook_handler_panic_does_not_propagate` | Handler panics | Hook dispatch continues, panic logged |
| T-HOOK-003-05 | `test_hook_multiple_handlers_same_point` | 3 handlers for same hook point | All 3 invoked |

#### Task: T-HOOK-004 — EventBus

```
FILE: crates/y-hooks/src/event_bus.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-HOOK-004-01 | `test_event_bus_subscribe_and_receive` | Subscribe, publish event | Subscriber receives event |
| T-HOOK-004-02 | `test_event_bus_multiple_subscribers` | 3 subscribers | All 3 receive same event |
| T-HOOK-004-03 | `test_event_bus_filter_by_type` | Subscriber filters on `ToolExecuted` | Only receives matching events |
| T-HOOK-004-04 | `test_event_bus_slow_subscriber_drops_oldest` | Slow subscriber, high event volume | Oldest events dropped, no backpressure |
| T-HOOK-004-05 | `test_event_bus_fire_and_forget` | Publish with no subscribers | No error |
| T-HOOK-004-06 | `test_event_bus_unsubscribe` | Unsubscribe handler | No longer receives events |
| T-HOOK-004-07 | `test_event_bus_custom_event` | Publish `Event::Custom` | Subscriber receives with payload |

### 4.2 Integration Tests

```
FILE: crates/y-hooks/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-HOOK-INT-01 | `chain_integration_test.rs` | `test_context_chain_full_pipeline` | BuildSystemPrompt → InjectMemory → InjectTools → LoadHistory |
| T-HOOK-INT-02 | `chain_integration_test.rs` | `test_tool_chain_with_guardrail` | Validation middleware → guardrail middleware → execution |
| T-HOOK-INT-03 | `chain_integration_test.rs` | `test_chain_abort_propagation` | Guardrail aborts → tool not executed → abort reason in response |
| T-HOOK-INT-04 | `event_integration_test.rs` | `test_event_bus_under_load` | 1000 events, 10 subscribers, verify delivery |
| T-HOOK-INT-05 | `hook_integration_test.rs` | `test_hook_and_event_combined` | Hook fires event, subscriber receives it |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-HOOK-001 | `MiddlewareChain` | Priority-sorted vec, register/unregister, execution pipeline | High |
| I-HOOK-002 | `ChainRunner` | Timeout-guarded per-middleware execution, tracing | High |
| I-HOOK-003 | `HookRegistry` | Handler registration, dispatch by hook point, panic isolation | High |
| I-HOOK-004 | `EventBus` | `tokio::broadcast`-based, subscriber management, filtering | High |
| I-HOOK-005 | `HookConfig` | Timeouts, capacities, default middleware registrations | Medium |
| I-HOOK-006 | `PluginLoader` skeleton | Dynamic middleware loading placeholder (full impl Phase 4) | Low |

---

## 6. Performance Benchmarks

```
FILE: crates/y-hooks/benches/chain_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| 10-middleware chain execution | P95 < 5ms | `criterion` |
| 50-middleware chain execution | P95 < 20ms | `criterion` |
| Event bus dispatch (1000 events) | P95 < 10ms | `criterion` |
| Hook dispatch (10 handlers) | P95 < 1ms | `criterion` |
| Middleware register/unregister | P95 < 100us | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-hooks` |
| Clippy clean | 0 warnings | `cargo clippy -p y-hooks` |
| No deadlock potential | Verified | Code review: no nested locks |

---

## 8. Acceptance Criteria

- [ ] Middleware chains execute in priority order
- [ ] ShortCircuit and abort correctly stop chain execution
- [ ] Hook handlers are dispatched to correct hook points
- [ ] Handler panics are caught and do not crash the system
- [ ] EventBus delivers to all subscribers, slow subscribers drop oldest
- [ ] 10-middleware chain completes in < 5ms P95
- [ ] Coverage >= 80%
