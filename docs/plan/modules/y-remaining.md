# R&D Plan: Supporting Modules

**Modules**: `y-prompt`, `y-mcp`, `y-journal`, `y-scheduler`, `y-diagnostics`, `y-test-utils`
**Phase**: Various (see per-module sections)
**Priority**: These modules support the core system but have narrower scope

---

## 1. y-prompt (Phase 3.4)

**Purpose**: Structured prompt system — `PromptSection`, `PromptTemplate`, mode overlays, TOML-based `SectionStore`.
**Design Reference**: `prompt-design.md`
**Depends On**: `y-core`

### Module Structure

```
y-prompt/src/
  lib.rs              — Public API
  section.rs          — PromptSection: structured units with lazy loading
  template.rs         — PromptTemplate: mode overlays, template inheritance
  store.rs            — SectionStore: TOML-based section persistence
  budget.rs           — Per-section token budget enforcement
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROMPT-001 | `test_section_creation_with_metadata` | Create section | Category, priority, token_estimate set |
| T-PROMPT-002 | `test_template_mode_overlay` | Apply "code" mode overlay | Sections merged/replaced |
| T-PROMPT-003 | `test_template_inheritance` | Child inherits parent sections | Parent sections present |
| T-PROMPT-004 | `test_section_store_load_from_toml` | Load sections from TOML file | All sections parsed |
| T-PROMPT-005 | `test_section_lazy_loading` | Reference to external section | Loaded on demand |
| T-PROMPT-006 | `test_per_section_budget_enforcement` | Section exceeds budget | Truncated or error |
| T-PROMPT-007 | `test_template_serialization_roundtrip` | Template → TOML → Template | Identity |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-PROMPT-001 | `PromptSection` with metadata and lazy loading | High |
| I-PROMPT-002 | `PromptTemplate` with mode overlays and inheritance | High |
| I-PROMPT-003 | `SectionStore` TOML persistence | Medium |
| I-PROMPT-004 | Per-section token budget | Medium |

---

## 2. y-mcp (Phase 3.2)

**Purpose**: MCP (Model Context Protocol) support for third-party tool and memory integration.
**Design Reference**: `tools-design.md` (MCP tool type)
**Depends On**: `y-core`

### Module Structure

```
y-mcp/src/
  lib.rs              — Public API
  client.rs           — McpClient: connects to MCP servers                         ✅ implemented
  error.rs            — McpError                                                   ✅ implemented
  tool_adapter.rs     — McpToolAdapter: wraps MCP tools as y-core Tool trait        ✅ implemented
  memory_adapter.rs   — McpMemoryAdapter: wraps MCP memory as MemoryClient         ❌ NOT YET IMPLEMENTED
  transport.rs        — Transport layer (stdio, HTTP)                              ✅ implemented
  discovery.rs        — Server/tool discovery                                      ✅ implemented
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-MCP-001 | `test_mcp_client_connect` | Connect to MCP server (mock) | Connection established |
| T-MCP-002 | `test_mcp_tool_discovery` | List tools from server | Returns tool definitions |
| T-MCP-003 | `test_mcp_tool_execute` | Execute MCP tool | Result mapped to ToolOutput |
| T-MCP-004 | `test_mcp_tool_adapter_wraps_as_trait` | McpToolAdapter | Implements `Tool` trait |
| T-MCP-005 | `test_mcp_transport_stdio` | stdio transport | Message exchange works |
| T-MCP-006 | `test_mcp_memory_adapter` | MCP memory server | Implements `MemoryClient` trait |
| T-MCP-007 | `test_mcp_server_disconnect_handling` | Server disconnects | Graceful error |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-MCP-001 | `McpClient` with transport abstraction | High |
| I-MCP-002 | `McpToolAdapter` wrapping MCP tools as `Tool` | High |
| I-MCP-003 | `McpMemoryAdapter` wrapping MCP memory as `MemoryClient` | Medium |
| I-MCP-004 | stdio transport | High |
| I-MCP-005 | Tool/server discovery | Medium |

---

## 3. y-journal (Phase 3.3)

