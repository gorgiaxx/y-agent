# y-agent Database Schema Design

**Version**: v0.1
**Created**: 2026-03-08
**Status**: Draft

---

## 1. Purpose

This document defines the database schemas for y-agent's three storage backends: SQLite (operational state), PostgreSQL (diagnostics/analytics), and Qdrant (vector store). Each table traces back to the design document that owns it.

---

## 2. Storage Architecture

| Backend | Purpose | Owner | Access Pattern |
|---------|---------|-------|---------------|
| SQLite (WAL) | Operational state: checkpoints, sessions, workflows, file journal, schedules, tools, agents | Multiple crates via `y-storage` | High-frequency read/write, single-node |
| PostgreSQL | Diagnostics: traces, observations, scores, cost records | `y-diagnostics` | Append-heavy, analytical queries |
| Qdrant | Semantic retrieval: long-term memories, knowledge base documents | `y-context` (via Memory Service) | Vector similarity search + payload filtering |

### Dual-Database Rationale

SQLite handles all operational state because it is zero-dependency, embeddable, and sufficient for single-node workloads. PostgreSQL handles diagnostics because those queries benefit from GIN indexes, JSONB, and materialized views. This separation means diagnostics can be disabled (`diagnostics_pg` feature flag off) without affecting core agent operation.

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

### 3.4 File Journal

**Source**: [file-journal-design.md](../design/file-journal-design.md)

```sql
-- File mutation journal for rollback
CREATE TABLE file_journal_entries (
    id              TEXT PRIMARY KEY,           -- UUID
    scope_id        TEXT NOT NULL,              -- Workflow or task scope
    scope_type      TEXT NOT NULL CHECK (scope_type IN ('workflow', 'task', 'step')),
    file_path       TEXT NOT NULL,
    storage_tier    TEXT NOT NULL CHECK (storage_tier IN ('inline', 'blob', 'git_ref')),
    original_hash   TEXT NOT NULL,              -- SHA-256 of original content
    original_size   INTEGER NOT NULL,
    inline_content  BLOB,                       -- For inline tier (< 256KB)
    blob_path       TEXT,                       -- For blob tier (256KB-10MB)
    git_ref         TEXT,                       -- For git-ref tier (tracked files)
    file_existed    INTEGER NOT NULL DEFAULT 1, -- 0 if file was created (rollback = delete)
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN (
                        'active', 'rolled_back', 'committed', 'conflict'
                    )),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_journal_scope ON file_journal_entries(scope_id, scope_type);
CREATE INDEX idx_journal_path ON file_journal_entries(file_path);
CREATE INDEX idx_journal_status ON file_journal_entries(status);
```

### 3.5 Tool Registry

**Source**: [tools-design.md](../design/tools-design.md), [agent-autonomy-design.md](../design/agent-autonomy-design.md)

```sql
-- Dynamic tool definitions (agent-created at runtime)
CREATE TABLE tool_dynamic_definitions (
    id              TEXT PRIMARY KEY,           -- UUID
    name            TEXT NOT NULL UNIQUE,
    description     TEXT NOT NULL,
    tool_type       TEXT NOT NULL CHECK (tool_type IN ('script', 'http_api', 'composite')),
    implementation  TEXT NOT NULL,              -- JSON: script source, API spec, or composite steps
    parameters      TEXT NOT NULL,              -- JSON Schema for input parameters
    result_schema   TEXT,                       -- JSON Schema for output
    capabilities    TEXT NOT NULL,              -- JSON: RuntimeCapability requirements
    is_sandboxed    INTEGER NOT NULL DEFAULT 1, -- Always 1 for dynamic tools
    creator_agent   TEXT,                       -- Agent ID that created this tool
    validation_status TEXT NOT NULL DEFAULT 'pending' CHECK (validation_status IN (
                        'pending', 'validated', 'failed', 'disabled'
                    )),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_tool_name ON tool_dynamic_definitions(name);
CREATE INDEX idx_tool_status ON tool_dynamic_definitions(validation_status);

-- Tool activation tracking (for lazy loading analytics)
CREATE TABLE tool_activation_log (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    tool_name       TEXT NOT NULL,
    activated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    activation_source TEXT NOT NULL CHECK (activation_source IN (
                        'ToolSearch', 'always_active', 'dependency', 'user_request'
                    ))
);

CREATE INDEX idx_tool_activation_session ON tool_activation_log(session_id);
```

### 3.6 Scheduled Tasks

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
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Schedule execution history
CREATE TABLE schedule_executions (
    id              TEXT PRIMARY KEY,           -- UUID
    schedule_id     TEXT NOT NULL REFERENCES schedule_definitions(id),
    session_id      TEXT REFERENCES session_metadata(id),
    status          TEXT NOT NULL CHECK (status IN (
                        'triggered', 'running', 'completed', 'failed', 'skipped'
                    )),
    resolved_params TEXT,                       -- JSON: actual parameter values used
    error_message   TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT
);

