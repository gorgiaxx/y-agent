// SQLite-backed TraceStore implementation for persistent diagnostics storage.
//
// Stores trace and observation records in the shared SQLite database so that
// diagnostic data survives application restarts.  The schema is managed by
// migration 011_diagnostics.
//
// All queries use `sqlx::query_as` / `sqlx::query` (no compile-time
// verification) to avoid requiring DATABASE_URL during builds.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::trace_store::{TraceStore, TraceStoreError};
use crate::types::{
    Observation, ObservationStatus, ObservationType, Score, Trace, TraceStatus,
};

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

fn storage_err(msg: impl ToString) -> TraceStoreError {
    TraceStoreError::Storage { message: msg.to_string() }
}

// ---------------------------------------------------------------------------
// Enum <-> &str helpers
// ---------------------------------------------------------------------------

fn trace_status_to_str(s: TraceStatus) -> &'static str {
    match s {
        TraceStatus::Active => "active",
        TraceStatus::Completed => "completed",
        TraceStatus::Failed => "failed",
        TraceStatus::Cancelled => "cancelled",
    }
}

fn str_to_trace_status(s: &str) -> TraceStatus {
    match s {
        "completed" => TraceStatus::Completed,
        "failed" => TraceStatus::Failed,
        "cancelled" => TraceStatus::Cancelled,
        _ => TraceStatus::Active,
    }
}

fn obs_type_to_str(t: ObservationType) -> &'static str {
    match t {
        ObservationType::Generation => "generation",
        ObservationType::ToolCall => "tool_call",
        ObservationType::Span => "span",
        ObservationType::UserInput => "user_input",
        ObservationType::McpCall => "mcp_call",
        ObservationType::Retrieval => "retrieval",
        ObservationType::Embedding => "embedding",
        ObservationType::Reranking => "reranking",
        ObservationType::SubAgent => "sub_agent",
        ObservationType::Planning => "planning",
        ObservationType::Reflection => "reflection",
        ObservationType::Guardrail => "guardrail",
        ObservationType::Hook => "hook",
        ObservationType::Cache => "cache",
    }
}

fn str_to_obs_type(s: &str) -> ObservationType {
    match s {
        "tool_call" => ObservationType::ToolCall,
        "span" => ObservationType::Span,
        "user_input" => ObservationType::UserInput,
        "mcp_call" => ObservationType::McpCall,
        "retrieval" => ObservationType::Retrieval,
        "embedding" => ObservationType::Embedding,
        "reranking" => ObservationType::Reranking,
        "sub_agent" => ObservationType::SubAgent,
        "planning" => ObservationType::Planning,
        "reflection" => ObservationType::Reflection,
        "guardrail" => ObservationType::Guardrail,
        "hook" => ObservationType::Hook,
        "cache" => ObservationType::Cache,
        _ => ObservationType::Generation,
    }
}

fn obs_status_to_str(s: ObservationStatus) -> &'static str {
    match s {
        ObservationStatus::Running => "running",
        ObservationStatus::Completed => "completed",
        ObservationStatus::Failed => "failed",
    }
}

fn str_to_obs_status(s: &str) -> ObservationStatus {
    match s {
        "completed" => ObservationStatus::Completed,
        "failed" => ObservationStatus::Failed,
        _ => ObservationStatus::Running,
    }
}

// ---------------------------------------------------------------------------
// Row types for sqlx::FromRow
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct TraceRow {
    id: String,
    session_id: String,
    name: String,
    status: Option<String>,
    user_input: Option<String>,
    metadata: Option<String>,
    started_at: String,
    completed_at: Option<String>,
    total_input_tokens: Option<i64>,
    total_output_tokens: Option<i64>,
    total_cost_usd: Option<f64>,
    llm_duration_ms: Option<i64>,
    tool_duration_ms: Option<i64>,
}

