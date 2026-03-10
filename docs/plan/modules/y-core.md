# R&D Plan: y-core

**Module**: `crates/y-core`
**Phase**: 1.4 (Engineering Foundation)
**Priority**: Critical — all other crates depend on this
**Design References**: All design documents (trait source of truth)
**Status**: Trait definitions complete, tests pending

---

## 1. Module Purpose

`y-core` defines the trait contracts and shared types that mediate all cross-crate interactions. It contains zero business logic — only types, traits, and error definitions. Every other crate in the workspace depends inward on `y-core`.

---

## 2. Dependency Map

```
y-core has NO internal dependencies (leaf crate)

Depended on by: ALL other crates
External deps: async-trait, serde, serde_json, futures, thiserror, uuid, chrono
```

---

## 3. Module Structure

```
y-core/src/
  lib.rs          — Module re-exports and documentation table
  types.rs        — Shared IDs (SessionId, WorkflowId, etc.), Message, TokenUsage
  error.rs        — ClassifiedError trait, Redactable trait, ErrorSeverity, CoreError
  provider.rs     — LlmProvider, ProviderPool, ChatRequest/Response, ProviderError
  tool.rs         — Tool, ToolRegistry, ToolDefinition, ToolInput/Output, ToolError
  runtime.rs      — RuntimeAdapter, RuntimeCapability model, ExecutionRequest/Result
  memory.rs       — MemoryClient, ExperienceStore, Memory, MemoryQuery
  hook.rs         — Middleware, HookHandler, EventSubscriber, Event, ChainType
  session.rs      — SessionStore, TranscriptStore, SessionNode, SessionType/State
  checkpoint.rs   — CheckpointStorage, WorkflowCheckpoint, CheckpointStatus
  skill.rs        — SkillRegistry, SkillManifest, SkillVersion, SubDocumentRef
```

---

## 4. Development Tasks

### 4.1 Trait Contract Tests (TDD — Red Phase)

Write tests that define the expected behavior of each trait before any downstream implementation exists. These tests use mock implementations to validate the contract.

#### Task: T-CORE-001 — Types module tests

