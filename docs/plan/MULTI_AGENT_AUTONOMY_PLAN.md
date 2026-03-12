# Multi-Agent & Agent Autonomy R&D Plan

**Version**: v0.1
**Created**: 2026-03-11
**Status**: Draft
**Design References**: [`multi-agent-design.md`](../design/multi-agent-design.md) (v0.5), [`agent-autonomy-design.md`](../design/agent-autonomy-design.md) (v0.2), [`AGENT_AUTONOMY.md`](../standards/AGENT_AUTONOMY.md) (v0.1), [`micro-agent-pipeline-design.md`](../design/micro-agent-pipeline-design.md) (v0.1)
**Supersedes**: [`MULTI_AGENT_REMEDIATION.md`](MULTI_AGENT_REMEDIATION.md) (absorbed into this plan), [`modules/y-agent.md`](modules/y-agent.md) (absorbed into this plan)

---

## 1. Overview

This document is the detailed R&D plan for the **Multi-Agent Framework** and **Agent Autonomy** systems. It covers the complete implementation path from current state (~30-40% scaffolding) to full design compliance, organized into 8 phases with strict TDD methodology.

### Scope Coverage

| Design Document | Coverage in This Plan |
|----------------|----------------------|
| `multi-agent-design.md` | Agent definitions, registry, pool, 4 collaboration patterns, delegation protocol, context strategies, agent modes, AgentExecutor, task tool, observability |
| `agent-autonomy-design.md` | Dynamic tool lifecycle, dynamic agent lifecycle, capability-gap resolution, self-orchestration protocol, parameterized scheduling, meta-tools |
| `AGENT_AUTONOMY.md` | Unified agent invocation principle, `AgentDelegator` trait, cross-module invocation, built-in system agents, trust hierarchy, permission inheritance, validation pipeline, prompt management, migration of anti-patterns (including `generate_title` in y-session) |

### Current State Assessment

| Component | Status | Notes |
|-----------|--------|-------|
| `AgentDefinition` TOML parsing | ✅ Scaffolded | `definition.rs` (9.5KB) — needs schema alignment |
| `AgentMode` enum (Build/Plan/Explore/General) | ✅ Scaffolded | `mode.rs` (10.5KB) — mode overlay logic present |
| `ContextStrategy` enum (None/Summary/Filtered/Full) | ✅ Scaffolded | `context.rs` (11KB) — injection logic present |
| `AgentPool` | ✅ Scaffolded | `pool.rs` (12.5KB) — needs instance lifecycle refactor |
| `AgentRegistry` | ✅ Scaffolded | `registry.rs` (12.6KB) — needs built-in agent registration |
| `DelegationProtocol` | ✅ Scaffolded | `delegation.rs` (8.7KB) — needs depth tracking |
| `TrustTier` | ✅ Scaffolded | `trust.rs` (2.8KB) — needs `BuiltIn > UserDefined > Dynamic` alignment |
| `DynamicAgentDefinition` | ✅ Scaffolded | `dynamic_agent.rs` (25KB) — needs field completion |
| `CapabilityGap` detection | ✅ Scaffolded | `gap.rs` (10KB) — needs resolution protocol |
| `AgentExecutor` | ✅ Scaffolded | `executor.rs` (8.2KB) — needs Orchestrator integration |
| `Meta-tools` | ✅ Scaffolded | `meta_tools.rs` (15KB) — needs tool registration |
| `task` tool | ✅ Scaffolded | `task_tool.rs` (8.4KB) — needs delegation depth check |
| Sequential pattern | ✅ Scaffolded | `patterns/sequential.rs` (2.1KB) |
| Hierarchical pattern | ✅ Scaffolded | `patterns/hierarchical.rs` (3.1KB) |
| `AgentDelegator` trait in `y-core` | ❌ Missing | Required by AGENT_AUTONOMY.md §2.4 |
| Built-in agent TOML files | ❌ Missing | `config/agents/` directory does not exist |
| P2P pattern | ❌ Missing | Design Phase 4 |
| Micro-Agent Pipeline | ❌ Missing | Separate design doc |
| SQLite persistence for DynamicAgentStore | ❌ Missing | Requires `y-storage` integration |
| `CapabilityGapMiddleware` | ❌ Missing | Requires `y-hooks` integration |
| Observability metrics | ❌ Missing | Requires `y-diagnostics` integration |

---

## 2. Dependency Map

```
y-agent
  ├── y-core (AgentDelegator trait, ContextStrategyHint, DelegationOutput, DelegationError)
  ├── y-agent (orchestrator) (Orchestrator, TaskExecutor trait, TaskType::SubAgent)
  ├── y-session (child/ephemeral sessions for context isolation)
  ├── y-hooks (CapabilityGapMiddleware hook point, event bus)
  ├── y-provider (ProviderPool — indirect via AgentDelegator, for SummaryStrategy)
  ├── y-storage (SqliteDynamicAgentStore persistence)
  ├── y-tools (Tool Registry for meta-tool registration, tool_search)
  ├── y-guardrails (Permission Model for HITL escalation, risk scoring)
  ├── toml (agent definition parsing)
  ├── tokio (async, semaphore for concurrency, task spawning)
  ├── serde / serde_json (definitions, delegation messages)
  ├── thiserror (errors)
  └── tracing (agent_id, delegation, collaboration_pattern spans)
```

