# Y-Hooks Module Remediation Plan

**Module**: `crates/y-hooks`
**Design Doc**: `docs/design/hooks-plugin-design.md` (v0.7)
**Audit Date**: 2026-03-11
**Status**: Audit Complete — Remediation Required

---

## 1. Audit Summary

The `y-hooks` crate has Phase 1 core primitives largely implemented. However, significant gaps remain across Phase 2 (middleware integration patterns), Phase 3 (plugin loading), and Phase 4 (performance optimization). The `y-core::hook` trait definitions also require expansion to cover all design-specified event types and hook points.

### What Is Implemented (✅)

| Component | Status | Notes |
|-----------|--------|-------|
| `MiddlewareChain` | ✅ Complete | Priority-sorted, register/unregister, short-circuit, abort; 10 unit tests |
| `ChainRunner` | ✅ Complete | Per-middleware timeout, tracing spans, error propagation; 4 unit tests |
| `HookRegistry` | ✅ Complete | Handler registration, dispatch by hook point, panic isolation via `tokio::spawn`; 5 unit tests |
| `EventBus` | ✅ Complete | `tokio::broadcast`-based, fire-and-forget, slow subscriber drops oldest; 7 unit tests |
| `HookConfig` | ✅ Complete | Timeout, channel capacity, max subscribers; 2 unit tests |
| `HookError` | ✅ Complete | 6 error variants |
| `PluginLoader` | ✅ Skeleton | Interface only; `load()` returns error placeholder |
| Benchmarks | ✅ Partial | `hooks_bench.rs` covers middleware chain (10 MW) and event bus (1000 events) |

### What Is Missing (❌)

| Gap | Design Reference | Severity |
|-----|-----------------|----------|
| Missing `EventSubscriber` trait integration in `EventBus` | §High-Level Design → Event Bus | **High** |
| Missing `EventFilter` mechanism | §High-Level Design → Event Bus | **High** |
| Missing `ContextOverflow` hook point in `y-core::HookPoint` | §Hook Points table | Medium |
| Missing `PostSkillInjection` hook point | §Hook Points table | Medium |
| Missing 19 of 33 `Event` variants in `y-core::Event` | §Event Types table | **High** |
| Missing `Plugin` trait and `PluginRegistrar` in `y-core` | §Plugin API | Medium (Phase 3) |
| Missing `libloading`-based dynamic plugin loading | §Plugin Loading Flow | Medium (Phase 3) |
| Missing plugin configuration schema (TOML) | §Plugin Configuration | Medium (Phase 3) |
| Missing per-subscriber channel (design says channel-per-subscriber, impl uses broadcast) | §Alternatives → Event Bus | **High** |
| Missing `HookSystem` facade that unifies Registry + Chains + EventBus | N/A (operational convenience) | Low |
| Missing integration tests | R&D Plan §4.2 | Medium |
| Missing 3 additional benchmarks | R&D Plan §6 | Low |
| Missing middleware chain compilation optimization | §Performance → Optimization Strategies | Low (Phase 4) |
| Missing feature flags for rollback | §Rollout → Rollback Plan | Medium |
| Missing metrics emission (`hooks.dispatch.total`, etc.) | §Observability | Medium |

---

## 2. Remediation Phases

### Phase R1: Event System Completion (High Priority)

Expand the `Event` enum and wire `EventSubscriber` trait into `EventBus`.

#### [MODIFY] [hook.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/hook.rs)

**R1.1 — Add missing `HookPoint` variants:**

```diff
 pub enum HookPoint {
     ...
     DynamicAgentDeactivated,
+    ContextOverflow,
+    PostSkillInjection,
 }
```

**R1.2 — Add missing `Event` variants (19 of 33 missing):**

Per design §Event Types, the following event variants must be added:

| Category | Missing Events |
|----------|---------------|
| LLM | `LlmCallStarted`, `LlmCallFailed` |
| Tool | `ToolFailed` |
| Memory | `MemoryStored`, `MemoryRecalled` |
| Session | `SessionCreated`, `SessionClosed` |
| Compaction | `CompactionTriggered`, `CompactionFailed` |
| Context | `ContextOverflow`, `CanonicalSynced`, `SessionRepaired` |
| Agent | `AgentLoopIteration` |
| Pipeline | `PipelineStarted`, `PipelineStepCompleted`, `PipelineStepFailed`, `PipelineCompleted`, `WorkingMemorySlotWritten` |
| Autonomy | `ToolGapDetected`, `ToolGapResolved`, `AgentGapDetected`, `AgentGapResolved`, `DynamicToolRegistered`, `DynamicAgentRegistered`, `DynamicAgentDeactivated`, `WorkflowTemplateCreated` |

