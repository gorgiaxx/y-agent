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
                        'main', 'child', 'branch', 'ephemeral', 'sub_agent', 'canonical'
                    )),
    state           TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
                        'active', 'paused', 'archived', 'merged', 'tombstone'
                    )),
    agent_id        TEXT,
    title           TEXT,
    manual_title    TEXT,
    token_count     INTEGER NOT NULL DEFAULT 0,
    message_count   INTEGER NOT NULL DEFAULT 0,
    transcript_path TEXT NOT NULL,
    channel         TEXT,
    label           TEXT,
    last_compaction TEXT,
    compaction_count INTEGER NOT NULL DEFAULT 0,
    context_reset_index INTEGER,
    custom_system_prompt TEXT,
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
-- 3. Schedules (with policy columns baked in)
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
                        'pending', 'triggered', 'running', 'completed', 'failed', 'skipped'
                    )),
    triggered_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    workflow_execution_id TEXT,
    request_summary TEXT,
    response_summary TEXT,
    resolved_params TEXT,
    error_message   TEXT,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_schedule_exec_schedule ON schedule_executions(schedule_id);
CREATE INDEX IF NOT EXISTS idx_schedule_exec_status   ON schedule_executions(status);

------------------------------------------------------------------------
-- 4. Chat checkpoints (turn-level rollback)
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
-- 5. Chat messages
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS chat_messages (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
    content         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'tombstone', 'pruned')),
    checkpoint_id   TEXT REFERENCES chat_checkpoints(checkpoint_id),
    model           TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    cost_usd        REAL,
    context_window  INTEGER,
    parent_message_id TEXT,
    pruning_group_id TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_cm_session_status  ON chat_messages(session_id, status);
CREATE INDEX IF NOT EXISTS idx_cm_session_created ON chat_messages(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_cm_parent          ON chat_messages(parent_message_id);
CREATE INDEX IF NOT EXISTS idx_cm_pruning_group   ON chat_messages(session_id, pruning_group_id);

------------------------------------------------------------------------
-- 6. Diagnostics (traces, observations, scores -- v2 columns baked in)
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

------------------------------------------------------------------------
-- 7. Provider metrics event log (observability persistence)
------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS provider_metrics_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id     TEXT NOT NULL,
    model           TEXT NOT NULL,
    event_type      TEXT NOT NULL CHECK (event_type IN ('success', 'error')),
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cost_micros     INTEGER NOT NULL DEFAULT 0,
    recorded_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_pml_provider_time
    ON provider_metrics_log(provider_id, recorded_at DESC);
CREATE INDEX IF NOT EXISTS idx_pml_recorded_at
    ON provider_metrics_log(recorded_at);
