-- y-agent consolidated SQLite schema
-- All tables required for fresh installation.
-- This file is embedded into the binary via include_str!().
--
-- Naming conventions:
--   - Tables: snake_case
--   - Timestamps: ISO 8601 TEXT via strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
--   - JSON columns: stored as TEXT
--   - Boolean columns: INTEGER (0/1)

------------------------------------------------------------------------
-- 1. Session tree metadata
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS session_metadata (
    id              TEXT PRIMARY KEY,
    parent_id       TEXT REFERENCES session_metadata(id),
    root_id         TEXT NOT NULL REFERENCES session_metadata(id),
    depth           INTEGER NOT NULL DEFAULT 0,
    path            TEXT NOT NULL,
    session_type    TEXT NOT NULL CHECK (session_type IN (
                        'main', 'child', 'branch', 'ephemeral', 'canonical'
                    )),
    state           TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
                        'active', 'paused', 'archived', 'merged', 'tombstone'
                    )),
    agent_id        TEXT,
    title           TEXT,
    token_count     INTEGER NOT NULL DEFAULT 0,
    message_count   INTEGER NOT NULL DEFAULT 0,
    transcript_path TEXT NOT NULL,
    channel         TEXT,
    label           TEXT,
    last_compaction TEXT,
    compaction_count INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_session_parent ON session_metadata(parent_id);
CREATE INDEX IF NOT EXISTS idx_session_root   ON session_metadata(root_id);
CREATE INDEX IF NOT EXISTS idx_session_state  ON session_metadata(state);
CREATE INDEX IF NOT EXISTS idx_session_agent  ON session_metadata(agent_id);

