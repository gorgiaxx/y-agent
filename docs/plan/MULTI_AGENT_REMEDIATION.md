# Multi-Agent Module Remediation Plan

**Version**: v0.1
**Created**: 2026-03-10
**Status**: Draft
**Design References**: `multi-agent-design.md` (v0.4), `agent-autonomy-design.md` (v0.2), `micro-agent-pipeline-design.md` (v0.1)
**Current Module Plan**: `docs/plan/modules/y-multi-agent.md`

---

## 1. Audit Summary

The current `y-multi-agent` crate implements approximately **30-40%** of the design requirements. Completed work is primarily at the data-structure definition level (`AgentDefinition` TOML parsing, `AgentMode` four-mode enum, `ContextStrategy` four-strategy enum, basic `AgentPool`, basic `DelegationProtocol`, Sequential/Hierarchical patterns, `DynamicAgentDefinition` scaffolding). Runtime behavior, Orchestrator integration, security permission model, persistence, and Meta-tool system remain unimplemented.

---

## 2. Implementation Phases

Based on the design document's rollout stages and current progress, remediation is divided into **5 phases**, each independently deliverable and verifiable.

---

### Phase R1: Data Model Alignment & Security Foundation (Est. 2-3 days)

> **Goal**: Eliminate structural contradictions between implementation and design, complete security-critical data structures.

#### R1.1 Unify Trust Model

**Problem**: `trust.rs` defines `TrustTier` (Trusted/Verified/Untrusted), while `dynamic_agent.rs` separately defines `TrustLevel` (Dynamic/UserDefined/BuiltIn). The design requires a three-tier hierarchy: `BuiltIn > UserDefined > Dynamic`.

**Changes**:

##### [MODIFY] [trust.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/trust.rs)

- Rename `TrustTier` variants to `BuiltIn`, `UserDefined`, `Dynamic` to align with design
- Implement `PartialOrd`/`Ord` so that `BuiltIn > UserDefined > Dynamic`
- Update `can_manage_agents()` and `can_write()` logic accordingly
- Remove duplicate `TrustLevel` enum from `dynamic_agent.rs`

##### [MODIFY] [dynamic_agent.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/dynamic_agent.rs)

- Delete `TrustLevel` enum; use unified `TrustTier` instead
- Rename `trust_level` field to `trust_tier` in `DynamicAgentDefinition`

#### R1.2 Complete DynamicAgentDefinition Fields

**Problem**: Missing `id`, `source`, `delegation_depth`, `version`, `status`, `deactivated_at`, `deactivation_reason` fields required by design.

**Changes**:

##### [MODIFY] [dynamic_agent.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/dynamic_agent.rs)

- Add `AgentSource` enum (`BuiltIn`, `UserDefined`, `Dynamic { creator_agent_id }`)
- Add `AgentStatus` enum (`Active`, `Deactivated`)
- Complete `DynamicAgentDefinition` struct with:
  - `id: String` (unique ID)
  - `source: AgentSource`
  - `delegation_depth: u32`
  - `version: u64`
  - `status: AgentStatus` (replaces `active: bool`)
  - `deactivated_at: Option<String>`
  - `deactivation_reason: Option<String>`

#### R1.3 Implement EffectivePermissions

**Problem**: Design requires precise permission inheritance model; current implementation only has `permission_snapshot: Vec<String>`.

**Changes**:

##### [MODIFY] [dynamic_agent.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/dynamic_agent.rs)

- Add `EffectivePermissions` struct:
  ```rust
  struct EffectivePermissions {
      tools_allowed: Vec<String>,   // intersection with creator
      tools_denied: Vec<String>,    // union with creator
      max_iterations: u32,          // min(declared, creator)
      max_tool_calls: u32,          // min(declared, creator)
      max_tokens: u64,              // min(declared, creator)
      delegation_depth: u32,        // creator.depth - 1
  }
  ```
- Add `EffectivePermissions::compute(declared, creator_snapshot)` method
- Replace `permission_snapshot: Vec<String>` with `effective_permissions: EffectivePermissions`
- Update `validate_definition()` to use `EffectivePermissions` for permission checks