impl TraceRow {
    fn into_trace(self) -> Option<Trace> {
        let id = Uuid::parse_str(&self.id).ok()?;
        let session_id = Uuid::parse_str(&self.session_id).unwrap_or_default();
        let started_at: DateTime<Utc> = self.started_at.parse().ok()?;
        let completed_at: Option<DateTime<Utc>> =
            self.completed_at.as_deref().and_then(|s| s.parse().ok());
        let metadata: serde_json::Value = self
            .metadata
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        let status = str_to_trace_status(self.status.as_deref().unwrap_or("active"));
        Some(Trace {
            id,
            session_id,
            name: self.name,
            metadata,
            tags: vec![],
            status,
            started_at,
            completed_at,
            total_input_tokens: self.total_input_tokens.unwrap_or(0) as u64,
            total_output_tokens: self.total_output_tokens.unwrap_or(0) as u64,
            total_cost_usd: self.total_cost_usd.unwrap_or(0.0),
            user_input: self.user_input,
            total_duration_ms: completed_at
                .map(|end| (end - started_at).num_milliseconds().max(0) as u64),
            llm_duration_ms: self.llm_duration_ms.unwrap_or(0) as u64,
            tool_duration_ms: self.tool_duration_ms.unwrap_or(0) as u64,
            replay_context: None,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ObsRow {
    id: String,
    trace_id: String,
    parent_id: Option<String>,
    session_id: Option<String>,
    obs_type: Option<String>,
    name: String,
    status: Option<String>,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cost_usd: Option<f64>,
    input: Option<String>,
    output: Option<String>,
    metadata: Option<String>,
    sequence: Option<i64>,
    started_at: String,
    completed_at: Option<String>,
}

impl ObsRow {
    fn into_observation(self) -> Option<Observation> {
        let id = Uuid::parse_str(&self.id).ok()?;
        let trace_id = Uuid::parse_str(&self.trace_id).ok()?;
        let started_at: DateTime<Utc> = self.started_at.parse().ok()?;
        let completed_at: Option<DateTime<Utc>> =
            self.completed_at.as_deref().and_then(|s| s.parse().ok());
        let input: serde_json::Value = self
            .input
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        let output: serde_json::Value = self
            .output
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        let metadata: serde_json::Value = self
            .metadata
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Null);
        Some(Observation {
            id,
            trace_id,
            parent_id: self.parent_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
            session_id: self.session_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
            obs_type: str_to_obs_type(self.obs_type.as_deref().unwrap_or("generation")),
            name: self.name,
            input,
            output,
            model: self.model,
            input_tokens: self.input_tokens.unwrap_or(0) as u64,
            output_tokens: self.output_tokens.unwrap_or(0) as u64,
            cost_usd: self.cost_usd.unwrap_or(0.0),
            status: str_to_obs_status(self.status.as_deref().unwrap_or("running")),
            started_at,
            completed_at,
            metadata,
            sequence: self.sequence.unwrap_or(0) as u32,
            depth: 0,
            path: vec![],
            error_message: None,
        })
    }
}

// ---------------------------------------------------------------------------
// SqliteTraceStore
// ---------------------------------------------------------------------------

/// SQLite-backed trace store for persistent diagnostics.
///
/// Data is stored in `diag_traces` and `diag_observations` tables created
/// by migration `011_diagnostics`.
#[derive(Debug, Clone)]
pub struct SqliteTraceStore {
    pool: SqlitePool,
}

impl SqliteTraceStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TraceStore for SqliteTraceStore {
    async fn insert_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        let id = trace.id.to_string();
        let session_id = trace.session_id.to_string();
        let status = trace_status_to_str(trace.status);
        let metadata =
            serde_json::to_string(&trace.metadata).unwrap_or_else(|_| "null".into());
        let started_at = trace.started_at.to_rfc3339();
        let completed_at = trace.completed_at.map(|t| t.to_rfc3339());
        let input_toks = trace.total_input_tokens as i64;
        let output_toks = trace.total_output_tokens as i64;
        let llm_ms = trace.llm_duration_ms as i64;
        let tool_ms = trace.tool_duration_ms as i64;

        sqlx::query(
            "INSERT INTO diag_traces \
             (id, session_id, name, status, user_input, metadata, started_at, completed_at, \
              total_input_tokens, total_output_tokens, total_cost_usd, llm_duration_ms, tool_duration_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(&id)
        .bind(&session_id)
        .bind(&trace.name)
        .bind(status)
        .bind(&trace.user_input)
        .bind(&metadata)
        .bind(&started_at)
        .bind(&completed_at)
        .bind(input_toks)
        .bind(output_toks)
        .bind(trace.total_cost_usd)
        .bind(llm_ms)
        .bind(tool_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| storage_err(format!("insert_trace: {e}")))?;
        Ok(())
    }

    async fn get_trace(&self, id: Uuid) -> Result<Trace, TraceStoreError> {
        let id_str = id.to_string();
        let row: Option<TraceRow> = sqlx::query_as(
            "SELECT id, session_id, name, status, user_input, metadata, started_at, completed_at, \
             total_input_tokens, total_output_tokens, total_cost_usd, llm_duration_ms, tool_duration_ms \
             FROM diag_traces WHERE id = ?1",
        )
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| storage_err(format!("get_trace: {e}")))?;

        row.and_then(TraceRow::into_trace)
            .ok_or(TraceStoreError::TraceNotFound { id })
    }

    async fn update_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        let id = trace.id.to_string();
        let status = trace_status_to_str(trace.status);
        let metadata =
            serde_json::to_string(&trace.metadata).unwrap_or_else(|_| "null".into());
        let completed_at = trace.completed_at.map(|t| t.to_rfc3339());
        let input_toks = trace.total_input_tokens as i64;
        let output_toks = trace.total_output_tokens as i64;
        let llm_ms = trace.llm_duration_ms as i64;
        let tool_ms = trace.tool_duration_ms as i64;

        let rows = sqlx::query(
            "UPDATE diag_traces SET \
             status = ?1, metadata = ?2, completed_at = ?3, \
             total_input_tokens = ?4, total_output_tokens = ?5, total_cost_usd = ?6, \
             llm_duration_ms = ?7, tool_duration_ms = ?8 \
             WHERE id = ?9",
        )
        .bind(status)
        .bind(&metadata)
        .bind(&completed_at)
        .bind(input_toks)
        .bind(output_toks)
        .bind(trace.total_cost_usd)
        .bind(llm_ms)
        .bind(tool_ms)
        .bind(&id)
        .execute(&self.pool)
        .await
        .map_err(|e| storage_err(format!("update_trace: {e}")))?
        .rows_affected();

        if rows == 0 {
            return Err(TraceStoreError::TraceNotFound { id: trace.id });
        }
        Ok(())
    }

    async fn list_traces(
        &self,
        status: Option<TraceStatus>,
        since: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        let limit_i64 = limit as i64;
        let rows: Vec<TraceRow> = sqlx::query_as(
            "SELECT id, session_id, name, status, user_input, metadata, started_at, completed_at, \
             total_input_tokens, total_output_tokens, total_cost_usd, llm_duration_ms, tool_duration_ms \
             FROM diag_traces ORDER BY started_at DESC LIMIT ?1",
        )
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| storage_err(format!("list_traces: {e}")))?;

        let traces = rows
            .into_iter()
            .filter_map(|row| {
                let t = row.into_trace()?;
                if let Some(s) = since {
                    if t.started_at < s { return None; }
                }
                if let Some(filter) = status {
                    if t.status != filter { return None; }
                }
                Some(t)
            })
            .collect();
        Ok(traces)
    }

