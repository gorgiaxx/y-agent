# R&D Plan: y-multi-agent

**Module**: `crates/y-multi-agent`
**Phase**: 4.4 (Intelligence Layer)
**Priority**: Medium — extends single-agent to multi-agent collaboration
**Design References**: `multi-agent-design.md`, `agent-autonomy-design.md`
**Depends On**: `y-core`, `y-hooks`, `y-agent-core`, `y-session`

---

## 1. Module Purpose

`y-multi-agent` enables multiple agents to collaborate on tasks. It provides agent definitions (TOML-based), an agent pool, a delegation protocol, and multiple collaboration patterns. Initial implementation focuses on Sequential and Hierarchical patterns; P2P and Micro-Agent Pipeline are deferred.

---

## 2. Dependency Map

```
y-multi-agent
  ├── y-core (traits: AgentId, SessionStore)
  ├── y-agent-core (Orchestrator for sub-agent execution)
  ├── y-session (child/ephemeral sessions for delegated tasks)
  ├── y-hooks (AgentGap hook points, DynamicAgent events)
  ├── toml (TOML agent definition parsing)
  ├── tokio (async, task spawning for parallel agents)
  ├── serde / serde_json (agent definitions, delegation messages)
  ├── thiserror (errors)
  └── tracing (agent_id, delegation, collaboration_pattern spans)
```

---

## 3. Module Structure

```
y-multi-agent/src/
  lib.rs              — Public API: AgentManager
  error.rs            — MultiAgentError
  config.rs           — MultiAgentConfig (max agents, trust tiers)
  definition.rs       — AgentDefinition: TOML parsing, validation
  pool.rs             — AgentPool: lifecycle management, soft delete
  delegation.rs       — DelegationProtocol: task delegation with result collection
  patterns/
    mod.rs            — Pattern selection
    sequential.rs     — SequentialPattern: agents execute in order
    hierarchical.rs   — HierarchicalPattern: supervisor delegates to workers
  trust.rs            — TrustTier: trusted/verified/untrusted permission scoping
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-MA-001 — Agent definition parsing

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-001-01 | `test_agent_definition_from_toml` | Parse valid TOML | All fields populated |
| T-MA-001-02 | `test_agent_definition_missing_name_fails` | TOML without name | Validation error |
| T-MA-001-03 | `test_agent_definition_mode_validation` | Invalid mode string | Error |
| T-MA-001-04 | `test_agent_definition_serialization` | Roundtrip TOML → struct → TOML | Identity |

#### Task: T-MA-002 — Agent pool

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-002-01 | `test_pool_register_agent` | Register agent | Retrievable by ID |
| T-MA-002-02 | `test_pool_deactivate_soft_delete` | Deactivate agent | `is_active=false`, not hard deleted |
| T-MA-002-03 | `test_pool_list_active_only` | List agents | Excludes deactivated |
| T-MA-002-04 | `test_pool_get_by_name` | Get agent by name | Returns correct definition |
| T-MA-002-05 | `test_pool_max_agents_enforced` | Exceed max agents | Error |

#### Task: T-MA-003 — Delegation protocol

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-003-01 | `test_delegation_creates_child_session` | Delegate task | Child session created with agent_id |
| T-MA-003-02 | `test_delegation_returns_result` | Agent completes task | Result collected in parent |
| T-MA-003-03 | `test_delegation_timeout` | Agent exceeds timeout | Timeout error, session archived |
| T-MA-003-04 | `test_delegation_error_propagation` | Agent fails | Error propagated to delegator |
| T-MA-003-05 | `test_delegation_trust_scoping` | Untrusted agent | Permission snapshot applied |

#### Task: T-MA-004 — Collaboration patterns

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MA-004-01 | `test_sequential_agents_execute_in_order` | A → B → C | Output of A is input to B |
| T-MA-004-02 | `test_sequential_failure_stops_chain` | B fails | C not executed |
| T-MA-004-03 | `test_hierarchical_supervisor_delegates` | Supervisor splits task | Workers receive subtasks |
| T-MA-004-04 | `test_hierarchical_result_aggregation` | Workers complete | Supervisor receives all results |
| T-MA-004-05 | `test_hierarchical_worker_failure` | One worker fails | Supervisor handles error |

### 4.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-MA-INT-01 | `delegation_integration_test.rs` | `test_full_delegation_lifecycle` | Define agent → delegate → execute → collect result |
| T-MA-INT-02 | `delegation_integration_test.rs` | `test_sequential_two_agents` | Two agents in sequence with mock LLM |
| T-MA-INT-03 | `delegation_integration_test.rs` | `test_hierarchical_with_parallel_workers` | Supervisor + 2 parallel workers |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-MA-001 | `AgentDefinition` | TOML parsing, validation, mode enum | High |
| I-MA-002 | `AgentPool` | Registration, soft delete, listing, max limit | High |
| I-MA-003 | `DelegationProtocol` | Task delegation, session creation, result collection | High |
| I-MA-004 | `SequentialPattern` | Ordered agent chain execution | High |
| I-MA-005 | `HierarchicalPattern` | Supervisor-worker delegation | High |
| I-MA-006 | `TrustTier` | Permission scoping by trust level | Medium |
| I-MA-007 | Dynamic agent lifecycle | Agent creation/deactivation at runtime | Medium |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 75% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-multi-agent` |
| Clippy clean | 0 warnings | `cargo clippy -p y-multi-agent` |

---

## 7. Acceptance Criteria

- [ ] Agent definitions parse from TOML with validation
- [ ] Agent pool manages lifecycle with soft delete
- [ ] Delegation creates child sessions and collects results
- [ ] Sequential pattern executes agents in order
- [ ] Hierarchical pattern delegates to parallel workers
- [ ] Trust tiers scope permissions correctly
- [ ] Coverage >= 75%