CREATE INDEX idx_schedule_exec_schedule ON schedule_executions(schedule_id);
CREATE INDEX idx_schedule_exec_status ON schedule_executions(status);
```

### 3.7 Dynamic Agent Definitions

**Source**: [agent-autonomy-design.md](../design/agent-autonomy-design.md)

```sql
-- Agent definitions (both static TOML-loaded and dynamic agent-created)
CREATE TABLE agent_definitions (
    id              TEXT PRIMARY KEY,           -- UUID
    name            TEXT NOT NULL UNIQUE,
    description     TEXT NOT NULL,
    source          TEXT NOT NULL CHECK (source IN ('static', 'dynamic')),
    mode            TEXT NOT NULL CHECK (mode IN ('build', 'plan', 'explore', 'general')),
    definition      TEXT NOT NULL,              -- JSON: full AgentDefinition
    trust_tier      TEXT NOT NULL DEFAULT 'untrusted' CHECK (trust_tier IN (
                        'trusted', 'verified', 'untrusted'
                    )),
    permission_snapshot TEXT,                   -- JSON: frozen permission set at creation
    creator_agent   TEXT,                       -- Agent ID that created this (NULL for static)
    is_active       INTEGER NOT NULL DEFAULT 1, -- Soft delete
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_agent_name ON agent_definitions(name);
CREATE INDEX idx_agent_active ON agent_definitions(is_active);
CREATE INDEX idx_agent_mode ON agent_definitions(mode);
```

### 3.8 Short-Term Memory

**Source**: [memory-short-term-design.md](../design/memory-short-term-design.md)

```sql
-- Experience store entries (session-scoped indexed archival)
CREATE TABLE stm_experience_store (
    id              TEXT PRIMARY KEY,           -- UUID
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    slot_index      INTEGER NOT NULL,           -- Stable index for dereference
    summary         TEXT NOT NULL,              -- Compressed experience summary
    evidence_type   TEXT NOT NULL CHECK (evidence_type IN (
                        'user_stated', 'user_correction', 'task_outcome', 'agent_observation'
                    )),
    skill_id        TEXT,                       -- Associated skill (NULL for skillless experiences)
    token_estimate  INTEGER NOT NULL,           -- Token budget awareness
    metadata        TEXT NOT NULL DEFAULT '{}', -- JSON
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_experience_session ON stm_experience_store(session_id);
CREATE INDEX idx_experience_skill ON stm_experience_store(skill_id);
CREATE UNIQUE INDEX idx_experience_slot ON stm_experience_store(session_id, slot_index);
```

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

## 4. PostgreSQL Schema (Diagnostics)

All diagnostics tables live in the `observability` schema.

**Source**: [diagnostics-observability-design.md](../design/diagnostics-observability-design.md)

### 4.1 Schema Setup

```sql
CREATE SCHEMA IF NOT EXISTS observability;
```

### 4.2 Traces

```sql
CREATE TABLE observability.traces (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      TEXT NOT NULL,
    workflow_id     TEXT,
    name            TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN (
                        'running', 'success', 'failed', 'timeout', 'cancelled'
                    )),
    input           JSONB,
    output          JSONB,
    metadata        JSONB NOT NULL DEFAULT '{}',
    tags            TEXT[] NOT NULL DEFAULT '{}',
    total_tokens    INTEGER NOT NULL DEFAULT 0,
    total_cost      NUMERIC(10, 6) NOT NULL DEFAULT 0,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    duration_ms     INTEGER GENERATED ALWAYS AS (
                        EXTRACT(MILLISECONDS FROM (completed_at - started_at))::INTEGER
                    ) STORED
);

CREATE INDEX idx_traces_session ON observability.traces(session_id);
CREATE INDEX idx_traces_status ON observability.traces(status);
CREATE INDEX idx_traces_started ON observability.traces(started_at);
CREATE INDEX idx_traces_tags ON observability.traces USING GIN(tags);
CREATE INDEX idx_traces_metadata ON observability.traces USING GIN(metadata);
```

### 4.3 Observations

```sql
CREATE TABLE observability.observations (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    trace_id        UUID NOT NULL REFERENCES observability.traces(id) ON DELETE CASCADE,
    parent_id       UUID REFERENCES observability.observations(id),
    path            UUID[] NOT NULL DEFAULT '{}',  -- Materialized path for tree queries
    depth           INTEGER NOT NULL DEFAULT 0,
    observation_type TEXT NOT NULL CHECK (observation_type IN (
                        'span', 'llm_call', 'tool_call', 'mcp_call', 'retrieval',
                        'embedding', 'reranking', 'sub_agent', 'planning',
                        'reflection', 'guardrail', 'hook', 'cache'
                    )),
    name            TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN (
                        'running', 'success', 'failed', 'timeout', 'cancelled'
                    )),
    input           JSONB,
    output          JSONB,
    metadata        JSONB NOT NULL DEFAULT '{}',
    -- LLM-specific fields (NULL for non-LLM observations)
    model           TEXT,
    provider        TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    cost            NUMERIC(10, 6),
    -- Timing
    started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    duration_ms     INTEGER GENERATED ALWAYS AS (
                        EXTRACT(MILLISECONDS FROM (completed_at - started_at))::INTEGER
                    ) STORED
);