---

## 3. Module Structure (Target)

```
y-agent/src/
  lib.rs                — Public API exports
  error.rs              — MultiAgentError
  config.rs             — MultiAgentConfig (pool limits, trust defaults)
  definition.rs         — AgentDefinition: TOML parsing, schema validation
  pool.rs               — AgentPool: instance lifecycle, concurrency (Semaphore)
  registry.rs           — AgentRegistry: unified BuiltIn/UserDefined/Dynamic management
  delegation.rs         — DelegationProtocol: task delegation, depth tracking, result collection
  mode.rs               — AgentMode overlay: tool filtering, prompt injection
  context.rs            — ContextInjector: None/Summary/Filtered/Full strategies
  trust.rs              — TrustTier: BuiltIn > UserDefined > Dynamic
  dynamic_agent.rs      — DynamicAgentDefinition, EffectivePermissions, validation pipeline
  executor.rs           — AgentExecutor: full delegation lifecycle
  task_tool.rs          — Built-in `task` tool for in-conversation delegation
  meta_tools.rs         — agent_create/update/deactivate/search meta-tools
  gap.rs                — CapabilityGap detection (agent gaps) and resolution protocol
  patterns/
    mod.rs              — Pattern selection + PatternExecutor trait
    sequential.rs       — Sequential Pipeline pattern
    hierarchical.rs     — Hierarchical Delegation pattern
    peer_to_peer.rs     — [NEW] Peer-to-Peer pattern (Phase 7)
    micro_pipeline.rs   — [NEW] Micro-Agent Pipeline adapter (Phase 7)

y-core/src/
  agent.rs              — [NEW] AgentDelegator trait, ContextStrategyHint, DelegationOutput

config/agents/          — [NEW] Built-in agent TOML definitions
  compaction-summarizer.toml
  context-summarizer.toml
  title-generator.toml
  task-intent-analyzer.toml
  pattern-extractor.toml
  capability-assessor.toml
  tool-engineer.toml
  agent-architect.toml
```

---

## 4. Implementation Phases

### Phase 1: Foundation Types & Trust Model (Est. 2 days)

> **Goal**: Align core data types with design, establish trust hierarchy and permission inheritance model.

#### 1.1 Unify TrustTier

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-01 | Rename `TrustTier` variants to `BuiltIn`, `UserDefined`, `Dynamic`; implement `PartialOrd`/`Ord` | `trust.rs` |
| I-MA-02 | Remove duplicate `TrustLevel` enum from `dynamic_agent.rs`; use unified `TrustTier` | `dynamic_agent.rs` |

#### 1.2 Complete DynamicAgentDefinition

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-03 | Add `AgentSource` enum (`BuiltIn`, `UserDefined`, `Dynamic { creator_agent_id }`) | `dynamic_agent.rs` |
| I-MA-04 | Add `AgentStatus` enum (`Active`, `Deactivated`) replacing `active: bool` | `dynamic_agent.rs` |
| I-MA-05 | Add missing fields: `id`, `source`, `delegation_depth`, `version`, `deactivated_at`, `deactivation_reason` | `dynamic_agent.rs` |

#### 1.3 Implement EffectivePermissions

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-06 | Define `EffectivePermissions` struct with compute method (intersection/union/min) | `dynamic_agent.rs` |
| I-MA-07 | Replace `permission_snapshot: Vec<String>` with `effective_permissions: EffectivePermissions` | `dynamic_agent.rs` |

#### 1.4 Complete Validation Pipeline

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-08 | Add Safety Screening stage (dangerous tool combos, prompt injection detection) | `dynamic_agent.rs` |
| I-MA-09 | Update `validate_definition()` to three stages: Schema → Permission → Safety | `dynamic_agent.rs` |

#### Phase 1 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P1-01 | `test_trust_tier_ordering` | Compare `BuiltIn`, `UserDefined`, `Dynamic` | `BuiltIn > UserDefined > Dynamic` |
| T-MA-P1-02 | `test_trust_tier_serde_roundtrip` | Serialize/deserialize `TrustTier` | Identity |
| T-MA-P1-03 | `test_effective_permissions_tools_intersection` | Creator allows `[a, b, c]`, child declares `[b, c, d]` | Effective = `[b, c]` |
| T-MA-P1-04 | `test_effective_permissions_denied_union` | Creator denies `[x]`, child denies `[y]` | Effective denied = `[x, y]` |
| T-MA-P1-05 | `test_effective_permissions_limits_min` | Creator `max_iter=10`, child declares `15` | Effective = `10` |
| T-MA-P1-06 | `test_delegation_depth_decrement` | Creator depth `2` | Child depth = `1` |
| T-MA-P1-07 | `test_reject_creation_at_depth_zero` | Creator depth `0` | `DelegationDepthExhausted` error |
| T-MA-P1-08 | `test_validation_schema_stage` | Invalid TOML, missing required fields | Schema validation error |
| T-MA-P1-09 | `test_validation_permission_stage` | Tool not in creator's allowlist | Permission violation error |
| T-MA-P1-10 | `test_validation_safety_stage` | `shell_exec` + no denied tools | Safety violation warning |
| T-MA-P1-11 | `test_agent_status_replaces_bool` | `AgentStatus::Deactivated` | Backward-compatible behavior |
| T-MA-P1-12 | `test_agent_source_serde_roundtrip` | `AgentSource::Dynamic { creator_agent_id }` | Roundtrip identity |

