# R&D Plan: y-tools

**Module**: `crates/y-tools`
**Phase**: 3.2 (Execution Layer)
**Priority**: High — tools are the primary mechanism for agent actions
**Design References**: `tools-design.md`, `runtime-tools-integration-design.md`
**Depends On**: `y-core`, `y-hooks` (ToolMiddleware chain)

---

## 1. Module Purpose

`y-tools` implements the tool registry supporting four tool types (Built-in, MCP, Custom, Dynamic). It provides lazy loading via `ToolIndex` + `ToolSearch`, JSON Schema parameter validation, session-scoped `ToolActivationSet` with LRU eviction, and integration with the Tool middleware chain in `y-hooks`.

---

## 2. Dependency Map

```
y-tools
  ├── y-core (traits: Tool, ToolRegistry, ToolDefinition, ToolInput/Output, ToolError)
  ├── y-hooks (ToolMiddleware chain execution — injected via trait)
  ├── jsonschema (JSON Schema Draft 7 validation)
  ├── tokio (async)
  ├── serde / serde_json (tool definitions, parameters)
  ├── thiserror (errors)
  └── tracing (tool_name, tool_type, duration spans)
```

---

## 3. Module Structure

```
y-tools/src/
  lib.rs              — Public API: ToolRegistryImpl, re-exports
  error.rs            — ToolRegistryError
  config.rs           — ToolRegistryConfig (max_active, search_limit, etc.)
  registry.rs         — ToolRegistryImpl: ToolRegistry trait impl
  index.rs            — ToolIndex: compact entries for context injection
  activation.rs       — ToolActivationSet: LRU cache of active tools (ceiling 20)
  validator.rs        — JsonSchemaValidator: compiled schema cache
  executor.rs         — ToolExecutor: middleware chain integration
  builtin/
    mod.rs            — Built-in tool registration
    tool_search.rs    — ToolSearch meta-tool
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-TOOL-001 — ToolIndex

```
FILE: crates/y-tools/src/index.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-001-01 | `test_index_returns_compact_entries` | `tool_index()` | Entries contain only name, description, category |
| T-TOOL-001-02 | `test_index_excludes_full_schema` | Index entries | No `parameters` or `capabilities` field |
| T-TOOL-001-03 | `test_index_includes_all_registered_tools` | Register 5 tools | Index has 5 entries |
| T-TOOL-001-04 | `test_index_updates_on_register` | Register new tool | Index includes new entry |
| T-TOOL-001-05 | `test_index_updates_on_unregister` | Remove a tool | Index excludes removed entry |

#### Task: T-TOOL-002 — ToolActivationSet (LRU)

```
FILE: crates/y-tools/src/activation.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-002-01 | `test_activation_add_tool` | Activate a tool | Tool in active set |
| T-TOOL-002-02 | `test_activation_lru_eviction_at_ceiling` | Activate 21 tools (ceiling 20) | Oldest evicted |
| T-TOOL-002-03 | `test_activation_access_refreshes_lru` | Access tool, then fill to ceiling | Recently accessed not evicted |
| T-TOOL-002-04 | `test_activation_always_active_not_evicted` | Mark tool as always_active | Never evicted regardless of LRU |
| T-TOOL-002-05 | `test_activation_get_active_definitions` | 5 active tools | Returns 5 full definitions |
| T-TOOL-002-06 | `test_activation_deactivate` | Remove specific tool | No longer in active set |

#### Task: T-TOOL-003 — JSON Schema validator

```
FILE: crates/y-tools/src/validator.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-003-01 | `test_validate_valid_params` | Valid JSON against schema | Ok |
| T-TOOL-003-02 | `test_validate_missing_required_field` | Missing required field | `ValidationError` |
| T-TOOL-003-03 | `test_validate_wrong_type` | String where number expected | `ValidationError` |
| T-TOOL-003-04 | `test_validate_additional_properties_denied` | Extra field in strict schema | `ValidationError` |
| T-TOOL-003-05 | `test_validate_compiled_schema_cache` | Same schema validated twice | Second call uses cached compiled |
| T-TOOL-003-06 | `test_validate_empty_schema_accepts_all` | `{}` schema | Accepts any JSON object |
| T-TOOL-003-07 | `test_validate_nested_object` | Nested object schema | Validates nested fields |

#### Task: T-TOOL-004 — ToolRegistryImpl