```
FILE: crates/y-core/src/types.rs
TEST_LOCATION: crates/y-core/src/types.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-001-01 | `test_session_id_new_generates_unique_ids` | `SessionId::new()` | Two calls produce different IDs |
| T-CORE-001-02 | `test_session_id_from_string_roundtrip` | `SessionId::from_string()` + `as_str()` | Roundtrip preserves value |
| T-CORE-001-03 | `test_session_id_display_matches_inner` | `Display` impl for `SessionId` | `format!("{id}")` equals inner string |
| T-CORE-001-04 | `test_all_id_types_default_generates_valid` | `Default` for all ID types | Non-empty string |
| T-CORE-001-05 | `test_message_serialization_roundtrip` | `Message` serde | JSON serialize → deserialize identity |
| T-CORE-001-06 | `test_token_usage_addition` | `TokenUsage` arithmetic (if applicable) | Field-wise sum is correct |
| T-CORE-001-07 | `test_tool_call_request_serialization` | `ToolCallRequest` serde | JSON roundtrip |

#### Task: T-CORE-002 — Error module tests

```
FILE: crates/y-core/src/error.rs
TEST_LOCATION: crates/y-core/src/error.rs (#[cfg(test)])
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-002-01 | `test_error_severity_transient_is_retryable` | `ErrorSeverity::Transient` | Matches expected semantics |
| T-CORE-002-02 | `test_error_severity_permanent_not_retryable` | `ErrorSeverity::Permanent` | Matches expected semantics |
| T-CORE-002-03 | `test_core_error_display_messages` | `CoreError` variants | Each variant produces non-empty display |
| T-CORE-002-04 | `test_redactable_trait_contract` | Mock `Redactable` impl | `redacted()` removes sensitive data |

#### Task: T-CORE-003 — Provider trait contract tests

```
FILE: crates/y-core/tests/provider_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-003-01 | `test_provider_error_severity_rate_limit_is_transient` | `ProviderError::RateLimited.severity()` | Returns `Transient` |
| T-CORE-003-02 | `test_provider_error_severity_auth_is_permanent` | `ProviderError::AuthenticationFailed.severity()` | Returns `Permanent` |
| T-CORE-003-03 | `test_provider_error_severity_no_provider_is_user_action` | `ProviderError::NoProviderAvailable.severity()` | Returns `UserActionRequired` |
| T-CORE-003-04 | `test_provider_error_severity_network_is_transient` | `ProviderError::NetworkError.severity()` | Returns `Transient` |
| T-CORE-003-05 | `test_chat_request_construction` | `ChatRequest` field access | All fields accessible |
| T-CORE-003-06 | `test_chat_response_serialization_roundtrip` | `ChatResponse` serde | JSON roundtrip preserves all fields |
| T-CORE-003-07 | `test_finish_reason_serde_rename` | `FinishReason` serde | `"tool_use"` deserializes to `ToolUse` |
| T-CORE-003-08 | `test_route_request_default` | `RouteRequest::default()` | Empty tags, no model, Normal priority |
| T-CORE-003-09 | `test_provider_metadata_cost_fields` | `ProviderMetadata` | Cost fields are non-negative |

#### Task: T-CORE-004 — Tool trait contract tests

```
FILE: crates/y-core/tests/tool_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-004-01 | `test_tool_error_retryable_timeout` | `ToolError::Timeout.is_retryable()` | Returns `true` |
| T-CORE-004-02 | `test_tool_error_not_retryable_not_found` | `ToolError::NotFound.is_retryable()` | Returns `false` |
| T-CORE-004-03 | `test_tool_error_retryable_rate_limited` | `ToolError::RateLimited.is_retryable()` | Returns `true` |
| T-CORE-004-04 | `test_tool_definition_serialization` | `ToolDefinition` serde | JSON roundtrip preserves all fields |
| T-CORE-004-05 | `test_tool_index_entry_compact` | `ToolIndexEntry` | Contains only name, description, category |
| T-CORE-004-06 | `test_tool_output_success_flag` | `ToolOutput` construction | `success` field reflects intent |
| T-CORE-004-07 | `test_tool_category_serde_rename` | `ToolCategory` serde | `"file_system"` deserializes to `FileSystem` |

#### Task: T-CORE-005 — Runtime trait contract tests

```
FILE: crates/y-core/tests/runtime_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-005-01 | `test_resource_limits_default_values` | `ResourceLimits::default()` | 512MB memory, 1.0 CPU, 300s timeout, 10MB output |
| T-CORE-005-02 | `test_runtime_capability_default_is_restrictive` | `RuntimeCapability::default()` | Network=None, no host access, no shell |
| T-CORE-005-03 | `test_execution_result_success_exit_zero` | `ExecutionResult::success()` | `true` when `exit_code == 0` |
| T-CORE-005-04 | `test_execution_result_failure_nonzero` | `ExecutionResult::success()` | `false` when `exit_code != 0` |
| T-CORE-005-05 | `test_execution_result_stdout_string` | `ExecutionResult::stdout_string()` | Valid UTF-8 conversion |
| T-CORE-005-06 | `test_network_capability_serde_tagged` | `NetworkCapability` serde | Internal tagged enum roundtrip |
| T-CORE-005-07 | `test_mount_mode_serde` | `MountMode` serde | `"read_only"` deserializes to `ReadOnly` |

#### Task: T-CORE-006 — Session trait contract tests

```
FILE: crates/y-core/tests/session_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-006-01 | `test_session_type_serde_roundtrip` | `SessionType` serde | All variants roundtrip |
| T-CORE-006-02 | `test_session_state_serde_roundtrip` | `SessionState` serde | All variants roundtrip |
| T-CORE-006-03 | `test_session_filter_default` | `SessionFilter::default()` | All fields are `None` |
| T-CORE-006-04 | `test_session_node_serialization` | `SessionNode` serde | JSON roundtrip preserves tree structure |
| T-CORE-006-05 | `test_session_error_display` | `SessionError` variants | Non-empty display strings |

#### Task: T-CORE-007 — Checkpoint trait contract tests

```
FILE: crates/y-core/tests/checkpoint_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-007-01 | `test_checkpoint_status_serde_roundtrip` | `CheckpointStatus` serde | All 5 variants roundtrip |
| T-CORE-007-02 | `test_workflow_checkpoint_serialization` | `WorkflowCheckpoint` serde | JSON roundtrip preserves committed/pending separation |
| T-CORE-007-03 | `test_checkpoint_error_display` | `CheckpointError` variants | Error messages contain context (workflow_id, etc.) |

#### Task: T-CORE-008 — Hook/Middleware trait contract tests

```
FILE: crates/y-core/tests/hook_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-008-01 | `test_middleware_context_new` | `MiddlewareContext::new()` | Not aborted, empty metadata |
| T-CORE-008-02 | `test_middleware_context_abort` | `MiddlewareContext::abort()` | `aborted == true`, reason set |
| T-CORE-008-03 | `test_chain_type_serde_roundtrip` | `ChainType` serde | All 5 variants roundtrip |
| T-CORE-008-04 | `test_hook_point_display` | `HookPoint::Display` | Non-empty string for all variants |
| T-CORE-008-05 | `test_event_serde_tagged` | `Event` serde | Internally tagged enum roundtrip |
| T-CORE-008-06 | `test_middleware_result_variants` | `MiddlewareResult` | `Continue` and `ShortCircuit` constructible |

#### Task: T-CORE-009 — Memory trait contract tests

```
FILE: crates/y-core/tests/memory_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-009-01 | `test_memory_type_serde_roundtrip` | `MemoryType` serde | All 4 variants roundtrip |
| T-CORE-009-02 | `test_evidence_type_serde_roundtrip` | `EvidenceType` serde | All 4 variants roundtrip |
| T-CORE-009-03 | `test_memory_serialization` | `Memory` serde | JSON roundtrip preserves importance and access_count |
| T-CORE-009-04 | `test_experience_record_serialization` | `ExperienceRecord` serde | Roundtrip preserves slot_index and evidence_type |
| T-CORE-009-05 | `test_memory_query_construction` | `MemoryQuery` | All fields accessible |

#### Task: T-CORE-010 — Skill trait contract tests

```
FILE: crates/y-core/tests/skill_contract_test.rs
```

| Test ID | Test Name | Behavior Under Test | Assertion |
|---------|-----------|-------------------|-----------|
| T-CORE-010-01 | `test_skill_version_display` | `SkillVersion::Display` | Displays inner hash string |
| T-CORE-010-02 | `test_skill_version_equality` | `SkillVersion` eq | Same hash is equal |
| T-CORE-010-03 | `test_skill_manifest_serialization` | `SkillManifest` serde | JSON roundtrip preserves sub_documents and token_estimate |
| T-CORE-010-04 | `test_skill_error_token_budget` | `SkillError::TokenBudgetExceeded` | Display shows actual vs max |
| T-CORE-010-05 | `test_skill_summary_compact` | `SkillSummary` | Does not contain root_content (compact) |

---

## 5. Implementation Tasks

Since `y-core` is trait-only, implementation is minimal — primarily ensuring all types compile, serialize correctly, and satisfy their contracts.

| Task ID | Task | Description | Files |
|---------|------|-------------|-------|
| I-CORE-001 | Verify all traits compile with mockall | Ensure `#[automock]` works on all traits | All trait files |
| I-CORE-002 | Add `From` conversions for error types | Cross-crate error conversion helpers | `error.rs` |
| I-CORE-003 | Validate serde attributes | Ensure all `rename_all`, `tag`, `skip_serializing_if` are correct | All files |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 90% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-core` |
| Clippy clean | 0 warnings | `cargo clippy -p y-core` |
| No unsafe | 0 blocks | Manual review |
| Doc build | 0 warnings | `cargo doc -p y-core --no-deps` |

---

## 7. Acceptance Criteria

- [ ] All 10 test task groups (T-CORE-001 through T-CORE-010) pass
- [ ] Every public type has at least one serialization roundtrip test
- [ ] Every error enum variant has a display test
- [ ] Every trait can be mocked with `mockall` for downstream crate testing
- [ ] `cargo test -p y-core` runs in < 5 seconds
- [ ] Coverage >= 90%