---

### Phase 2: AgentDelegator Trait & Cross-Module Invocation (Est. 2 days)

> **Goal**: Define the `AgentDelegator` trait in `y-core` that enables all modules to request agent delegation without depending on `y-agent`. This is the **foundational requirement** of the Agent Autonomy Standard (AGENT_AUTONOMY.md §2.4).

#### 2.1 Define AgentDelegator in y-core

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-10 | Define `AgentDelegator` trait: `async fn delegate(&self, agent_name, input, context_strategy)` | `y-core/src/agent.rs` [NEW] |
| I-MA-11 | Define `ContextStrategyHint` enum: `None`, `Summary`, `Filtered`, `Full` | `y-core/src/agent.rs` |
| I-MA-12 | Define `DelegationOutput` struct: `text`, `tokens_used`, `duration_ms` | `y-core/src/agent.rs` |
| I-MA-13 | Define `DelegationError` enum | `y-core/src/agent.rs` |
| I-MA-14 | Export from `y-core/src/lib.rs` | `y-core/src/lib.rs` |

#### 2.2 Implement AgentDelegator for AgentPool

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-15 | Implement `AgentDelegator` for `AgentPool`: agent lookup, prompt construction, instance creation, delegation | `pool.rs` or `delegation.rs` |
| I-MA-16 | Add `Arc<dyn AgentDelegator>` dependency injection pattern documentation | `pool.rs` |

#### Phase 2 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P2-01 | `test_agent_delegator_trait_object_safe` | Create `Arc<dyn AgentDelegator>` | Compiles, no errors |
| T-MA-P2-02 | `test_context_strategy_hint_serde` | Roundtrip `ContextStrategyHint` | Identity |
| T-MA-P2-03 | `test_delegation_output_fields` | Create `DelegationOutput` with all fields | Fields accessible |
| T-MA-P2-04 | `test_delegation_error_variants` | All error variants constructible | Correct `Display` output |
| T-MA-P2-05 | `test_agent_pool_implements_delegator` | `AgentPool` as `dyn AgentDelegator` | Delegates successfully (mock agent) |
| T-MA-P2-06 | `test_delegation_unknown_agent` | Delegate to non-existent agent | `DelegationError::AgentNotFound` |
| T-MA-P2-07 | `test_delegation_structured_input` | Pass `serde_json::Value` input | Agent receives formatted input |

---

### Phase 3: Registry & Pool Separation (Est. 2-3 days)

> **Goal**: Separate agent definition management (Registry) from runtime instance lifecycle (Pool). Align with the design's two-layer architecture.

#### 3.1 Refactor AgentRegistry

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-17 | Ensure `AgentRegistry` unifies three sources: `BuiltIn`, `UserDefined`, `Dynamic` | `registry.rs` |
| I-MA-18 | Add `register_builtin()`, `register_user_defined()`, `register_dynamic()` methods | `registry.rs` |
| I-MA-19 | Add `search()` method: filter by name, mode, trust tier, status, capabilities/tags | `registry.rs` |
| I-MA-20 | Add tiered query support: results tagged by source tier | `registry.rs` |

#### 3.2 Refactor AgentPool as Instance Manager

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-21 | Ensure `AgentInstance` lifecycle state machine: `Creating → Configuring → Running → Completed | Failed | Interrupted` | `pool.rs` |
| I-MA-22 | Implement concurrency control via `tokio::sync::Semaphore`: `max_concurrent_agents`, `max_agents_per_delegation` | `pool.rs` |
| I-MA-23 | Add per-instance resource tracking: `iterations`, `tool_calls`, `tokens_used`, `start_time` | `pool.rs` |
| I-MA-24 | Add delegation depth counter: decrement per level, reject at 0 | `pool.rs` |

#### Phase 3 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P3-01 | `test_registry_three_sources` | Register BuiltIn, UserDefined, Dynamic | All retrievable |
| T-MA-P3-02 | `test_registry_search_by_name` | Search partial name match | Correct results |
| T-MA-P3-03 | `test_registry_search_by_mode` | Filter by `plan` mode | Only plan-mode agents |
| T-MA-P3-04 | `test_registry_search_by_trust_tier` | Filter by `Dynamic` | Only dynamic agents |
| T-MA-P3-05 | `test_registry_tiered_results` | List all | Results tagged with source tier |
| T-MA-P3-06 | `test_pool_concurrency_limit` | Exceed `max_concurrent_agents` | Blocks or rejects |
| T-MA-P3-07 | `test_pool_per_delegation_limit` | Exceed `max_agents_per_delegation` | Rejects |
| T-MA-P3-08 | `test_pool_instance_lifecycle` | Create → Configure → Run → Complete | State transitions correct |
| T-MA-P3-09 | `test_pool_instance_resource_tracking` | Track iterations, tool_calls | Counters increment |
| T-MA-P3-10 | `test_pool_delegation_depth_rejection` | Depth 0 | `DelegationDepthExhausted` |