CREATE INDEX idx_obs_trace ON observability.observations(trace_id);
CREATE INDEX idx_obs_parent ON observability.observations(parent_id);
CREATE INDEX idx_obs_type ON observability.observations(observation_type);
CREATE INDEX idx_obs_path ON observability.observations USING GIN(path);
CREATE INDEX idx_obs_metadata ON observability.observations USING GIN(metadata);
```

### 4.4 Scores

```sql
CREATE TABLE observability.scores (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    trace_id        UUID NOT NULL REFERENCES observability.traces(id) ON DELETE CASCADE,
    observation_id  UUID REFERENCES observability.observations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    value           NUMERIC NOT NULL,
    source          TEXT NOT NULL CHECK (source IN ('human', 'auto', 'model')),
    comment         TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_scores_trace ON observability.scores(trace_id);
CREATE INDEX idx_scores_observation ON observability.scores(observation_id);
CREATE INDEX idx_scores_name ON observability.scores(name);
```

### 4.5 Cost Records

```sql
CREATE TABLE observability.cost_records (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    trace_id        UUID NOT NULL REFERENCES observability.traces(id) ON DELETE CASCADE,
    observation_id  UUID REFERENCES observability.observations(id) ON DELETE CASCADE,
    provider        TEXT NOT NULL,
    model           TEXT NOT NULL,
    input_tokens    INTEGER NOT NULL,
    output_tokens   INTEGER NOT NULL,
    cost_usd        NUMERIC(10, 6) NOT NULL,
    pricing_version TEXT NOT NULL,              -- Tracks which pricing was applied
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_cost_trace ON observability.cost_records(trace_id);
CREATE INDEX idx_cost_provider ON observability.cost_records(provider);
CREATE INDEX idx_cost_recorded ON observability.cost_records(recorded_at);

-- Materialized view for cost aggregation
CREATE MATERIALIZED VIEW observability.daily_cost_summary AS
SELECT
    DATE(recorded_at) AS day,
    provider,
    model,
    SUM(input_tokens) AS total_input_tokens,
    SUM(output_tokens) AS total_output_tokens,
    SUM(cost_usd) AS total_cost_usd,
    COUNT(*) AS call_count
FROM observability.cost_records
GROUP BY DATE(recorded_at), provider, model;

CREATE UNIQUE INDEX idx_daily_cost_key
ON observability.daily_cost_summary(day, provider, model);
```

### 4.6 Retention Policy

```sql
-- Automated cleanup for old diagnostics data
-- Run periodically (e.g., daily via pg_cron or application scheduler)

-- Default retention: 30 days for traces, 7 days for detailed observations
DELETE FROM observability.traces
WHERE completed_at < NOW() - INTERVAL '30 days'
  AND status != 'running';

-- Refresh materialized view after cleanup
REFRESH MATERIALIZED VIEW CONCURRENTLY observability.daily_cost_summary;
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

## 6. Migration Strategy

### 6.1 Tool

`sqlx-cli` for both SQLite and PostgreSQL migrations.

```bash
# Install
cargo install sqlx-cli --features sqlite,postgres

# Create migration
sqlx migrate add -r {description}

# Run migrations
sqlx migrate run --database-url "sqlite:y-agent.db"
sqlx migrate run --database-url "postgres://..."
```

### 6.2 Directory Structure

```
migrations/
  sqlite/
    001_initial_sessions.up.sql
    001_initial_sessions.down.sql
    002_orchestrator_checkpoints.up.sql
    002_orchestrator_checkpoints.down.sql
    003_file_journal.up.sql
    003_file_journal.down.sql
    004_tools_and_agents.up.sql
    004_tools_and_agents.down.sql
    005_schedules.up.sql
    005_schedules.down.sql
    006_stm_experience.up.sql
    006_stm_experience.down.sql
  postgres/
    001_observability_schema.up.sql
    001_observability_schema.down.sql
    002_cost_records.up.sql
    002_cost_records.down.sql
    003_materialized_views.up.sql
    003_materialized_views.down.sql
```

### 6.3 Rules

- Every `up` migration has a corresponding `down` migration
- Migrations are append-only (never modify an existing migration file)
- Each migration is idempotent (`IF NOT EXISTS` where applicable)
- Large data migrations run outside of DDL transactions
- Schema changes require review regardless of risk tier

---

## 7. Data Integrity Constraints

### 7.1 Referential Integrity

- SQLite foreign keys enabled via pragma (Section 3.1)
- PostgreSQL cascading deletes for trace -> observation -> score hierarchy
- No cross-database references (SQLite tables do not reference PostgreSQL)

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
- PostgreSQL: standard `pg_dump` or streaming replication
- Qdrant: collection snapshots via REST API
- JSONL transcripts: included in filesystem backup