```
FILE: crates/y-tools/src/registry.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-004-01 | `test_registry_register_tool` | `register()` a tool definition | Retrievable via `get()` |
| T-TOOL-004-02 | `test_registry_get_not_found` | `get()` non-existent tool | `ToolError::NotFound` |
| T-TOOL-004-03 | `test_registry_search_by_name` | `search("file")` | Returns tools matching name |
| T-TOOL-004-04 | `test_registry_search_by_keyword` | `search("filesystem")` | Returns tools in FileSystem category |
| T-TOOL-004-05 | `test_registry_unregister` | `unregister()` | Tool no longer findable |
| T-TOOL-004-06 | `test_registry_register_duplicate_name_updates` | Register same name twice | Second replaces first |

#### Task: T-TOOL-005 — ToolExecutor (middleware integration)

```
FILE: crates/y-tools/src/executor.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-005-01 | `test_executor_validates_params_before_execution` | Invalid params | `ValidationError` before tool runs |
| T-TOOL-005-02 | `test_executor_runs_through_middleware_chain` | Execute with ToolMiddleware | Chain invoked pre and post |
| T-TOOL-005-03 | `test_executor_middleware_abort_prevents_execution` | Guardrail aborts | Tool not executed |
| T-TOOL-005-04 | `test_executor_returns_output_on_success` | Successful execution | `ToolOutput` with success=true |
| T-TOOL-005-05 | `test_executor_captures_runtime_error` | Tool panics | `ToolError::RuntimeError` |

#### Task: T-TOOL-006 — ToolSearch meta-tool

```
FILE: crates/y-tools/src/builtin/tool_search.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-TOOL-006-01 | `test_tool_search_returns_full_definitions` | Search query | Returns full `ToolDefinition`s (not index entries) |
| T-TOOL-006-02 | `test_tool_search_activates_found_tools` | Search and select | Found tools added to ToolActivationSet |
| T-TOOL-006-03 | `test_tool_search_respects_limit` | `search("file")` with limit=3 | At most 3 results |

### 4.2 Integration Tests

```
FILE: crates/y-tools/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-TOOL-INT-01 | `registry_integration_test.rs` | `test_register_search_execute_flow` | Register tool → search → validate → execute |
| T-TOOL-INT-02 | `registry_integration_test.rs` | `test_lazy_loading_full_cycle` | Index injection → ToolSearch → activation → execution |
| T-TOOL-INT-03 | `registry_integration_test.rs` | `test_lru_eviction_under_load` | Register 30 tools, activate all, verify LRU ceiling |
| T-TOOL-INT-04 | `registry_integration_test.rs` | `test_dynamic_tool_registration` | Register dynamic tool at runtime, execute in sandbox |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-TOOL-001 | `ToolRegistryImpl` | In-memory registry, `ToolRegistry` trait impl | High |
| I-TOOL-002 | `ToolIndex` | Compact index generation from registry | High |
| I-TOOL-003 | `ToolActivationSet` | LRU cache with ceiling, always_active support | High |
| I-TOOL-004 | `JsonSchemaValidator` | Compiled schema cache, Draft 7 validation | High |
| I-TOOL-005 | `ToolExecutor` | Middleware chain integration, validation, execution | High |
| I-TOOL-006 | `ToolSearch` built-in | Meta-tool for lazy loading, activates found tools | High |
| I-TOOL-007 | Built-in tool skeleton | Registration framework for built-in tools | Medium |
| I-TOOL-008 | Dynamic tool support | Agent-created tools with mandatory sandboxing | Medium |

---

## 6. Performance Benchmarks

```
FILE: crates/y-tools/benches/tool_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Tool dispatch (excluding LLM) | P95 < 100ms | `criterion` |
| JSON Schema validation | P95 < 1ms | `criterion` |
| ToolIndex generation (100 tools) | P95 < 5ms | `criterion` |
| ToolActivationSet LRU (20 tools) | P95 < 100us | `criterion` |
| ToolSearch query | P95 < 10ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-tools` |
| Clippy clean | 0 warnings | `cargo clippy -p y-tools` |
| Schema validation | 100% Draft 7 compliance | `jsonschema` test suite |

---

## 8. Acceptance Criteria

- [ ] 4 tool types (Built-in, MCP, Custom, Dynamic) registerable
- [ ] Lazy loading works: ToolIndex → ToolSearch → activate → execute
- [ ] LRU eviction at ceiling 20, always_active tools exempt
- [ ] JSON Schema validation rejects invalid parameters before execution
- [ ] Middleware chain (Tool chain in y-hooks) invoked pre/post execution
- [ ] Guardrail abort prevents tool execution
- [ ] Dynamic tools always sandboxed (`is_sandboxed = true`)
- [ ] Coverage >= 80%
