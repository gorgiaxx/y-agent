//! PostgreSQL-backed `TraceStore` implementation.
//!
//! Feature-gated behind `diagnostics_pg`. Stores traces, observations, and
//! scores in the `observability` `PostgreSQL` schema.

#[cfg(feature = "diagnostics_pg")]
mod inner {
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use sqlx::PgPool;
    use uuid::Uuid;

    use crate::trace_store::{TraceStore, TraceStoreError};
    use crate::types::{
        Observation, ObservationStatus, ObservationType, Score, ScoreSource, ScoreValue, Trace,
        TraceStatus,
    };

    /// PostgreSQL-backed trace store for diagnostics.
    ///
    /// Maps to the `observability.*` schema defined in
    /// `migrations/postgres/001_observability_schema.up.sql`.
    #[derive(Debug, Clone)]
    pub struct PgTraceStore {
        pool: PgPool,
    }

    impl PgTraceStore {
        /// Create a new `PgTraceStore` with the given connection pool.
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }

        /// Run `PostgreSQL` migrations from the `migrations/postgres/` directory.
        pub async fn run_migrations(&self) -> Result<(), TraceStoreError> {
            sqlx::migrate!("../../migrations/postgres")
                .run(&self.pool)
                .await
                .map_err(|e| TraceStoreError::Storage {
                    message: format!("migration failed: {e}"),
                })
        }
    }

    // ── Status mapping helpers ─────────────────────────────────────

    fn trace_status_to_pg(status: TraceStatus) -> &'static str {
        match status {
            TraceStatus::Active => "running",
            TraceStatus::Completed => "success",
            TraceStatus::Failed => "failed",
            TraceStatus::Cancelled => "cancelled",
        }
    }

    fn pg_to_trace_status(s: &str) -> TraceStatus {
        match s {
            "running" => TraceStatus::Active,
            "success" => TraceStatus::Completed,
            "failed" => TraceStatus::Failed,
            "cancelled" => TraceStatus::Cancelled,
            "timeout" => TraceStatus::Failed,
            _ => TraceStatus::Active,
        }
    }

    fn obs_type_to_pg(t: ObservationType) -> &'static str {
        match t {
            ObservationType::Generation => "llm_call",
            ObservationType::ToolCall => "tool_call",
            ObservationType::Span => "span",
            ObservationType::UserInput => "span",
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

    fn pg_to_obs_type(s: &str) -> ObservationType {
        match s {
            "llm_call" => ObservationType::Generation,
            "tool_call" => ObservationType::ToolCall,
            "span" => ObservationType::Span,
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
            _ => ObservationType::Span,
        }
    }

    fn obs_status_to_pg(status: ObservationStatus) -> &'static str {
        match status {
            ObservationStatus::Running => "running",
            ObservationStatus::Completed => "success",
            ObservationStatus::Failed => "failed",
        }
    }

    fn pg_to_obs_status(s: &str) -> ObservationStatus {
        match s {
            "running" => ObservationStatus::Running,
            "success" => ObservationStatus::Completed,
            "failed" => ObservationStatus::Failed,
            _ => ObservationStatus::Running,
        }
    }

    fn score_source_to_pg(source: ScoreSource) -> &'static str {
        match source {
            ScoreSource::System => "auto",
            ScoreSource::Llm => "model",
            ScoreSource::Human => "human",
            ScoreSource::UserFeedback => "user_feedback",
        }
    }

    fn pg_to_score_source(s: &str) -> ScoreSource {
        match s {
            "auto" => ScoreSource::System,
            "model" => ScoreSource::Llm,
            "human" => ScoreSource::Human,
            "user_feedback" => ScoreSource::UserFeedback,
            _ => ScoreSource::System,
        }
    }

    // ── Row types for sqlx query_as ───────────────────────────────

    #[derive(sqlx::FromRow)]
    struct TraceRow {
        id: Uuid,
        session_id: String,
        name: String,
        status: String,
        metadata: serde_json::Value,
        tags: Vec<String>,
        #[allow(dead_code)]
        total_tokens: i32,
        total_cost: f64,
        started_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
        user_input: Option<String>,
        llm_duration_ms: Option<i32>,
        tool_duration_ms: Option<i32>,
        replay_context: Option<serde_json::Value>,
    }

    impl TraceRow {
        fn into_trace(self) -> Trace {
            use std::str::FromStr;

            let session_id = Uuid::from_str(&self.session_id).unwrap_or_default();
            let duration_ms = self
                .completed_at
                .map(|end| (end - self.started_at).num_milliseconds().max(0) as u64);

            Trace {
                id: self.id,
                session_id,
                name: self.name,
                metadata: self.metadata,
                tags: self.tags,
                status: pg_to_trace_status(&self.status),
                started_at: self.started_at,
                completed_at: self.completed_at,
                total_input_tokens: 0, // Set from observations
                total_output_tokens: 0,
                total_cost_usd: self.total_cost,
                user_input: self.user_input,
                total_duration_ms: duration_ms,
                llm_duration_ms: self.llm_duration_ms.unwrap_or(0) as u64,
                tool_duration_ms: self.tool_duration_ms.unwrap_or(0) as u64,
                replay_context: self.replay_context,
            }
        }
    }

    #[derive(sqlx::FromRow)]
    struct ObservationRow {
        id: Uuid,
        trace_id: Uuid,
        parent_id: Option<Uuid>,
        session_id: Option<String>,
        observation_type: String,
        name: String,
        status: String,
        input: Option<serde_json::Value>,
        output: Option<serde_json::Value>,
        metadata: serde_json::Value,
        model: Option<String>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
        cost: Option<f64>,
        started_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
        depth: i32,
        path: Vec<Uuid>,
        error_message: Option<String>,
    }

    impl ObservationRow {
        fn into_observation(self) -> Observation {
            use std::str::FromStr;

            Observation {
                id: self.id,
                trace_id: self.trace_id,
                parent_id: self.parent_id,
                session_id: self.session_id.and_then(|s| Uuid::from_str(&s).ok()),
                obs_type: pg_to_obs_type(&self.observation_type),
                name: self.name,
                input: self.input.unwrap_or(serde_json::Value::Null),
                output: self.output.unwrap_or(serde_json::Value::Null),
                model: self.model,
                input_tokens: self.input_tokens.unwrap_or(0) as u64,
                output_tokens: self.output_tokens.unwrap_or(0) as u64,
                cost_usd: self.cost.unwrap_or(0.0),
                status: pg_to_obs_status(&self.status),
                started_at: self.started_at,
                completed_at: self.completed_at,
                metadata: self.metadata,
                sequence: 0,
                depth: self.depth as u32,
                path: self.path,
                error_message: self.error_message,
            }
        }
    }

    #[derive(sqlx::FromRow)]
    struct ScoreRow {
        id: Uuid,
        trace_id: Uuid,
        observation_id: Option<Uuid>,
        name: String,
        value: f64,
        data_type: Option<String>,
        string_value: Option<String>,
        source: String,
        comment: Option<String>,
        created_at: DateTime<Utc>,
    }

    impl ScoreRow {
        fn into_score(self) -> Score {
            let value = match self.data_type.as_deref() {
                Some("boolean") => ScoreValue::Boolean(self.value != 0.0),
                Some("categorical") => {
                    ScoreValue::Categorical(self.string_value.unwrap_or_default())
                }
                _ => ScoreValue::Numeric(self.value),
            };

            Score {
                id: self.id,
                trace_id: self.trace_id,
                observation_id: self.observation_id,
                name: self.name,
                value,
                source: pg_to_score_source(&self.source),
                comment: self.comment,
                created_at: self.created_at,
            }
        }
    }

    // ── TraceStore implementation ─────────────────────────────────

    #[async_trait]
    impl TraceStore for PgTraceStore {
        async fn insert_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
            let status = trace_status_to_pg(trace.status);
            let session_id = trace.session_id.to_string();
            let total_tokens = (trace.total_input_tokens + trace.total_output_tokens) as i32;
            let llm_dur = trace.llm_duration_ms as i32;
            let tool_dur = trace.tool_duration_ms as i32;

            sqlx::query(
                r"
                INSERT INTO observability.traces
                    (id, session_id, name, status, metadata, tags,
                     total_tokens, total_cost, started_at, completed_at,
                     user_input, llm_duration_ms, tool_duration_ms, replay_context)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                ",
            )
            .bind(trace.id)
            .bind(&session_id)
            .bind(&trace.name)
            .bind(status)
            .bind(&trace.metadata)
            .bind(&trace.tags)
            .bind(total_tokens)
            .bind(trace.total_cost_usd)
            .bind(trace.started_at)
            .bind(trace.completed_at)
            .bind(&trace.user_input)
            .bind(llm_dur)
            .bind(tool_dur)
            .bind(&trace.replay_context)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("insert_trace: {e}"),
            })?;

            Ok(())
        }

        async fn get_trace(&self, id: Uuid) -> Result<Trace, TraceStoreError> {
            let row: TraceRow = sqlx::query_as(
                r"
                SELECT id, session_id, name, status, metadata, tags,
                       total_tokens, total_cost, started_at, completed_at,
                       user_input, llm_duration_ms, tool_duration_ms, replay_context
                FROM observability.traces
                WHERE id = $1
                ",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("get_trace: {e}"),
            })?
            .ok_or(TraceStoreError::TraceNotFound { id })?;

            Ok(row.into_trace())
        }

        async fn update_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
            let status = trace_status_to_pg(trace.status);
            let total_tokens = (trace.total_input_tokens + trace.total_output_tokens) as i32;
            let llm_dur = trace.llm_duration_ms as i32;
            let tool_dur = trace.tool_duration_ms as i32;

            let result = sqlx::query(
                r"
                UPDATE observability.traces
                SET status = $2, metadata = $3, tags = $4,
                    total_tokens = $5, total_cost = $6, completed_at = $7,
                    user_input = $8, llm_duration_ms = $9,
                    tool_duration_ms = $10, replay_context = $11
                WHERE id = $1
                ",
            )
            .bind(trace.id)
            .bind(status)
            .bind(&trace.metadata)
            .bind(&trace.tags)
            .bind(total_tokens)
            .bind(trace.total_cost_usd)
            .bind(trace.completed_at)
            .bind(&trace.user_input)
            .bind(llm_dur)
            .bind(tool_dur)
            .bind(&trace.replay_context)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("update_trace: {e}"),
            })?;

            if result.rows_affected() == 0 {
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
            let status_str = status.map(trace_status_to_pg);

            let rows: Vec<TraceRow> = sqlx::query_as(
                r"
                SELECT id, session_id, name, status, metadata, tags,
                       total_tokens, total_cost, started_at, completed_at,
                       user_input, llm_duration_ms, tool_duration_ms, replay_context
                FROM observability.traces
                WHERE ($1::TEXT IS NULL OR status = $1)
                  AND ($2::TIMESTAMPTZ IS NULL OR started_at >= $2)
                ORDER BY started_at DESC
                LIMIT $3
                ",
            )
            .bind(status_str)
            .bind(since)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("list_traces: {e}"),
            })?;

            Ok(rows.into_iter().map(TraceRow::into_trace).collect())
        }

        async fn insert_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
            let obs_type = obs_type_to_pg(obs.obs_type);
            let status = obs_status_to_pg(obs.status);
            let input_tokens = obs.input_tokens as i32;
            let output_tokens = obs.output_tokens as i32;
            let depth = obs.depth as i32;
            let session_id_str = obs.session_id.map(|s| s.to_string());

            sqlx::query(
                r"
                INSERT INTO observability.observations
                    (id, trace_id, parent_id, session_id, observation_type, name, status,
                     input, output, metadata, model, input_tokens, output_tokens,
                     cost, started_at, completed_at, depth, path, error_message)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
                ",
            )
            .bind(obs.id)
            .bind(obs.trace_id)
            .bind(obs.parent_id)
            .bind(&session_id_str)
            .bind(obs_type)
            .bind(&obs.name)
            .bind(status)
            .bind(&obs.input)
            .bind(&obs.output)
            .bind(&obs.metadata)
            .bind(&obs.model)
            .bind(input_tokens)
            .bind(output_tokens)
            .bind(obs.cost_usd)
            .bind(obs.started_at)
            .bind(obs.completed_at)
            .bind(depth)
            .bind(&obs.path)
            .bind(&obs.error_message)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("insert_observation: {e}"),
            })?;

            Ok(())
        }

        async fn get_observations(
            &self,
            trace_id: Uuid,
        ) -> Result<Vec<Observation>, TraceStoreError> {
            let rows: Vec<ObservationRow> = sqlx::query_as(
                r"
                SELECT id, trace_id, parent_id, session_id, observation_type, name, status,
                       input, output, metadata, model, input_tokens, output_tokens,
                       cost, started_at, completed_at, depth, path, error_message
                FROM observability.observations
                WHERE trace_id = $1
                ORDER BY started_at ASC
                ",
            )
            .bind(trace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("get_observations: {e}"),
            })?;

            Ok(rows
                .into_iter()
                .map(ObservationRow::into_observation)
                .collect())
        }

        async fn insert_score(&self, score: Score) -> Result<(), TraceStoreError> {
            let source = score_source_to_pg(score.source);
            let (value, data_type, string_value): (f64, &str, Option<String>) = match &score.value {
                ScoreValue::Numeric(v) => (*v, "numeric", None),
                ScoreValue::Boolean(b) => (if *b { 1.0 } else { 0.0 }, "boolean", None),
                ScoreValue::Categorical(s) => (0.0, "categorical", Some(s.clone())),
            };

            sqlx::query(
                r"
                INSERT INTO observability.scores
                    (id, trace_id, observation_id, name, value, data_type,
                     string_value, source, comment)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ",
            )
            .bind(score.id)
            .bind(score.trace_id)
            .bind(score.observation_id)
            .bind(&score.name)
            .bind(value)
            .bind(data_type)
            .bind(&string_value)
            .bind(source)
            .bind(&score.comment)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("insert_score: {e}"),
            })?;

            Ok(())
        }

        async fn get_scores(&self, trace_id: Uuid) -> Result<Vec<Score>, TraceStoreError> {
            let rows: Vec<ScoreRow> = sqlx::query_as(
                r"
                SELECT id, trace_id, observation_id, name, value, data_type,
                       string_value, source, comment, created_at
                FROM observability.scores
                WHERE trace_id = $1
                ORDER BY created_at ASC
                ",
            )
            .bind(trace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("get_scores: {e}"),
            })?;

            Ok(rows.into_iter().map(ScoreRow::into_score).collect())
        }

        async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, TraceStoreError> {
            let result = sqlx::query(
                r"
                DELETE FROM observability.traces
                WHERE started_at < $1
                  AND status != 'running'
                ",
            )
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("delete_before: {e}"),
            })?;

            Ok(result.rows_affected())
        }

        async fn list_traces_by_session(
            &self,
            session_id: &str,
            limit: usize,
        ) -> Result<Vec<Trace>, TraceStoreError> {
            let rows: Vec<TraceRow> = sqlx::query_as(
                r"
                SELECT id, session_id, name, status, metadata, tags,
                       total_tokens, total_cost, started_at, completed_at,
                       user_input, llm_duration_ms, tool_duration_ms, replay_context
                FROM observability.traces
                WHERE session_id = $1
                ORDER BY started_at DESC
                LIMIT $2
                ",
            )
            .bind(session_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| TraceStoreError::Storage {
                message: format!("list_traces_by_session: {e}"),
            })?;

            Ok(rows.into_iter().map(TraceRow::into_trace).collect())
        }
    }
}

#[cfg(feature = "diagnostics_pg")]
pub use inner::PgTraceStore;