Each event should carry the key fields specified in the design document's Event Types table.

**R1.3 — Add `EventCategory` enum for filtering:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    Llm,
    Tool,
    Memory,
    Session,
    Compaction,
    Context,
    Orchestration,
    Agent,
    Pipeline,
    Autonomy,
}
```

Add a `fn category(&self) -> EventCategory` method to `Event`.

---

#### [MODIFY] [event_bus.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/event_bus.rs)

**R1.4 — Channel-per-subscriber architecture:**

The design explicitly chose channel-per-subscriber over `tokio::broadcast` for backpressure isolation (§Alternatives → Event Bus). The current implementation uses `tokio::broadcast`, which has shared backpressure.

Replace with `tokio::mpsc::Sender` per subscriber:

- `EventBus` holds `Vec<(usize, EventFilter, mpsc::Sender<Arc<Event>>)>` behind a `RwLock`
- `subscribe()` creates a new `mpsc::channel(capacity)` and returns the `Receiver` side
- `publish()` iterates subscribers, pre-filters, and `try_send()`; if full, drop + increment metric
- This matches the design: "Pre-filter before send; only matching events enter channel"

> [!WARNING]
> This is a breaking change to the `EventBus` API. All callers of `subscribe()` and `recv()` must be updated.

**R1.5 — Integrate `EventSubscriber` trait:**

Add `subscribe_handler()` method that accepts `Arc<dyn EventSubscriber>`, spawns a background task that consumes from the per-subscriber channel and calls `on_event()`.

**R1.6 — Add metrics:**

Add `event_bus.published`, `event_bus.delivered`, `event_bus.dropped` counters using `std::sync::atomic::AtomicU64`, accessible via `EventBus::metrics()`.

---

### Phase R2: Hook & Middleware Enhancements (Medium Priority)

#### [MODIFY] [hook_registry.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_registry.rs)

**R2.1 — Add `unregister()` method:**

The `HookRegistry` currently has no way to remove handlers. Add `unregister(handler_id)` to support plugin unloading.

**R2.2 — Add handler ordering:**

Hook handlers should be dispatched in registration order. Currently they are stored in a `Vec` which preserves insertion order — this is correct, but should be documented and tested.

**R2.3 — Add hook dispatch metrics:**

Add `hooks.dispatch.total`, `hooks.dispatch.duration_us`, `hooks.dispatch.errors` counters per `HookPoint`, accessible via `HookRegistry::metrics()`.

---

#### [MODIFY] [chain.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/chain.rs)

**R2.4 — Add middleware inspection API:**

Add `get_middleware(name: &str) -> Option<&Arc<dyn Middleware>>` for diagnostic purposes.

**R2.5 — Add chain metrics:**

Add `middleware.chain.duration_us`, `middleware.chain.short_circuits`, `middleware.chain.errors` counters per `ChainType`.

---

#### [NEW] [hook_system.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_system.rs)

**R2.6 — `HookSystem` facade:**

Unified facade that holds all 5 `MiddlewareChain`s, the `HookRegistry`, the `EventBus`, and the `ChainRunner`. Provides methods like:

- `fn context_chain(&self) -> &MiddlewareChain`
- `fn tool_chain(&self) -> &MiddlewareChain`
- `fn llm_chain(&self) -> &MiddlewareChain`
- `fn compaction_chain(&self) -> &MiddlewareChain`
- `fn memory_chain(&self) -> &MiddlewareChain`
- `fn hooks(&self) -> &HookRegistry`
- `fn events(&self) -> &EventBus`
- `fn execute_chain(&self, chain_type: ChainType, ctx: &mut MiddlewareContext) -> Result<()>`
- `fn dispatch_hook(&self, data: &HookData)`
- `fn publish_event(&self, event: Event)`

This is the primary entry point for other modules to interact with the hook system.

---

### Phase R3: Plugin System (Medium-Low Priority, Design Phase 3)

#### [MODIFY] [hook.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/hook.rs)

**R3.1 — Add `Plugin` trait:**

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn on_load(&self, registrar: &mut PluginRegistrar) -> Result<()>;
    fn on_unload(&self) -> Result<()>;
}
```

**R3.2 — Add `PluginRegistrar` struct:**

```rust
pub struct PluginRegistrar {
    hooks: Vec<(HookPoint, Arc<dyn HookHandler>)>,
    middleware: Vec<(ChainType, u32, Arc<dyn Middleware>)>,
    event_subscriptions: Vec<(EventFilter, Arc<dyn EventSubscriber>)>,
}
```

Methods: `register_hook()`, `register_middleware()`, `subscribe_events()`.

---