#### R1.4 Complete Validation Pipeline

**Problem**: Validation pipeline is missing the Safety Screening stage.

**Changes**:

##### [MODIFY] [dynamic_agent.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/dynamic_agent.rs)

- Add Safety Screening stage:
  - Detect dangerous tool combinations (e.g., `ShellExec` + no denied tools)
  - Detect system prompt injection patterns
- Add `ValidationError` variant: `SafetyViolation { reason }`
- Update `validate_definition()` to three stages: Schema → Permission → Safety

#### R1 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-MA-R1-01 | `TrustTier` ordering: `BuiltIn > UserDefined > Dynamic` | `trust.rs` |
| T-MA-R1-02 | `EffectivePermissions::compute` intersection/union/min logic | `dynamic_agent.rs` |
| T-MA-R1-03 | Reject creation when `delegation_depth` is 0 | `dynamic_agent.rs` |
| T-MA-R1-04 | Safety screening detects dangerous tool combinations | `dynamic_agent.rs` |
| T-MA-R1-05 | `AgentStatus::Deactivated` replaces `active: bool` | `dynamic_agent.rs` |
| T-MA-R1-06 | `AgentSource` enum serialization/deserialization | `dynamic_agent.rs` |

---

### Phase R2: AgentRegistry & Pool Separation (Est. 2-3 days)

> **Goal**: Separate definition registration (Registry) from runtime instance management (Pool), aligning with the design's two-layer architecture.

#### R2.1 Create AgentRegistry

**Problem**: Design requires `AgentRegistry` to unify management of static + built-in + dynamic definitions. Current `pool.rs` conflates registry and pool responsibilities.

**Changes**:

##### [NEW] [registry.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/registry.rs)

- `AgentRegistry` struct: unified management of `AgentDefinition` registrations
- Support three registration sources: `BuiltIn` (framework-shipped), `UserDefined` (TOML config), `Dynamic` (runtime-created)
- API: `register()`, `get()`, `list()`, `search()`, `unregister()`
- Built-in `tool-engineer` and `agent-architect` definitions (loaded from TOML strings)
- Tiered queries by `TrustTier`

#### R2.2 Refactor AgentPool as Runtime Instance Manager

**Changes**:

##### [MODIFY] [pool.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/pool.rs)

- Refactor to manage `AgentInstance` (runtime instances) instead of `AgentDefinition`
- Add `AgentInstance` struct with lifecycle state machine:
  ```
  Creating → Configuring → Running → Completed | Failed | Interrupted
  ```
- Implement concurrency control: `max_concurrent_agents`, `max_agents_per_delegation`
- Per-instance resource tracking: `iterations`, `tool_calls`, `tokens_used`
- Use `tokio::sync::Semaphore` for concurrency management

##### [MODIFY] [lib.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/lib.rs)

- Export `AgentRegistry`, update public API

#### R2 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-MA-R2-01 | Registry registers/queries all three definition types | `registry.rs` |
| T-MA-R2-02 | Registry ships built-in `tool-engineer` and `agent-architect` | `registry.rs` |
| T-MA-R2-03 | Registry `search()` filters by name/capabilities | `registry.rs` |
| T-MA-R2-04 | Pool rejects when `max_concurrent_agents` exceeded | `pool.rs` |
| T-MA-R2-05 | Pool instance lifecycle state machine transitions | `pool.rs` |
| T-MA-R2-06 | Pool per-instance resource tracking (iterations, tool_calls) | `pool.rs` |

---

### Phase R3: Mode Overlay & Context Injection (Est. 3-4 days)

> **Goal**: Implement actual mode overlay and context injection logic so that `ContextStrategy` enums are no longer empty shells.

#### R3.1 Implement Mode Configuration Overlay

**Changes**:

##### [NEW] [mode.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/mode.rs)

