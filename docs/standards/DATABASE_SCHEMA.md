# y-agent Database Schema Design

**Version**: v0.1
**Created**: 2026-03-08
**Status**: Draft

---

## 1. Purpose

This document defines the database schemas for y-agent's active storage backends: SQLite (operational + diagnostics state) and Qdrant (vector store). Each table traces back to the design document that owns it.

---

## 2. Storage Architecture

| Backend | Purpose | Owner | Access Pattern |
|---------|---------|-------|---------------|
| SQLite (WAL) | Operational + diagnostics state: checkpoints, sessions, workflows, schedules, chat history, traces, provider metrics | Multiple crates via `y-storage` | High-frequency read/write, single-node |
| Qdrant | Semantic retrieval: long-term memories, knowledge base documents | `y-context` (via Memory Service) | Vector similarity search + payload filtering |

### Embedded-Schema Rationale

SQLite handles both operational state and diagnostics because the current deployment model is single-node, zero-dependency, and startup-controlled. Schema compatibility is enforced in application code: incompatible databases are archived and recreated before the normal connection pool is opened.

---

## 3. SQLite Schema (Operational)

All SQLite tables live in a single database file (`y-agent.db`) with WAL mode enabled.

### 3.1 Pragmas (Connection Setup)

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -64000;  -- 64MB
```

### 3.2 Session Management

**Source**: [context-session-design.md](../design/context-session-design.md)

```sql
-- Session tree metadata
CREATE TABLE session_metadata (
    id              TEXT PRIMARY KEY,           -- UUID
    parent_id       TEXT REFERENCES session_metadata(id),
    root_id         TEXT NOT NULL REFERENCES session_metadata(id),
    depth           INTEGER NOT NULL DEFAULT 0,
    path            TEXT NOT NULL,              -- JSON array of ancestor IDs
    session_type    TEXT NOT NULL CHECK (session_type IN (
                        'main', 'child', 'branch', 'ephemeral', 'canonical'
                    )),
    state           TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
                        'active', 'paused', 'archived', 'merged', 'tombstone'
                    )),
    agent_id        TEXT,                       -- Agent that owns this session
    title           TEXT,
    token_count     INTEGER NOT NULL DEFAULT 0,
    message_count   INTEGER NOT NULL DEFAULT 0,
    transcript_path TEXT NOT NULL,              -- Path to JSONL transcript file
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_session_parent ON session_metadata(parent_id);
CREATE INDEX idx_session_root ON session_metadata(root_id);
CREATE INDEX idx_session_state ON session_metadata(state);
CREATE INDEX idx_session_agent ON session_metadata(agent_id);
```

### 3.3 Orchestrator Checkpoints

**Source**: [orchestrator-design.md](../design/orchestrator-design.md)

```sql
-- Workflow definitions (agent-created or user-defined)
CREATE TABLE orchestrator_workflows (
    id              TEXT PRIMARY KEY,           -- UUID
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    definition      TEXT NOT NULL,              -- TOML or Expression DSL source
    compiled_dag    TEXT NOT NULL,              -- JSON serialized DAG
    parameter_schema TEXT,                      -- JSON Schema for parameters
    tags            TEXT NOT NULL DEFAULT '[]', -- JSON array of tags
    creator         TEXT NOT NULL CHECK (creator IN ('user', 'agent')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_workflow_name ON orchestrator_workflows(name);
CREATE INDEX idx_workflow_creator ON orchestrator_workflows(creator);

-- Checkpoint state per workflow execution
CREATE TABLE orchestrator_checkpoints (
    id              TEXT PRIMARY KEY,           -- UUID
    workflow_id     TEXT NOT NULL REFERENCES orchestrator_workflows(id),
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    step_number     INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN (
                        'running', 'completed', 'failed', 'interrupted', 'compensating'
                    )),
    committed_state TEXT NOT NULL,              -- JSON: committed channels + task outputs
    pending_state   TEXT,                       -- JSON: uncommitted writes (NULL when committed)
    interrupt_data  TEXT,                       -- JSON: interrupt metadata (if status = interrupted)
    versions_seen   TEXT NOT NULL DEFAULT '{}', -- JSON: task_id -> version for stale detection
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_checkpoint_workflow ON orchestrator_checkpoints(workflow_id);
CREATE INDEX idx_checkpoint_session ON orchestrator_checkpoints(session_id);
CREATE INDEX idx_checkpoint_status ON orchestrator_checkpoints(status);
```

### 3.4 Scheduled Tasks

**Source**: [scheduled-tasks-design.md](../design/scheduled-tasks-design.md)

```sql
-- Schedule definitions
CREATE TABLE schedule_definitions (
    id              TEXT PRIMARY KEY,           -- UUID
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN ('cron', 'interval', 'event')),
    schedule_expr   TEXT NOT NULL,              -- Cron expression, interval duration, or event filter
    workflow_id     TEXT NOT NULL REFERENCES orchestrator_workflows(id),
    parameter_bindings TEXT,                    -- JSON: parameter name -> value/expression
    parameter_schema TEXT,                      -- JSON Schema (from workflow)
    enabled         INTEGER NOT NULL DEFAULT 1,
    creator         TEXT NOT NULL CHECK (creator IN ('user', 'agent')),
    missed_policy   TEXT NOT NULL DEFAULT 'skip'
                        CHECK (missed_policy IN ('skip', 'catch_up', 'backfill')),
    concurrency_policy TEXT NOT NULL DEFAULT 'skip'
                        CHECK (concurrency_policy IN ('skip', 'queue', 'replace')),
    max_executions_per_hour INTEGER NOT NULL DEFAULT 0,
    tags            TEXT NOT NULL DEFAULT '[]',
    last_fire       TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_schedule_def_enabled ON schedule_definitions(enabled);

-- Schedule execution history
CREATE TABLE schedule_executions (
    id              TEXT PRIMARY KEY,           -- UUID
    schedule_id     TEXT NOT NULL REFERENCES schedule_definitions(id),
    session_id      TEXT REFERENCES session_metadata(id),
    status          TEXT NOT NULL CHECK (status IN (
                        'pending', 'triggered', 'running', 'completed', 'failed', 'skipped'
                    )),
    triggered_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    workflow_execution_id TEXT,                 -- Linked workflow run ID when available
    request_summary TEXT,                       -- JSON: trigger/workflow/request context
    response_summary TEXT,                      -- JSON: output/summary/duration context
    resolved_params TEXT,                       -- JSON: actual parameter values used
    error_message   TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT
);

CREATE INDEX idx_schedule_exec_schedule ON schedule_executions(schedule_id);
CREATE INDEX idx_schedule_exec_status ON schedule_executions(status);
```

### 3.5 Removed Legacy / Planned Tables

The active runtime schema no longer creates these tables because there are no
concrete read/write implementations wired into the application:

- `file_journal_entries`
- `tool_dynamic_definitions`
- `tool_activation_log`
- `agent_definitions`
- `stm_experience_store`
- `dynamic_agents`

If these capabilities return, they should be reintroduced together with their
own concrete store implementations and compatibility handling.

### 3.9 Chat Messages (Session History Tree)

**Source**: [CHAT_BUBBLE_ACTIONS_PLAN.md](../plan/CHAT_BUBBLE_ACTIONS_PLAN.md) (Phase 2)

```sql
-- Chat messages with soft-delete (tombstone) support for branch recovery.
CREATE TABLE chat_messages (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'tombstone')),
    checkpoint_id   TEXT REFERENCES chat_checkpoints(checkpoint_id),
    model           TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    cost_usd        REAL,
    context_window  INTEGER,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_cm_session_status ON chat_messages(session_id, status);
CREATE INDEX idx_cm_session_created ON chat_messages(session_id, created_at);
```

---

## 4. SQLite Diagnostics Schema

**Source**: [diagnostics-observability-design.md](../design/diagnostics-observability-design.md)

All diagnostics tables live in the shared `SQLite` database file.

### 4.1 Traces

```sql
CREATE TABLE diag_traces (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    name            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active',
    user_input      TEXT,
    metadata        TEXT NOT NULL DEFAULT 'null',
    tags            TEXT NOT NULL DEFAULT '[]',
    replay_context  TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT,
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost_usd      REAL NOT NULL DEFAULT 0.0,
    llm_duration_ms     INTEGER NOT NULL DEFAULT 0,
    tool_duration_ms    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_diag_traces_session
ON diag_traces(session_id, started_at DESC);
```

### 4.2 Observations

```sql
CREATE TABLE diag_observations (
    id              TEXT PRIMARY KEY,
    trace_id        TEXT NOT NULL REFERENCES diag_traces(id) ON DELETE CASCADE,
    parent_id       TEXT,
    session_id      TEXT,
    obs_type        TEXT NOT NULL,
    name            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running',
    model           TEXT,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cost_usd        REAL NOT NULL DEFAULT 0.0,
    input           TEXT NOT NULL DEFAULT 'null',
    output          TEXT NOT NULL DEFAULT 'null',
    metadata        TEXT NOT NULL DEFAULT 'null',
    sequence        INTEGER NOT NULL DEFAULT 0,
    depth           INTEGER NOT NULL DEFAULT 0,
    path            TEXT NOT NULL DEFAULT '[]',
    error_message   TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT
);

CREATE INDEX idx_diag_obs_trace
ON diag_observations(trace_id, sequence ASC);
```

### 4.3 Scores

```sql
CREATE TABLE diag_scores (
    id              TEXT PRIMARY KEY,
    trace_id        TEXT NOT NULL REFERENCES diag_traces(id) ON DELETE CASCADE,
    observation_id  TEXT REFERENCES diag_observations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    value           REAL NOT NULL DEFAULT 0.0,
    data_type       TEXT NOT NULL DEFAULT 'numeric',
    string_value    TEXT,
    comment         TEXT,
    source          TEXT NOT NULL DEFAULT 'system',
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_diag_scores_trace ON diag_scores(trace_id);
CREATE INDEX idx_diag_scores_obs ON diag_scores(observation_id);
```

### 4.4 Provider Metrics

```sql
CREATE TABLE provider_metrics_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id     TEXT NOT NULL,
    model           TEXT NOT NULL,
    event_type      TEXT NOT NULL CHECK (event_type IN ('success', 'error')),
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cost_micros     INTEGER NOT NULL DEFAULT 0,
    recorded_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_pml_provider_time
ON provider_metrics_log(provider_id, recorded_at DESC);
CREATE INDEX idx_pml_recorded_at
ON provider_metrics_log(recorded_at);
```

### 4.5 Retention Policy

```sql
-- Application-managed cleanup for old diagnostics data
DELETE FROM diag_traces
WHERE completed_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days')
  AND status != 'active';

DELETE FROM provider_metrics_log
WHERE recorded_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-30 days');
```

---

## 5. Qdrant Collections (Vector Store)

**Source**: [memory-architecture-design.md](../design/memory-architecture-design.md), [knowledge-base-design.md](../design/knowledge-base-design.md)

### 5.1 Long-Term Memory Collection

```json
{
  "collection_name": "ltm_memories",
  "vectors": {
    "size": 1536,
    "distance": "Cosine"
  },
  "hnsw_config": {
    "m": 16,
    "ef_construct": 100
  },
  "payload_schema": {
    "memory_type": { "type": "keyword", "indexed": true },
    "scope": { "type": "keyword", "indexed": true },
    "importance": { "type": "float", "indexed": true },
    "created_at": { "type": "datetime", "indexed": true },
    "access_count": { "type": "integer", "indexed": true }
  }
}
```

Payload fields:
- `memory_type`: "personal" | "task" | "tool" | "experience"
- `scope`: workspace or project identifier
- `importance`: 0.0-1.0 with time decay
- `when_to_use`: embedding target (not stored in payload, used for vector)
- `content`: full memory content (stored in payload)
- `metadata`: arbitrary JSON (stored in payload)

### 5.2 Knowledge Base Collection

```json
{
  "collection_name": "kb_documents",
  "vectors": {
    "size": 1536,
    "distance": "Cosine"
  },
  "hnsw_config": {
    "m": 16,
    "ef_construct": 100
  },
  "payload_schema": {
    "domain": { "type": "keyword", "indexed": true },
    "source_type": { "type": "keyword", "indexed": true },
    "source_url": { "type": "keyword", "indexed": false },
    "freshness": { "type": "datetime", "indexed": true },
    "chunk_index": { "type": "integer", "indexed": false }
  }
}
```

Payload fields:
- `domain`: domain classification (e.g., "rust", "docker", "llm")
- `source_type`: "pdf" | "web" | "api" | "manual"
- `source_url`: original source location
- `content`: chunk content
- `freshness`: when the source was last verified/updated
- `chunk_index`: position within the source document

### 5.3 Vector Dimension Note

The vector dimension (1536) assumes OpenAI `text-embedding-3-small`. This is configurable per collection. When using other embedding models, create a separate collection with the appropriate dimension. The `MemoryClient` trait abstracts this.

---

## 6. Schema Application Strategy

### 6.1 Runtime Source

The runtime schema is embedded directly in the binary:

```bash
# Embedded SQLite schema
crates/y-storage/src/schema.sql

# Compatibility guard / reset logic
crates/y-storage/src/migration.rs
```

### 6.2 Compatibility Rules

- `PRAGMA user_version` stores the active schema version
- Startup validates required tables and columns before opening the normal pool
- Legacy sqlx-migration databases are archived and recreated automatically
- Schema initialization remains idempotent via `CREATE TABLE IF NOT EXISTS`
- Schema changes require review regardless of risk tier

---

## 7. Data Integrity Constraints

### 7.1 Referential Integrity

- SQLite foreign keys enabled via pragma (Section 3.1)
- SQLite cascading deletes for trace -> observation -> score hierarchy
- No cross-backend references (SQLite tables do not reference Qdrant)

### 7.2 Consistency Guarantees

| Operation | Guarantee | Mechanism |
|-----------|-----------|-----------|
| Checkpoint commit | Atomic | SQLite transaction wrapping committed/pending swap |
| File journal capture | Best-effort | Fail-open: journal failure does not block tool execution |
| File journal rollback | Conflict-aware | Fail-safe: never silently overwrites user-modified files |
| Diagnostics flush | Eventual | Buffered writes with periodic flush (< 50ms P95) |
| Session transcript | Append-only | JSONL append with fsync on close |

### 7.3 Backup Strategy

- SQLite: periodic `.backup` command to snapshot (non-blocking with WAL)
- Qdrant: collection snapshots via REST API
- JSONL transcripts: included in filesystem backup