    async fn insert_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
        let id = obs.id.to_string();
        let trace_id = obs.trace_id.to_string();
        let parent_id = obs.parent_id.map(|u| u.to_string());
        let session_id = obs.session_id.map(|u| u.to_string());
        let obs_type = obs_type_to_str(obs.obs_type);
        let status = obs_status_to_str(obs.status);
        let input = serde_json::to_string(&obs.input).unwrap_or_else(|_| "null".into());
        let output = serde_json::to_string(&obs.output).unwrap_or_else(|_| "null".into());
        let metadata = serde_json::to_string(&obs.metadata).unwrap_or_else(|_| "null".into());
        let started_at = obs.started_at.to_rfc3339();
        let completed_at = obs.completed_at.map(|t| t.to_rfc3339());
        let input_toks = obs.input_tokens as i64;
        let output_toks = obs.output_tokens as i64;
        let seq = obs.sequence as i64;

        sqlx::query(
            "INSERT INTO diag_observations \
             (id, trace_id, parent_id, session_id, obs_type, name, status, model, \
              input_tokens, output_tokens, cost_usd, input, output, metadata, \
              sequence, started_at, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
             ON CONFLICT(id) DO NOTHING",
        )
        .bind(&id)
        .bind(&trace_id)
        .bind(&parent_id)
        .bind(&session_id)
        .bind(obs_type)
        .bind(&obs.name)
        .bind(status)
        .bind(&obs.model)
        .bind(input_toks)
        .bind(output_toks)
        .bind(obs.cost_usd)
        .bind(&input)
        .bind(&output)
        .bind(&metadata)
        .bind(seq)
        .bind(&started_at)
        .bind(&completed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| storage_err(format!("insert_observation: {e}")))?;
        Ok(())
    }

    async fn get_observations(&self, trace_id: Uuid) -> Result<Vec<Observation>, TraceStoreError> {
        let trace_id_str = trace_id.to_string();
        let rows: Vec<ObsRow> = sqlx::query_as(
            "SELECT id, trace_id, parent_id, session_id, obs_type, name, status, model, \
             input_tokens, output_tokens, cost_usd, input, output, metadata, sequence, \
             started_at, completed_at \
             FROM diag_observations WHERE trace_id = ?1 ORDER BY sequence ASC",
        )
        .bind(&trace_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| storage_err(format!("get_observations: {e}")))?;

        Ok(rows.into_iter().filter_map(ObsRow::into_observation).collect())
    }

    async fn insert_score(&self, _score: Score) -> Result<(), TraceStoreError> {
        // No scores table in the SQLite schema yet -- silently ignore.
        Ok(())
    }

    async fn get_scores(&self, _trace_id: Uuid) -> Result<Vec<Score>, TraceStoreError> {
        Ok(vec![])
    }

    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, TraceStoreError> {
        let before_str = before.to_rfc3339();
        let n = sqlx::query("DELETE FROM diag_traces WHERE started_at < ?1")
            .bind(&before_str)
            .execute(&self.pool)
            .await
            .map_err(|e| storage_err(format!("delete_before: {e}")))?
            .rows_affected();
        Ok(n)
    }

    async fn list_traces_by_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        let limit_i64 = limit as i64;
        let rows: Vec<TraceRow> = sqlx::query_as(
            "SELECT id, session_id, name, status, user_input, metadata, started_at, completed_at, \
             total_input_tokens, total_output_tokens, total_cost_usd, llm_duration_ms, tool_duration_ms \
             FROM diag_traces WHERE session_id = ?1 ORDER BY started_at ASC LIMIT ?2",
        )
        .bind(session_id)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| storage_err(format!("list_traces_by_session: {e}")))?;

        Ok(rows.into_iter().filter_map(TraceRow::into_trace).collect())
    }

    async fn get_observations_by_trace_ids(
        &self,
        trace_ids: &[Uuid],
    ) -> Result<Vec<Observation>, TraceStoreError> {
        if trace_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Build a single query with IN (...) placeholders.
        let placeholders: Vec<String> = (1..=trace_ids.len())
            .map(|i| format!("?{i}"))
            .collect();
        let sql = format!(
            "SELECT id, trace_id, parent_id, session_id, obs_type, name, status, model, \
             input_tokens, output_tokens, cost_usd, input, output, metadata, sequence, \
             started_at, completed_at \
             FROM diag_observations WHERE trace_id IN ({}) ORDER BY sequence ASC",
            placeholders.join(", ")
        );

        let mut query = sqlx::query_as::<_, ObsRow>(&sql);
        for id in trace_ids {
            query = query.bind(id.to_string());
        }

        let rows: Vec<ObsRow> = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| storage_err(format!("get_observations_by_trace_ids: {e}")))?;

        Ok(rows.into_iter().filter_map(ObsRow::into_observation).collect())
    }
}