---

### Phase 4: Mode Overlay, Context Injection & Built-in Agents (Est. 3-4 days)

> **Goal**: Implement mode overlays with real tool filtering, context injection with strategy implementations, and register built-in system agents.

#### 4.1 Mode Overlay

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-25 | Ensure `apply_mode_overlay()` filters tool lists per mode: `build` = all, `plan` = read-only, `explore` = search+read, `general` = all | `mode.rs` |
| I-MA-26 | Implement mode-specific system prompt prefix injection | `mode.rs` |
| I-MA-27 | Support mode override per delegation (delegator can override agent's default mode) | `mode.rs` |

#### 4.2 Context Injection

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-28 | Ensure `NoneStrategy`: passes only delegation prompt | `context.rs` |
| I-MA-29 | Implement `SummaryStrategy`: delegate to `context-summarizer` built-in agent via `AgentDelegator` | `context.rs` |
| I-MA-30 | Implement `FilteredStrategy`: filter by role, recency, keyword | `context.rs` |
| I-MA-31 | Implement `FullStrategy`: forward complete history, truncate to `max_context_tokens` | `context.rs` |

#### 4.3 Built-in Agent TOML Definitions

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-32 | Create `config/agents/` directory | `config/agents/` [NEW] |
| I-MA-33 | Define `compaction-summarizer.toml` (explore, no tools, fast/cheap model) | `config/agents/compaction-summarizer.toml` [NEW] |
| I-MA-34 | Define `context-summarizer.toml` (explore, no tools, fast/cheap model) | `config/agents/context-summarizer.toml` [NEW] |
| I-MA-34a | Define `title-generator.toml` (explore, no tools, fast/cheap model) | `config/agents/title-generator.toml` [NEW] |
| I-MA-35 | Define `task-intent-analyzer.toml` (plan, no tools, balanced model) | `config/agents/task-intent-analyzer.toml` [NEW] |
| I-MA-36 | Define `pattern-extractor.toml` (plan, file_read, high-capability) | `config/agents/pattern-extractor.toml` [NEW] |
| I-MA-37 | Define `capability-assessor.toml` (plan, tool_search+agent_search, balanced) | `config/agents/capability-assessor.toml` [NEW] |
| I-MA-38 | Define `tool-engineer.toml` (build, full tools, high-capability) | `config/agents/tool-engineer.toml` [NEW] |
| I-MA-39 | Define `agent-architect.toml` (plan, agent meta-tools only, high-capability) | `config/agents/agent-architect.toml` [NEW] |
| I-MA-40 | Register all built-in agents from TOML files in `AgentRegistry` at startup | `registry.rs` |

#### Phase 4 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P4-01 | `test_plan_mode_retains_readonly` | `plan` mode overlay | Only `file_read`, `search_code` etc. retained |
| T-MA-P4-02 | `test_explore_mode_retains_search_read` | `explore` mode overlay | Only search + read tools |
| T-MA-P4-03 | `test_build_mode_all_allowed` | `build` mode overlay | All allowed tools pass through |
| T-MA-P4-04 | `test_mode_prompt_injection` | Mode overlay applies | System prompt prefix present |
| T-MA-P4-05 | `test_mode_override_per_delegation` | Delegator overrides to `plan` | Agent's tools filtered to read-only |
| T-MA-P4-06 | `test_none_strategy_delegation_only` | `NoneStrategy` | Only delegation prompt returned |
| T-MA-P4-07 | `test_filtered_strategy_by_recency` | Last 5 messages | Correct subset returned |
| T-MA-P4-08 | `test_filtered_strategy_by_role` | Filter `user` role only | Only user messages |
| T-MA-P4-09 | `test_full_strategy_truncation` | 10K tokens input, 4K limit | Truncated to limit |
| T-MA-P4-10 | `test_builtin_agent_compaction_summarizer` | Parse `compaction-summarizer.toml` | Valid definition, explore mode, no tools |
| T-MA-P4-11 | `test_builtin_agent_tool_engineer` | Parse `tool-engineer.toml` | Valid definition, build mode, 6 tools |
| T-MA-P4-12 | `test_builtin_agent_agent_architect` | Parse `agent-architect.toml` | Valid definition, plan mode, `shell_exec` denied |
| T-MA-P4-13 | `test_registry_loads_all_builtin_agents` | Startup registration | 8 built-in agents registered |

---

### Phase 5: Meta-Tools & DynamicAgentStore Persistence (Est. 3-4 days)

> **Goal**: Expose full agent lifecycle meta-tools, migrate DynamicAgentStore to SQLite persistence.

#### 5.1 Meta-Tool Implementation

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-41 | Implement `agent_create` tool: three-stage validation + registration + persistence | `meta_tools.rs` |
| I-MA-42 | Implement `agent_update` tool: re-validation + partial update + versioning | `meta_tools.rs` |
| I-MA-43 | Implement `agent_deactivate` tool: soft-delete with reason + preserve experience | `meta_tools.rs` |
| I-MA-44 | Implement `agent_search` tool: search by name/role/mode/trust_tier/status/tags | `meta_tools.rs` |
| I-MA-45 | Register all four meta-tools with JSON Schema parameter definitions in `y-tools` ToolRegistry | `meta_tools.rs` |

#### 5.2 SQLite Persistence

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-46 | Refactor `DynamicAgentStore` into trait (for testing with `InMemoryDynamicAgentStore`) | `dynamic_agent.rs` |
| I-MA-47 | Implement `SqliteDynamicAgentStore` | `y-storage` crate [NEW] |
| I-MA-48 | Create migration: `dynamic_agents` table (id, name, definition_json, trust_tier, delegation_depth, version, status, effective_permissions_json, created_by, created_at, updated_at, deactivated_at, deactivation_reason) | `migrations/` [NEW] |
| I-MA-49 | Wire `SqliteDynamicAgentStore` into startup in `y-cli` | `y-cli` |

#### Phase 5 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P5-01 | `test_agent_create_valid` | Create with valid definition | Registered, persisted |
| T-MA-P5-02 | `test_agent_create_permission_escalation` | Child requests tool not in creator's list | Rejected with violation details |
| T-MA-P5-03 | `test_agent_create_safety_violation` | `shell_exec` + no denied tools | Safety screening flags |
| T-MA-P5-04 | `test_agent_update_version_increment` | Update existing agent | Version incremented |
| T-MA-P5-05 | `test_agent_update_re_validates` | Update with invalid change | Re-validation rejects |
| T-MA-P5-06 | `test_agent_deactivate_soft_delete` | Deactivate agent | Status = `Deactivated`, `deactivated_at` set, not hard-deleted |
| T-MA-P5-07 | `test_agent_deactivate_preserves_experience` | Deactivate agent | Experience records retained |
| T-MA-P5-08 | `test_agent_search_by_mode` | Search `plan` mode agents | Correct results |
| T-MA-P5-09 | `test_agent_search_by_trust_tier` | Search `Dynamic` tier | Only dynamic agents |
| T-MA-P5-10 | `test_sqlite_store_crud` | Create, read, update, delete | All operations succeed |
| T-MA-P5-11 | `test_sqlite_store_persist_reload` | Persist → restart → reload | Data intact |
| T-MA-P5-12 | `test_sqlite_store_version_tracking` | Multiple updates | Version counter correct |

---

### Phase 6: AgentExecutor, Task Tool & Orchestrator Integration (Est. 4-5 days)

> **Goal**: Complete the agent execution pipeline: AgentExecutor integrates with Orchestrator, task tool enables in-conversation delegation, delegation depth is enforced end-to-end.

#### 6.1 AgentExecutor

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-50 | Implement full lifecycle: load AgentDefinition → create session branch → apply mode overlay → inject context → run agent loop → return TaskOutput | `executor.rs` |
| I-MA-51 | Implement `TaskExecutor` trait (from `y-agent (orchestrator)`) for AgentExecutor | `executor.rs` |
| I-MA-52 | Wire into Orchestrator's `TaskType::SubAgent(AgentDelegation)` dispatch | `executor.rs` + `y-agent (orchestrator)` |
| I-MA-53 | Implement HITL interrupt/resume for agent instances | `executor.rs` |

#### 6.2 Task Tool

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-54 | Implement built-in `task` tool: parameters (`agent_name`, `mode`, `prompt`, `context_strategy`) | `task_tool.rs` |
| I-MA-55 | Invoke `DelegationProtocol` + `AgentExecutor`, return sub-agent output as tool result | `task_tool.rs` |
| I-MA-56 | Enforce delegation depth check: nested `task` calls decrement depth | `task_tool.rs` |
| I-MA-57 | Register `task` tool in `y-tools` ToolRegistry at startup | `task_tool.rs` + `y-cli` |

#### Phase 6 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P6-01 | `test_executor_full_lifecycle` | Load → configure → run → complete | Result returned correctly |
| T-MA-P6-02 | `test_executor_mode_overlay_applied` | Execute in `plan` mode | Write tools unavailable |
| T-MA-P6-03 | `test_executor_context_injection` | Execute with `summary` strategy | Summarized context provided |
| T-MA-P6-04 | `test_executor_session_branch` | Execute agent | Isolated session branch created |
| T-MA-P6-05 | `test_executor_resource_limits` | Exceed `max_iterations` | `IterationLimitExceeded` error |
| T-MA-P6-06 | `test_executor_timeout` | Exceed `timeout` | Timeout error |
| T-MA-P6-07 | `test_task_tool_basic_delegation` | `task(agent="researcher", prompt="...")` | Sub-agent result returned |
| T-MA-P6-08 | `test_task_tool_mode_override` | `task(agent="...", mode="plan")` | Agent runs in plan mode |
| T-MA-P6-09 | `test_task_tool_depth_check` | Nested delegation exceeds depth | Rejected |
| T-MA-P6-10 | `test_task_tool_unknown_agent` | Non-existent agent name | `AgentNotFound` error |

---

### Phase 7: Capability-Gap Resolution & Collaboration Patterns (Est. 4-5 days)

> **Goal**: Implement unified CapabilityGapMiddleware for agent gaps, complete remaining collaboration patterns.

#### 7.1 Agent Gap Detection & Resolution

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-58 | Implement `AgentGapDetector`: detect `AgentNotFound`, `CapabilityMismatch`, `ModeInappropriate` | `gap.rs` |
| I-MA-59 | Implement resolution protocol: spawn `agent-architect` → design → validate → register → resume | `gap.rs` |
| I-MA-60 | Implement HITL fallback for unresolvable gaps | `gap.rs` |
| I-MA-61 | Integrate as middleware component in `y-hooks` CapabilityGapMiddleware chain | `gap.rs` + `y-hooks` |

#### 7.2 Peer-to-Peer Pattern (Design Phase 4)

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-62 | Implement P2P pattern: agents communicate through shared typed channels | `patterns/peer_to_peer.rs` [NEW] |
| I-MA-63 | Create shared channel abstraction using Orchestrator's channel model | `patterns/peer_to_peer.rs` |
| I-MA-64 | Implement channel reducers for aggregating P2P results | `patterns/peer_to_peer.rs` |

#### 7.3 Micro-Agent Pipeline Adapter

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-65 | Implement Micro-Agent Pipeline pattern adapter: Working Memory slots, stateless steps | `patterns/micro_pipeline.rs` [NEW] |
| I-MA-66 | Integrate with Orchestrator DAG for pipeline step coordination | `patterns/micro_pipeline.rs` |

#### Phase 7 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P7-01 | `test_agent_not_found_gap_detection` | Delegate to non-existent agent | Gap classified as `AgentNotFound` |
| T-MA-P7-02 | `test_capability_mismatch_detection` | Agent lacks required tools | Gap classified as `CapabilityMismatch` |
| T-MA-P7-03 | `test_mode_inappropriate_detection` | `explore` agent for `build` task | Gap classified as `ModeInappropriate` |
| T-MA-P7-04 | `test_gap_resolution_spawns_architect` | `AgentNotFound` gap | `agent-architect` spawned |
| T-MA-P7-05 | `test_gap_resolution_success` | Architect creates valid agent | Original delegation succeeds |
| T-MA-P7-06 | `test_gap_resolution_failure_hitl` | Architect fails | HITL escalation |
| T-MA-P7-07 | `test_peer_to_peer_channel_communication` | 3 agents, shared channel | Messages exchanged |
| T-MA-P7-08 | `test_peer_to_peer_channel_reducer` | Aggregate results | Reduced output correct |
| T-MA-P7-09 | `test_micro_pipeline_wm_slots` | Step writes WM slot | Next step reads it |
| T-MA-P7-10 | `test_micro_pipeline_session_discard` | After step completes | Session context discarded |

---

### Phase 8: Anti-Pattern Migration, Observability & CLI Integration (Est. 3-4 days)

> **Goal**: Migrate existing anti-patterns to unified agent invocation, wire observability metrics, integrate built-in agents into CLI startup.

#### 8.1 Anti-Pattern Migration (AGENT_AUTONOMY.md §7)

| Task ID | Description | File | Migration |
|---------|-------------|------|-----------|
| I-MA-67 | Refactor `y-context/compaction.rs`: replace `CompactionLlm` trait with delegation to `compaction-summarizer` | `y-context` | Inject `Arc<dyn AgentDelegator>` |
| I-MA-68 | Refactor `y-agent/context.rs`: replace `apply_summary()` inline logic with delegation to `context-summarizer` | `context.rs` | Use `AgentDelegator.delegate()` |
| I-MA-69 | Wire `task-intent-analyzer` for `y-context` EnrichInput middleware | `y-context` | Inject `Arc<dyn AgentDelegator>` |
| I-MA-70 | Wire `pattern-extractor` for `y-skills/evolution.rs` | `y-skills` | Inject `Arc<dyn AgentDelegator>` |
| I-MA-71 | Wire `capability-assessor` for `y-hooks` capability mismatch assessment | `y-hooks` | Inject `Arc<dyn AgentDelegator>` |
| I-MA-71a | Refactor `y-session/manager.rs`: replace `generate_title()` with delegation to `title-generator` | `y-session` | Inject `Arc<dyn AgentDelegator>` |

#### 8.2 Observability

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-72 | Emit delegation metrics: `agents.delegations.total`, `duration_ms`, `failed` | `pool.rs` / `executor.rs` |
| I-MA-73 | Emit pool metrics: `agents.pool.active_instances`, `queued`, `rejected` | `pool.rs` |
| I-MA-74 | Emit instance metrics: `agents.instance.iterations`, `tool_calls`, `tokens_used` | `executor.rs` |
| I-MA-75 | Emit context metrics: `agents.context.strategy_used`, `tokens_shared` | `context.rs` |
| I-MA-76 | Emit gap metrics: `autonomy.gaps.detected`, `resolved`, `escalated`, `resolution_duration_ms` | `gap.rs` |
| I-MA-77 | Emit agent lifecycle metrics: `autonomy.agents.created`, `updated`, `deactivated`, `validation_failures` | `meta_tools.rs` |
| I-MA-78 | Ensure each delegation creates a child trace span linked to parent | `executor.rs` |

#### 8.3 CLI Startup Wiring

| Task ID | Description | File |
|---------|-------------|------|
| I-MA-79 | Register all 8 built-in agents in `AgentRegistry` at CLI startup | `y-cli` |
| I-MA-80 | Inject `AgentPool` as `Arc<dyn AgentDelegator>` into `y-context`, `y-session`, `y-skills`, `y-hooks` | `y-cli` |
| I-MA-81 | Register `task` tool and 4 meta-tools in ToolRegistry at startup | `y-cli` |

#### Phase 8 Test Plan

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-P8-01 | `test_compaction_uses_agent_delegation` | Trigger compaction | `compaction-summarizer` agent invoked, no direct `ProviderPool` call |
| T-MA-P8-02 | `test_context_summary_uses_agent` | Context summary strategy | `context-summarizer` agent invoked |
| T-MA-P8-02a | `test_title_generation_uses_agent` | Generate session title | `title-generator` agent invoked via `AgentDelegator`, no direct `ProviderPool` call |
| T-MA-P8-03 | `test_delegation_emits_trace_span` | Any delegation | Child span created with agent_name |
| T-MA-P8-04 | `test_delegation_metrics_emitted` | Successful delegation | Counter incremented, duration recorded |
| T-MA-P8-05 | `test_pool_metrics_concurrent` | Multiple concurrent agents | `active_instances` reflects count |
| T-MA-P8-06 | `test_gap_metrics_on_detection` | Gap detected | `gaps.detected` counter incremented |
| T-MA-P8-07 | `test_cli_startup_builtin_agents` | App startup | 8 agents in registry |
| T-MA-P8-08 | `test_cli_startup_meta_tools` | App startup | 5 tools in registry (task + 4 meta) |

---

## 5. Integration Tests

After phase completion, add cross-phase integration tests:

| Test ID | File | Scenario | Phases Required |
|---------|------|----------|-----------------|
| T-MA-INT-01 | `tests/delegation_lifecycle.rs` | Full delegation: Registry → Pool → Executor → Result | P3 + P6 |
| T-MA-INT-02 | `tests/delegation_lifecycle.rs` | Sequential pattern: 2 agents with mock LLM | P3 + P6 |
| T-MA-INT-03 | `tests/delegation_lifecycle.rs` | Hierarchical: supervisor + 2 parallel workers | P3 + P6 |
| T-MA-INT-04 | `tests/agent_gap_resolution.rs` | AgentNotFound → agent-architect → auto-create → retry | P4 + P5 + P7 |
| T-MA-INT-05 | `tests/agent_gap_resolution.rs` | Dynamic agent SQLite persist → restart → reload | P5 |
| T-MA-INT-06 | `tests/unified_invocation.rs` | Compaction triggers `compaction-summarizer` via `AgentDelegator` | P2 + P4 + P8 |
| T-MA-INT-07 | `tests/unified_invocation.rs` | Permission inheritance end-to-end: root → child → grandchild | P1 + P5 |
| T-MA-INT-08 | `tests/task_tool_integration.rs` | In-conversation delegation via `task` tool with depth limit | P6 |

---

## 6. Phase Dependencies

```
Phase 1 (Foundation Types & Trust)
  ↓
Phase 2 (AgentDelegator Trait in y-core)
  ↓
Phase 3 (Registry & Pool Separation)
  ↓
Phase 4 (Mode + Context + Built-in Agents)  ←→  Phase 5 (Meta-Tools + Persistence)  [parallelizable]
  ↓                                                ↓
Phase 6 (AgentExecutor + Task Tool + Orchestrator Integration)  ← depends on P3 + P4 + P5
  ↓
Phase 7 (Gap Resolution + P2P + Micro Pipeline)  ← depends on P4 + P5 + P6
  ↓
Phase 8 (Migration + Observability + CLI)  ← depends on P2 + P4 + P6 + P7
```

---

## 7. Feature Flags

All features gate behind flags for independent rollback:

| Feature Flag | Scope | Default |
|-------------|-------|---------|
| `multi_agent` | Entire multi-agent framework (AgentExecutor, Pool, Registry) | Enabled |
| `agent_modes` | Mode overlays (tool filtering, prompt injection) | Enabled |
| `agent_pool` | Concurrency limits and instance management | Enabled |
| `dynamic_agents` | Dynamic agent creation/update/deactivate meta-tools | Enabled |
| `capability_gap_resolution` | CapabilityGapMiddleware (tool + agent gaps) | Enabled |
| `agent_autonomy` | Unified agent invocation (`AgentDelegator` wiring) | Enabled |
| `peer_to_peer_pattern` | P2P collaboration pattern | Disabled |
| `micro_pipeline_pattern` | Micro-Agent Pipeline pattern | Disabled |

---

## 8. Resource Governance Defaults

| Parameter | Default | Configurable |
|-----------|---------|-------------|
| `max_concurrent_agents` (global) | 10 | Yes (`config.toml`) |
| `max_agents_per_delegation` | 5 | Yes (`config.toml`) |
| `dynamic_agent_workspace_limit` | 50 | Yes (`config.toml`) |
| `default_delegation_depth` | 2 | Yes (`config.toml`) |
| `default_timeout` | 5m | Yes (per-agent TOML) |
| `max_llm_calls_per_instance` | 30 | Yes (per-agent TOML) |
| `max_tool_calls_per_instance` | 100 | Yes (per-agent TOML) |
| `max_tokens_per_instance` | 50000 | Yes (per-agent TOML) |
| `instance_memory_limit` | 256MB | Yes (`config.toml`) |

---

## 9. Test Statistics Summary

| Phase | Unit Tests | Integration Tests | Total |
|-------|-----------|------------------|-------|
| Phase 1 | 12 | 0 | 12 |
| Phase 2 | 7 | 0 | 7 |
| Phase 3 | 10 | 0 | 10 |
| Phase 4 | 13 | 0 | 13 |
| Phase 5 | 12 | 0 | 12 |
| Phase 6 | 10 | 0 | 10 |
| Phase 7 | 10 | 0 | 10 |
| Phase 8 | 8 | 0 | 8 |
| Integration | 0 | 8 | 8 |
| **Total** | **82** | **8** | **90** |

---

## 10. Performance Targets

| Metric | Target | Phase |
|--------|--------|-------|
| Delegation overhead (instance creation + context injection) | < 50ms | P6 |
| Agent pool scheduling | < 1ms | P3 |
| Context summary generation | < 5s (LLM call) | P4 |
| Context filtering | < 10ms | P4 |
| Agent instance teardown | < 5ms | P6 |
| `agent_create` (validate + persist + register) | < 2s | P5 |
| `agent_update` (re-validate + persist) | < 500ms | P5 |
| `agent_search` (keyword match) | < 5ms | P5 |
| CapabilityGapMiddleware overhead (no gap) | < 1ms | P7 |
| `agent-architect` resolution | < 30s | P7 |

---

## 11. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| All tests pass | 100% | `cargo test -p y-agent` |
| Clippy clean | 0 warnings | `cargo clippy -p y-agent -- -D warnings` |
| Test coverage | >= 75% (min), 85% (aspirational) | `cargo llvm-cov -p y-agent` |
| Documentation | 0 warnings | `cargo doc -p y-agent --no-deps` |
| Workspace build | Clean | `cargo build --workspace` |

---

## 12. Acceptance Criteria

- [ ] `AgentDelegator` trait defined in `y-core`; `AgentPool` implements it
- [ ] `AgentRegistry` unifies management of BuiltIn/UserDefined/Dynamic agents
- [ ] 8 built-in system agents defined in TOML and registered at startup
- [ ] `TrustTier` ordering: `BuiltIn > UserDefined > Dynamic` with `PartialOrd`
- [ ] `EffectivePermissions` computed correctly (intersection/union/min)
- [ ] Three-stage validation pipeline (Schema → Permission → Safety)
- [ ] `AgentPool` enforces concurrency limits via Semaphore
- [ ] `AgentExecutor` implements full lifecycle with Orchestrator integration
- [ ] `task` tool enables in-conversation delegation with depth check
- [ ] 4 meta-tools (`agent_create/update/deactivate/search`) functional
- [ ] `DynamicAgentStore` persisted in SQLite; survives restart
- [ ] `CapabilityGapMiddleware` detects and resolves agent gaps
- [ ] Mode overlay correctly filters tools per mode
- [ ] Context strategies (None/Summary/Filtered/Full) implemented
- [ ] Anti-patterns migrated: `CompactionLlm` → agent delegation, `generate_title` → agent delegation
- [ ] All delegations emit trace spans and metrics
- [ ] Coverage >= 75%

---

## 13. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| `AgentDelegator` trait in `y-core` affects all dependent crates | Design trait carefully with minimal surface; feature-flag dependent usage |
| Pool refactor breaks existing consumers | Retain deprecated wrappers; migrate incrementally |
| Phase 8 anti-pattern migration touches 4+ crates | One crate at a time; each migration is independently testable |
| `SummaryStrategy` requires working LLM (circular: needs agent to summarize context for agent) | Use `NoneStrategy` as default for built-in system agents; `SummaryStrategy` only for user-facing delegations |
| SQLite migration conflicts with existing schema | Follow existing `y-storage` sqlx migration patterns and naming |
| P2P and Micro-Pipeline depend on incomplete Orchestrator features | Defer behind feature flags; develop against mock interfaces |

---

## 14. Deferred Items

| Item | Reason | Target Phase |
|------|--------|--------------|
| Dynamic Tool Lifecycle (`tool_create`, `tool_update`) | Part of agent-autonomy-design Phase 1-4; separate plan | Separate R&D plan |
| Self-Orchestration Protocol (`workflow_create/list/get`) | Part of agent-autonomy-design Phase 2; separate plan | Separate R&D plan |
| Parameterized Scheduling (`schedule_create/list`) | Part of agent-autonomy-design Phase 3; separate plan | Separate R&D plan |
| Distributed multi-node agent pools | Out of scope per design | Future |
| Persistent long-running (daemon) agents | Out of scope per design | Future |
| Agent-to-agent streaming | Open Question #2 in design; pending benchmarking | Future |
| Agent capability negotiation | Open Question #1 in design | Future |