#### [MODIFY] [plugin.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/plugin.rs)

**R3.3 — Implement `PluginLoader` with `libloading`:**

- Add `libloading` dependency to `Cargo.toml`
- Implement `load_plugin(path: &Path)` that:
  1. Opens shared library via `libloading::Library::new()`
  2. Looks up `create_plugin` symbol
  3. Calls symbol to get `Box<dyn Plugin>`
  4. Calls `on_load()` with a `PluginRegistrar`
  5. Applies registrations to `HookSystem`
- Implement ABI version check before `create_plugin`
- Implement `unload_plugin(name: &str)` that calls `on_unload()` and removes registrations

**R3.4 — Plugin configuration:**

Add TOML config schema per design §Plugin Configuration:

```rust
#[derive(Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub path: String,
    pub config: HashMap<String, serde_json::Value>,
    pub priority: i32,
}
```

Add plugin path validation (absolute paths only, allowed directories).

---

### Phase R4: Performance & Feature Flags (Low Priority, Design Phase 4)

#### [MODIFY] [chain_runner.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/chain_runner.rs)

**R4.1 — Middleware chain compilation:**

Per design §Performance → Optimization Strategies: "At registration time, the chain is compiled into a single nested async function to avoid dynamic dispatch overhead during hot-path execution."

This is an optimization that can be deferred. When implemented, `MiddlewareChain::compile()` would produce a `CompiledChain` that avoids per-middleware Arc dereference and vtable dispatch.

---

#### [MODIFY] [Cargo.toml](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/Cargo.toml)

**R4.2 — Feature flags for rollback:**

Per design §Rollout → Rollback Plan:

```toml
[features]
default = ["hooks_enabled", "event_bus", "llm_middleware", "tool_middleware", "context_middleware"]
hooks_enabled = []
event_bus = []
llm_middleware = []
tool_middleware = []
context_middleware = []
```

Wrap hook dispatch, event publishing, and chain execution in `#[cfg(feature = "...")]` guards.

---

#### [NEW] Additional benchmarks

Per R&D Plan §6, add missing benchmarks:

| Benchmark | Target |
|-----------|--------|
| 50-middleware chain execution | P95 < 20ms |
| Hook dispatch (10 handlers) | P95 < 1ms |
| Middleware register/unregister | P95 < 100µs |

---

### Phase R5: Integration Tests (Medium Priority)

#### [NEW] `crates/y-hooks/tests/chain_integration_test.rs`

Per R&D Plan §4.2:

| Test | Scenario |
|------|----------|
| `test_context_chain_full_pipeline` | Simulate 4 context middleware in priority order |
| `test_tool_chain_with_guardrail` | Validation → guardrail → execution with short-circuit |
| `test_chain_abort_propagation` | Guardrail aborts → downstream skipped → reason preserved |

#### [NEW] `crates/y-hooks/tests/event_integration_test.rs`

| Test | Scenario |
|------|----------|
| `test_event_bus_under_load` | 1000 events, 10 subscribers, verify delivery completeness |

#### [NEW] `crates/y-hooks/tests/hook_integration_test.rs`

| Test | Scenario |
|------|----------|
| `test_hook_and_event_combined` | Hook handler publishes event to EventBus, subscriber receives it |

---

## 3. Priority and Ordering

| Phase | Priority | Estimated Effort | Dependencies |
|-------|----------|-----------------|--------------|
| **R1** Event System Completion | **High** | 2-3 days | None |
| **R2** Hook & Middleware Enhancements | **Medium** | 1-2 days | R1 (for EventCategory) |
| **R3** Plugin System | **Medium-Low** | 3-4 days | R2 (for HookSystem) |
| **R4** Performance & Feature Flags | **Low** | 1-2 days | R1-R3 |
| **R5** Integration Tests | **Medium** | 1-2 days | R1-R2 |

**Recommended order**: R1 → R5 → R2 → R3 → R4

---

## 4. Verification Plan

### Automated Tests

```bash
# Run all y-hooks unit tests
cargo test -p y-hooks

# Run all y-hooks integration tests
cargo test -p y-hooks --test '*'

# Run benchmarks
cargo bench -p y-hooks

# Clippy check
cargo clippy -p y-hooks -- -W warnings

# Check compilation of the entire workspace (for y-core changes)
cargo check --workspace
```

### Quality Gates

| Gate | Target |
|------|--------|
| All unit tests pass | `cargo test -p y-hooks` — 0 failures |
| All integration tests pass | `cargo test -p y-hooks --test '*'` — 0 failures |
| Clippy clean | 0 warnings |
| 10-middleware chain benchmark | P95 < 5ms |
| Event bus 1000 events benchmark | P95 < 10ms |