**Purpose**: Automatic file-level change tracking with three-tier storage and scope-based rollback.
**Design Reference**: `file-journal-design.md`
**Depends On**: `y-core`, `y-hooks` (FileJournalMiddleware), `y-storage`

### Module Structure

```
y-journal/src/
  lib.rs              — Public API: FileJournal
  middleware.rs        — FileJournalMiddleware: ToolMiddleware (pre) capturing file state
  storage.rs          — Three-tier storage: inline (<256KB), blob (256KB-10MB), git-ref
  rollback.rs         — ScopeRollback: rollback by workflow/task/step scope
  conflict.rs         — ConflictDetector: detect user modifications before rollback
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-JRNL-001 | `test_journal_capture_file_before_tool` | Tool modifies file | Original content captured |
| T-JRNL-002 | `test_journal_inline_tier_small_file` | File < 256KB | Stored inline |
| T-JRNL-003 | `test_journal_blob_tier_medium_file` | File 256KB-10MB | Stored as blob |
| T-JRNL-004 | `test_journal_rollback_restores_file` | Rollback scope | File restored to original |
| T-JRNL-005 | `test_journal_rollback_deletes_created_file` | File created by tool, rollback | File deleted |
| T-JRNL-006 | `test_journal_conflict_detection` | User modified file after tool | Conflict detected, no overwrite |
| T-JRNL-007 | `test_journal_scope_isolation` | Two workflows | Each rolls back independently |
| T-JRNL-008 | `test_journal_commit_clears_entries` | Commit scope | Entries marked committed |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-JRNL-001 | `FileJournalMiddleware` as ToolMiddleware (pre) | High |
| I-JRNL-002 | Three-tier storage (inline/blob/git-ref) | High |
| I-JRNL-003 | `ScopeRollback` with conflict detection | High |
| I-JRNL-004 | `ConflictDetector` (hash comparison) | Medium |

---

## 4. y-scheduler (Phase 3.3)

**Purpose**: Time-based and event-driven task scheduling.
**Design Reference**: `scheduled-tasks-design.md`
**Depends On**: `y-core`, `y-agent-core` (workflow execution), `y-storage`

### Module Structure

```
y-scheduler/src/
  lib.rs              — Public API: SchedulerManager
  config.rs           — SchedulerConfig
  cron.rs             — CronSchedule: cron expression parsing and next-fire
  interval.rs         — IntervalSchedule: fixed interval firing
  event.rs            — EventSchedule: event-driven triggers
  executor.rs         — ScheduleExecutor: fires workflows at scheduled times
  store.rs            — ScheduleStore: SQLite persistence of definitions and history
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-SCHED-001 | `test_cron_parse_valid_expression` | `"0 9 * * MON"` | Parses correctly |
| T-SCHED-002 | `test_cron_next_fire_time` | Current time + cron | Correct next fire |
| T-SCHED-003 | `test_interval_fires_periodically` | 5-minute interval | Fires at correct times |
| T-SCHED-004 | `test_schedule_enable_disable` | Disable schedule | No more fires |
| T-SCHED-005 | `test_schedule_execution_records_history` | Schedule fires | Execution record persisted |
| T-SCHED-006 | `test_schedule_parameter_binding` | Params with expressions | Resolved at fire time |
| T-SCHED-007 | `test_schedule_skip_if_already_running` | Overlapping fires | Second skipped |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-SCHED-001 | `CronSchedule` with cron expression parsing | High |
| I-SCHED-002 | `IntervalSchedule` | High |
| I-SCHED-003 | `ScheduleExecutor` with workflow integration | High |
| I-SCHED-004 | `ScheduleStore` SQLite persistence | Medium |
| I-SCHED-005 | `EventSchedule` triggers | Low |

---

## 5. y-diagnostics (Phase 5)

**Purpose**: PostgreSQL-backed trace store, cost intelligence, semantic trace search, trace replay.
**Design Reference**: `diagnostics-observability-design.md`
**Depends On**: `y-core`, `y-hooks` (event subscription), `y-storage` (PostgreSQL pool)
**Feature Flag**: `diagnostics_pg`

### Module Structure