------------------------------------------------------------------------
-- 2. Orchestrator workflows and checkpoints
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS orchestrator_workflows (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    definition      TEXT NOT NULL,
    compiled_dag    TEXT NOT NULL,
    parameter_schema TEXT,
    tags            TEXT NOT NULL DEFAULT '[]',
    creator         TEXT NOT NULL CHECK (creator IN ('user', 'agent')),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_workflow_name    ON orchestrator_workflows(name);
CREATE INDEX IF NOT EXISTS idx_workflow_creator ON orchestrator_workflows(creator);

CREATE TABLE IF NOT EXISTS orchestrator_checkpoints (
    id              TEXT PRIMARY KEY,
    workflow_id     TEXT NOT NULL REFERENCES orchestrator_workflows(id),
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    step_number     INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN (
                        'running', 'completed', 'failed', 'interrupted', 'compensating'
                    )),
    committed_state TEXT NOT NULL,
    pending_state   TEXT,
    interrupt_data  TEXT,
    versions_seen   TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_checkpoint_workflow ON orchestrator_checkpoints(workflow_id);
CREATE INDEX IF NOT EXISTS idx_checkpoint_session  ON orchestrator_checkpoints(session_id);
CREATE INDEX IF NOT EXISTS idx_checkpoint_status   ON orchestrator_checkpoints(status);

------------------------------------------------------------------------
-- 3. File mutation journal
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS file_journal_entries (
    id              TEXT PRIMARY KEY,
    scope_id        TEXT NOT NULL,
    scope_type      TEXT NOT NULL CHECK (scope_type IN ('workflow', 'task', 'step')),
    file_path       TEXT NOT NULL,
    storage_tier    TEXT NOT NULL CHECK (storage_tier IN ('inline', 'blob', 'git_ref')),
    original_hash   TEXT NOT NULL,
    original_size   INTEGER NOT NULL,
    inline_content  BLOB,
    blob_path       TEXT,
    git_ref         TEXT,
    file_existed    INTEGER NOT NULL DEFAULT 1,
    status          TEXT NOT NULL DEFAULT 'active' CHECK (status IN (
                        'active', 'rolled_back', 'committed', 'conflict'
                    )),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_journal_scope  ON file_journal_entries(scope_id, scope_type);
CREATE INDEX IF NOT EXISTS idx_journal_path   ON file_journal_entries(file_path);
CREATE INDEX IF NOT EXISTS idx_journal_status ON file_journal_entries(status);

------------------------------------------------------------------------
-- 4. Dynamic tool definitions, activation log, agent definitions
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS tool_dynamic_definitions (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT NOT NULL,
    tool_type       TEXT NOT NULL CHECK (tool_type IN ('script', 'http_api', 'composite')),
    implementation  TEXT NOT NULL,
    parameters      TEXT NOT NULL,
    result_schema   TEXT,
    capabilities    TEXT NOT NULL,
    is_sandboxed    INTEGER NOT NULL DEFAULT 1,
    creator_agent   TEXT,
    validation_status TEXT NOT NULL DEFAULT 'pending' CHECK (validation_status IN (
                        'pending', 'validated', 'failed', 'disabled'
                    )),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_tool_name   ON tool_dynamic_definitions(name);
CREATE INDEX IF NOT EXISTS idx_tool_status ON tool_dynamic_definitions(validation_status);

CREATE TABLE IF NOT EXISTS tool_activation_log (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    tool_name       TEXT NOT NULL,
    activated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    activation_source TEXT NOT NULL CHECK (activation_source IN (
                        'tool_search', 'always_active', 'dependency', 'user_request'
                    ))
);

CREATE INDEX IF NOT EXISTS idx_tool_activation_session ON tool_activation_log(session_id);

CREATE TABLE IF NOT EXISTS agent_definitions (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT NOT NULL,
    source          TEXT NOT NULL CHECK (source IN ('static', 'dynamic')),
    mode            TEXT NOT NULL CHECK (mode IN ('build', 'plan', 'explore', 'general')),
    definition      TEXT NOT NULL,
    trust_tier      TEXT NOT NULL DEFAULT 'untrusted' CHECK (trust_tier IN (
                        'trusted', 'verified', 'untrusted'
                    )),
    permission_snapshot TEXT,
    creator_agent   TEXT,
    is_active       INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_name   ON agent_definitions(name);
CREATE INDEX IF NOT EXISTS idx_agent_active ON agent_definitions(is_active);
CREATE INDEX IF NOT EXISTS idx_agent_mode   ON agent_definitions(mode);

------------------------------------------------------------------------
-- 5. Schedules (with policy columns baked in)
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS schedule_definitions (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    schedule_type   TEXT NOT NULL CHECK (schedule_type IN (
                        'cron', 'interval', 'event', 'onetime'
                    )),
    schedule_expr   TEXT NOT NULL,
    workflow_id     TEXT NOT NULL REFERENCES orchestrator_workflows(id),
    parameter_bindings TEXT,
    parameter_schema TEXT,
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

CREATE INDEX IF NOT EXISTS idx_schedule_def_enabled ON schedule_definitions(enabled);

CREATE TABLE IF NOT EXISTS schedule_executions (
    id              TEXT PRIMARY KEY,
    schedule_id     TEXT NOT NULL REFERENCES schedule_definitions(id),
    session_id      TEXT REFERENCES session_metadata(id),
    status          TEXT NOT NULL CHECK (status IN (
                        'triggered', 'running', 'completed', 'failed', 'skipped'
                    )),
    resolved_params TEXT,
    error_message   TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_schedule_exec_schedule ON schedule_executions(schedule_id);
CREATE INDEX IF NOT EXISTS idx_schedule_exec_status   ON schedule_executions(status);

------------------------------------------------------------------------
-- 6. Short-term memory experience store
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS stm_experience_store (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES session_metadata(id),
    slot_index      INTEGER NOT NULL,
    summary         TEXT NOT NULL,
    evidence_type   TEXT NOT NULL CHECK (evidence_type IN (
                        'user_stated', 'user_correction', 'task_outcome', 'agent_observation'
                    )),
    skill_id        TEXT,
    token_estimate  INTEGER NOT NULL,
    metadata        TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_experience_session ON stm_experience_store(session_id);
CREATE INDEX IF NOT EXISTS idx_experience_skill   ON stm_experience_store(skill_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_experience_slot ON stm_experience_store(session_id, slot_index);

------------------------------------------------------------------------
-- 7. Dynamic agents
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS dynamic_agents (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    trust_tier      TEXT NOT NULL DEFAULT 'dynamic',
    delegation_depth INTEGER NOT NULL DEFAULT 0,
    version         INTEGER NOT NULL DEFAULT 1,
    status          TEXT NOT NULL DEFAULT 'active',
    effective_permissions_json TEXT,
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    deactivated_at  TEXT,
    deactivation_reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_dynamic_agents_status     ON dynamic_agents(status);
CREATE INDEX IF NOT EXISTS idx_dynamic_agents_trust_tier ON dynamic_agents(trust_tier);
CREATE INDEX IF NOT EXISTS idx_dynamic_agents_created_by ON dynamic_agents(created_by);
CREATE INDEX IF NOT EXISTS idx_dynamic_agents_name       ON dynamic_agents(name);

------------------------------------------------------------------------
-- 8. Chat checkpoints (turn-level rollback)
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS chat_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL,
    turn_number   INTEGER NOT NULL,
    message_count_before INTEGER NOT NULL,
    journal_scope_id TEXT NOT NULL,
    invalidated   INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CONSTRAINT unique_session_turn UNIQUE (session_id, turn_number)
);

CREATE INDEX IF NOT EXISTS idx_chat_cp_session
    ON chat_checkpoints(session_id, turn_number DESC);

------------------------------------------------------------------------
-- 9. Chat messages
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS chat_messages (
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

CREATE INDEX IF NOT EXISTS idx_cm_session_status  ON chat_messages(session_id, status);
CREATE INDEX IF NOT EXISTS idx_cm_session_created ON chat_messages(session_id, created_at);

------------------------------------------------------------------------
-- 10. Diagnostics (traces, observations, scores -- v2 columns baked in)
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS diag_traces (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active',
    user_input  TEXT,
    metadata    TEXT NOT NULL DEFAULT 'null',
    tags        TEXT NOT NULL DEFAULT '[]',
    replay_context TEXT,
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    total_cost_usd      REAL NOT NULL DEFAULT 0.0,
    llm_duration_ms     INTEGER NOT NULL DEFAULT 0,
    tool_duration_ms    INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_diag_traces_session
    ON diag_traces(session_id, started_at DESC);

CREATE TABLE IF NOT EXISTS diag_observations (
    id          TEXT PRIMARY KEY,
    trace_id    TEXT NOT NULL REFERENCES diag_traces(id) ON DELETE CASCADE,
    parent_id   TEXT,
    session_id  TEXT,
    obs_type    TEXT NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'running',
    model       TEXT,
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd      REAL NOT NULL DEFAULT 0.0,
    input         TEXT NOT NULL DEFAULT 'null',
    output        TEXT NOT NULL DEFAULT 'null',
    metadata      TEXT NOT NULL DEFAULT 'null',
    sequence      INTEGER NOT NULL DEFAULT 0,
    depth         INTEGER NOT NULL DEFAULT 0,
    path          TEXT NOT NULL DEFAULT '[]',
    error_message TEXT,
    started_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_diag_obs_trace
    ON diag_observations(trace_id, sequence ASC);

CREATE TABLE IF NOT EXISTS diag_scores (
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

CREATE INDEX IF NOT EXISTS idx_diag_scores_trace
    ON diag_scores(trace_id);

CREATE INDEX IF NOT EXISTS idx_diag_scores_obs
    ON diag_scores(observation_id);
