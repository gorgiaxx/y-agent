# R&D Plan: y-guardrails

**Module**: `crates/y-guardrails`
**Phase**: 4.3 (Intelligence Layer)
**Priority**: High — safety and trust enforcement
**Design References**: `guardrails-hitl-design.md`
**Depends On**: `y-core`, `y-hooks` (implemented as middleware)

---

## 1. Module Purpose

`y-guardrails` implements safety validators, LoopGuard pattern detection, taint tracking, and the unified permission model. All guardrails are implemented as `Middleware` in the `y-hooks` chains (Tool and LLM), not as a parallel system. The module also provides the HITL escalation protocol for human-in-the-loop approval.

---

## 2. Dependency Map

```
y-guardrails
  ├── y-core (traits: Middleware, MiddlewareContext, ToolDefinition)
  ├── y-hooks (registers as ToolMiddleware and LlmMiddleware)
  ├── tokio (async, interrupt channels)
  ├── serde_json (policy parsing, taint metadata)
  ├── thiserror (errors)
  └── tracing (guardrail_name, decision, risk_score spans)
```

---

## 3. Module Structure

```
y-guardrails/src/
  lib.rs              — Public API: GuardrailManager
  error.rs            — GuardrailError
  config.rs           — GuardrailConfig (permission policies, loop thresholds)
  permission.rs       — PermissionModel: allow/notify/ask/deny per tool
  loop_guard.rs       — LoopGuard: 4 pattern types (repetition, oscillation, drift, redundant tool)
  taint.rs            — TaintTracker: tag propagation through data flow
  risk.rs             — RiskScorer: composite risk assessment
  middleware/
    mod.rs            — Middleware registrations
    tool_guard.rs     — ToolGuardMiddleware: pre-execution permission + risk check
    llm_guard.rs      — LlmGuardMiddleware: output safety validation
    loop_detector.rs  — LoopDetectorMiddleware: pattern detection in agent loop
  hitl.rs             — HitlProtocol: escalation, user prompt, timeout
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-GUARD-001 — Permission model

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-GUARD-001-01 | `test_permission_allow_passes` | Tool with `allow` policy | Execution permitted |
| T-GUARD-001-02 | `test_permission_deny_blocks` | Tool with `deny` policy | Execution blocked, abort reason |
| T-GUARD-001-03 | `test_permission_ask_triggers_hitl` | Tool with `ask` policy | HITL interrupt triggered |
| T-GUARD-001-04 | `test_permission_notify_logs_and_continues` | Tool with `notify` policy | Execution permitted, event emitted |
| T-GUARD-001-05 | `test_permission_dangerous_tool_requires_ask` | `is_dangerous=true`, no explicit policy | Defaults to `ask` |
| T-GUARD-001-06 | `test_permission_per_tool_override` | Global `allow`, tool-specific `deny` | Tool-level wins |

#### Task: T-GUARD-002 — LoopGuard

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-GUARD-002-01 | `test_loop_detect_repetition` | Same action 5 times | Pattern detected: Repetition |
| T-GUARD-002-02 | `test_loop_detect_oscillation` | A → B → A → B → A | Pattern detected: Oscillation |
| T-GUARD-002-03 | `test_loop_detect_drift` | 10 steps with no progress metric change | Pattern detected: Drift |
| T-GUARD-002-04 | `test_loop_detect_redundant_tool` | Same tool, same args, 3 times | Pattern detected: RedundantToolCall |
| T-GUARD-002-05 | `test_loop_no_false_positive` | Varied actions, clear progress | No detection |
| T-GUARD-002-06 | `test_loop_configurable_threshold` | Custom repetition threshold of 3 | Detects at 3, not 5 |
| T-GUARD-002-07 | `test_loop_detection_resets_on_progress` | Repeat 3, then progress, then repeat 3 | No detection (reset) |

#### Task: T-GUARD-003 — Taint tracking

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-GUARD-003-01 | `test_taint_tag_user_input` | User input marked as tainted | Taint tag propagates |
| T-GUARD-003-02 | `test_taint_propagation_through_tool` | Tainted input → tool → output | Output inherits taint |
| T-GUARD-003-03 | `test_taint_sanitization` | Tainted data passes sanitizer | Taint removed |
| T-GUARD-003-04 | `test_taint_blocks_dangerous_sink` | Tainted data → filesystem write | Blocked by policy |
| T-GUARD-003-05 | `test_taint_no_propagation_for_clean_data` | Clean input | No taint tags |

#### Task: T-GUARD-004 — Risk scorer

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-GUARD-004-01 | `test_risk_score_safe_tool` | Read-only tool | Low risk score |
| T-GUARD-004-02 | `test_risk_score_dangerous_tool` | Shell execution tool | High risk score |
| T-GUARD-004-03 | `test_risk_score_composite` | Multiple risk factors | Scores combine correctly |
| T-GUARD-004-04 | `test_risk_threshold_escalation` | Score exceeds threshold | Escalates to `ask` |

#### Task: T-GUARD-005 — HITL protocol

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-GUARD-005-01 | `test_hitl_escalation_pauses_execution` | HITL triggered | Execution paused |
| T-GUARD-005-02 | `test_hitl_user_approves` | User responds "approve" | Execution continues |
| T-GUARD-005-03 | `test_hitl_user_denies` | User responds "deny" | Execution aborted |
| T-GUARD-005-04 | `test_hitl_timeout` | No response within timeout | Defaults to deny |

### 4.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-GUARD-INT-01 | `guardrail_integration_test.rs` | `test_tool_guard_in_middleware_chain` | Tool chain with ToolGuard → tool execution |
| T-GUARD-INT-02 | `guardrail_integration_test.rs` | `test_loop_guard_stops_agent_loop` | Agent loop with repetitive actions → LoopGuard triggers |
| T-GUARD-INT-03 | `guardrail_integration_test.rs` | `test_taint_through_pipeline` | User input → tool → output, taint tracked |
| T-GUARD-INT-04 | `guardrail_integration_test.rs` | `test_hitl_full_flow` | Dangerous tool → HITL → user approve → execute |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-GUARD-001 | `PermissionModel` | Per-tool allow/notify/ask/deny with defaults | High |
| I-GUARD-002 | `LoopGuard` | 4 pattern detectors with configurable thresholds | High |
| I-GUARD-003 | `TaintTracker` | Tag propagation, sanitization, sink blocking | High |
| I-GUARD-004 | `RiskScorer` | Composite risk assessment from tool properties | Medium |
| I-GUARD-005 | `ToolGuardMiddleware` | Permission + risk check as ToolMiddleware | High |
| I-GUARD-006 | `LlmGuardMiddleware` | Output safety validation as LlmMiddleware | Medium |
| I-GUARD-007 | `LoopDetectorMiddleware` | Loop detection in agent loop | High |
| I-GUARD-008 | `HitlProtocol` | Escalation with interrupt/resume integration | High |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-guardrails` |
| No bypass paths | Verified | Security review |
| False positive rate | < 5% for LoopGuard | Test suite with diverse scenarios |

---

## 7. Acceptance Criteria

- [ ] Permission model enforces allow/notify/ask/deny per tool
- [ ] LoopGuard detects all 4 pattern types with configurable thresholds
- [ ] Taint tracking propagates tags through tool execution
- [ ] Risk scorer produces correct composite scores
- [ ] HITL escalation pauses execution and respects user decision
- [ ] All guardrails are middleware (no parallel system)
- [ ] False positive rate < 5% on varied test scenarios
- [ ] Coverage >= 80%