```
y-diagnostics/src/
  lib.rs              — Public API: DiagnosticsManager
  trace_store.rs      — PgTraceStore: traces and observations
  cost.rs             — CostIntelligence: cost tracking, daily summaries
  search.rs           — SemanticTraceSearch: full-text + metadata search
  replay.rs           — TraceReplay: reconstruct execution from traces
  subscriber.rs       — DiagnosticsSubscriber: EventSubscriber for auto-capture
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-DIAG-001 | `test_trace_store_create_trace` | Create trace | Stored in PostgreSQL |
| T-DIAG-002 | `test_trace_observation_hierarchy` | Trace → observation → child | Tree structure preserved |
| T-DIAG-003 | `test_cost_tracking_accumulates` | 5 LLM calls | Total cost summed |
| T-DIAG-004 | `test_cost_daily_summary` | Multiple calls across day | Materialized view correct |
| T-DIAG-005 | `test_trace_search_by_status` | Search failed traces | Only failed returned |
| T-DIAG-006 | `test_trace_replay_reconstructs` | Replay trace | Steps recreated in order |
| T-DIAG-007 | `test_subscriber_auto_captures` | Event bus events | Auto-stored as observations |
| T-DIAG-008 | `test_retention_cleanup` | Old traces | Deleted after retention period |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-DIAG-001 | `PgTraceStore` with traces/observations | High |
| I-DIAG-002 | `CostIntelligence` tracking and summaries | Medium |
| I-DIAG-003 | `DiagnosticsSubscriber` auto-capture from events | Medium |
| I-DIAG-004 | `SemanticTraceSearch` | Low |
| I-DIAG-005 | `TraceReplay` | Low |

---

## 6. y-test-utils (Phase 1)

**Purpose**: Shared test infrastructure — mock implementations, fixture factories, assertion helpers.
**Depends On**: `y-core` (traits to mock)
**Dev-only**: Never compiled into release builds.

### Module Structure

```
y-test-utils/src/
  lib.rs              — Re-exports all test utilities
  mock_provider.rs    — MockLlmProvider: configurable canned responses
  mock_runtime.rs     — MockRuntimeAdapter: fake execution results
  mock_storage.rs     — MockCheckpointStorage, MockSessionStore (HashMap-backed)
  mock_memory.rs      — MockMemoryClient: in-memory Vec with brute-force search
  mock_tools.rs       — MockToolRegistry, MockTool
  fixtures.rs         — Factory functions: sample_chat_response, sample_tool_definition, etc.
  assert_helpers.rs   — Custom assertion macros
```

### Key Tests

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-UTIL-001 | `test_mock_provider_returns_canned` | Configure response, call | Returns configured response |
| T-UTIL-002 | `test_mock_provider_records_calls` | 3 calls | Call history has 3 entries |
| T-UTIL-003 | `test_mock_storage_in_memory` | Write/read checkpoint | HashMap-backed storage works |
| T-UTIL-004 | `test_mock_runtime_returns_fake_result` | Execute command | Returns configured result |
| T-UTIL-005 | `test_fixtures_produce_valid_data` | All factory functions | Pass y-core validation |
| T-UTIL-006 | `test_mock_memory_brute_force_search` | Store 5, search | Returns by keyword match |

### Implementation Tasks

| Task ID | Task | Priority |
|---------|------|----------|
| I-UTIL-001 | `MockLlmProvider` with configurable responses and call recording | High |
| I-UTIL-002 | `MockRuntimeAdapter` with fake results | High |
| I-UTIL-003 | `MockCheckpointStorage` (HashMap) | High |
| I-UTIL-004 | `MockSessionStore` (HashMap) | High |
| I-UTIL-005 | `MockMemoryClient` (Vec) | Medium |
| I-UTIL-006 | Factory functions for all y-core types | High |
| I-UTIL-007 | Custom assertion macros | Low |

---

## Quality Gates (All Supporting Modules)

| Gate | Target | Tool |
|------|--------|------|
| Test coverage per module | >= 70% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test --workspace` |
| Clippy clean | 0 warnings | `cargo clippy --workspace` |
| No cross-peer dependencies | Verified | `cargo deny` |