- `ModeOverlay` struct: filters tool lists based on `AgentMode`
  - `Build`: all allowed_tools available
  - `Plan`: read-only tools only (`FileRead`, `SearchCode`, etc.)
  - `Explore`: search + read tools only
  - `General`: all allowed_tools
- `apply_mode_overlay(definition, mode_override) → FilteredDefinition`
- Mode-specific system prompt prefix injection

#### R3.2 Implement Context Injection Logic

**Changes**:

##### [NEW] [context.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/context.rs)

- `ContextInjector` trait with four strategy implementations:
  - `NoneStrategy`: passes only the delegation prompt
  - `SummaryStrategy`: calls LLM to generate conversation summary (requires provider dependency)
  - `FilteredStrategy`: filters messages by role/recency/keyword
  - `FullStrategy`: forwards complete conversation history (truncated to `max_context_tokens`)
- `apply_context(strategy, conversation, max_tokens) → Vec<Message>`

#### R3 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-MA-R3-01 | `Plan` mode retains only read-only tools | `mode.rs` |
| T-MA-R3-02 | `Explore` mode retains only search + read tools | `mode.rs` |
| T-MA-R3-03 | Mode-specific system prompt correctly injected | `mode.rs` |
| T-MA-R3-04 | `NoneStrategy` returns only delegation prompt | `context.rs` |
| T-MA-R3-05 | `FilteredStrategy` filters by recency | `context.rs` |
| T-MA-R3-06 | `FullStrategy` truncates to max_tokens limit | `context.rs` |

---

### Phase R4: Persistence & Meta-Tools (Est. 3-4 days)

> **Goal**: Migrate DynamicAgentStore to SQLite persistence, expose agent_create/update/deactivate/search meta-tools.

#### R4.1 DynamicAgentStore SQLite Persistence

**Changes**:

##### [MODIFY] [dynamic_agent.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/dynamic_agent.rs)

- Refactor `DynamicAgentStore` into a trait
- Retain `InMemoryDynamicAgentStore` for testing

##### [NEW] In `y-storage` crate:

- `SqliteDynamicAgentStore` implementation
- Migration: `dynamic_agents` table (id, name, definition_json, trust_tier, delegation_depth, version, status, effective_permissions_json, created_by, created_at, updated_at, deactivated_at, deactivation_reason)

#### R4.2 Meta-Tool Definitions

**Changes**:

##### [NEW] [meta_tools.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/meta_tools.rs)

- `agent_create` tool: creates dynamic agent, passes three-stage validation
- `agent_update` tool: updates existing dynamic agent
- `agent_deactivate` tool: soft-deletes dynamic agent
- `agent_search` tool: searches agent definitions by name/role/capability/tags
- Each tool includes JSON Schema parameter definition + execution logic

#### R4 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-MA-R4-01 | SQLite store CRUD operations | `y-storage` |
| T-MA-R4-02 | SQLite store persists and reloads correctly | `y-storage` |
| T-MA-R4-03 | `agent_create` passes three-stage validation | `meta_tools.rs` |
| T-MA-R4-04 | `agent_create` rejects permission escalation | `meta_tools.rs` |
| T-MA-R4-05 | `agent_deactivate` soft-deletes with reason | `meta_tools.rs` |
| T-MA-R4-06 | `agent_search` filters by mode/trust_tier/status | `meta_tools.rs` |

---

### Phase R5: CapabilityGap Middleware & Orchestrator Integration (Est. 4-5 days)

> **Goal**: Implement Agent Gap detection, auto-resolution protocol, and AgentExecutor integration with the Orchestrator.

#### R5.1 CapabilityGapMiddleware (Agent Part)

**Changes**:

##### [NEW] [gap.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/gap.rs)

- `AgentGapType` enum: `AgentNotFound`, `CapabilityMismatch`, `ModeInappropriate`
- `AgentGapDetector`: detects three agent gap types
- Resolution protocol: spawn `agent-architect` → design definition → validate → register → resume original delegation
- Fallback: HITL interrupt on unresolvable gaps

#### R5.2 AgentExecutor

**Changes**:

##### [NEW] [executor.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/executor.rs)

- `AgentExecutor`: loads definition from `AgentRegistry` → creates session branch → applies mode overlay → injects context → runs agent loop → returns `TaskOutput`
- Integrates `DelegationProtocol` and `AgentPool`
- Implements `TaskExecutor` trait (from `y-core`/`y-agent-core`)

#### R5.3 Built-in `task` Tool

**Changes**:

##### [NEW] [task_tool.rs](file:///Users/gorgias/Projects/y-agent/crates/y-multi-agent/src/task_tool.rs)

- Built-in `task` tool: in-conversation agent delegation
- Parameters: `agent_name`, `mode` (optional), `prompt`, `context_strategy` (optional)
- Invokes `DelegationProtocol` + `AgentExecutor`, returns sub-agent output as tool result

#### R5 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-MA-R5-01 | `AgentNotFound` gap triggers `agent-architect` | `gap.rs` |
| T-MA-R5-02 | `ModeInappropriate` gap classified correctly | `gap.rs` |
| T-MA-R5-03 | Gap resolution failure falls back to HITL | `gap.rs` |
| T-MA-R5-04 | `AgentExecutor` full lifecycle | `executor.rs` |
| T-MA-R5-05 | `task` tool parameter parsing and delegation | `task_tool.rs` |
| T-MA-R5-06 | `task` tool nested invocation triggers depth check | `task_tool.rs` |

---

## 3. Deferred Items

The following items belong to the design's Phase 4 or require deep integration with other modules. Recommended for implementation after the core framework stabilizes:

| Item | Reason |
|------|--------|
| Peer-to-Peer pattern | Depends on Orchestrator typed channels full implementation |
| Micro-Agent Pipeline pattern | Separate design document; depends on Working Memory module |
| Streaming partial results | Depends on EventBus and Provider streaming API |
| Observability metrics integration | Depends on y-diagnostics module |
| Agent-to-agent streaming | Open Question; pending benchmarking |

---

## 4. Dependency Graph

```
Phase R1 (Data Model)
  ↓
Phase R2 (Registry + Pool Separation)
  ↓
Phase R3 (Mode + Context) ←→ Phase R4 (Persistence + Meta-Tools)  [parallelizable]
  ↓
Phase R5 (Gap MW + Executor + task tool)  ← depends on R2 + R3 + R4
```

---

## 5. Verification

### Automated Tests

After each phase completion:

```bash
# Unit tests
cargo test -p y-multi-agent

# Clippy
cargo clippy -p y-multi-agent -- -D warnings

# If y-storage is involved
cargo test -p y-storage

# Full workspace build
cargo build --workspace
```

### Integration Tests

After Phase R5 completion, add integration tests:

| Test ID | Scenario |
|---------|----------|
| T-MA-INT-04 | Full delegation lifecycle: Registry → Pool → Executor → Result |
| T-MA-INT-05 | AgentNotFound → agent-architect → auto-create → retry delegation |
| T-MA-INT-06 | Dynamic agent SQLite storage → restart recovery |

---

## 6. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| `pool.rs` refactor breaks existing consumers | Retain old API as deprecated wrapper; migrate incrementally |
| `TrustTier` rename affects other crates | Full-project search for usage; batch update in one change |
| Phase R5 depends on Orchestrator's `TaskExecutor` trait | Develop against mock trait first; integrate when y-agent-core stabilizes |
| SQLite migration conflicts with existing migration system | Follow existing `y-storage` sqlx migration patterns |

---

## 7. Acceptance Criteria

- [ ] All existing tests continue to pass (regression safety)
- [ ] All new tests per phase pass
- [ ] `cargo clippy -p y-multi-agent -- -D warnings` zero warnings
- [ ] Coverage >= 75%
- [ ] All Phase 1-3 features from design documents implemented
- [ ] `DynamicAgentStore` migrated to SQLite persistence
- [ ] `AgentRegistry` unifies management of all three agent definition types
- [ ] `EffectivePermissions` permission inheritance model correctly implemented
