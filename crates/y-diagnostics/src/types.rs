//! Core diagnostic types for trace storage, cost tracking, and replay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Trace ────────────────────────────────────────────────────

/// Lifecycle status of a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TraceStatus {
    /// Active / in-progress.
    Active,
    /// Completed successfully.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled by user or system.
    Cancelled,
}

/// A top-level trace representing one user-facing action (e.g. a chat turn).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    /// Unique trace identifier.
    pub id: Uuid,
    /// Session this trace belongs to.
    pub session_id: Uuid,
    /// Human-readable name / label.
    pub name: String,
    /// Arbitrary key-value metadata.
    pub metadata: serde_json::Value,
    /// Tags for filtering.
    pub tags: Vec<String>,
    /// Current status.
    pub status: TraceStatus,
    /// When the trace started.
    pub started_at: DateTime<Utc>,
    /// When the trace completed (if it has).
    pub completed_at: Option<DateTime<Utc>>,
    /// Total input tokens consumed.
    pub total_input_tokens: u64,
    /// Total output tokens consumed.
    pub total_output_tokens: u64,
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Original user input that triggered this trace.
    pub user_input: Option<String>,
    /// Total wall-clock duration in milliseconds (computed on complete).
    pub total_duration_ms: Option<u64>,
    /// Time spent waiting for LLM responses (accumulated).
    pub llm_duration_ms: u64,
    /// Time spent executing tools (accumulated).
    pub tool_duration_ms: u64,
    /// Replay context for debugging (system prompt, history, tool defs).
    pub replay_context: Option<serde_json::Value>,
}

/// Context captured for trace replay and debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayContext {
    /// The system prompt used for this trace.
    pub system_prompt: String,
    /// Conversation history at the time of the trace.
    pub conversation_history: Vec<serde_json::Value>,
    /// Tool definitions available during this trace.
    pub tool_definitions: Vec<serde_json::Value>,
    /// Configuration snapshot at the time of the trace.
    pub config_snapshot: serde_json::Value,
}

impl Trace {
    /// Create a new active trace.
    pub fn new(session_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            name: name.into(),
            metadata: serde_json::Value::Null,
            tags: Vec::new(),
            status: TraceStatus::Active,
            started_at: Utc::now(),
            completed_at: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            user_input: None,
            total_duration_ms: None,
            llm_duration_ms: 0,
            tool_duration_ms: 0,
            replay_context: None,
        }
    }

    /// Mark as completed. Automatically computes `total_duration_ms`.
    pub fn complete(&mut self) {
        let now = Utc::now();
        self.status = TraceStatus::Completed;
        self.completed_at = Some(now);
        self.total_duration_ms =
            Some(u64::try_from((now - self.started_at).num_milliseconds().max(0)).unwrap_or(0));
    }

    /// Mark as failed. Automatically computes `total_duration_ms`.
    pub fn fail(&mut self) {
        let now = Utc::now();
        self.status = TraceStatus::Failed;
        self.completed_at = Some(now);
        self.total_duration_ms =
            Some(u64::try_from((now - self.started_at).num_milliseconds().max(0)).unwrap_or(0));
    }

    /// Duration in milliseconds (if completed).
    pub fn duration_ms(&self) -> Option<i64> {
        self.completed_at
            .map(|end| (end - self.started_at).num_milliseconds())
    }
}

// ─── Observation ──────────────────────────────────────────────

/// Kind of observation within a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservationType {
    /// LLM generation (prompt → response).
    Generation,
    /// Tool invocation.
    ToolCall,
    /// Sub-span / logical grouping.
    Span,
    /// User interaction (HITL).
    UserInput,
    /// MCP protocol call.
    McpCall,
    /// RAG retrieval step.
    Retrieval,
    /// Embedding computation.
    Embedding,
    /// Reranking step.
    Reranking,
    /// Delegated sub-agent execution.
    SubAgent,
    /// Planning step.
    Planning,
    /// Self-reflection / evaluation.
    Reflection,
    /// Guardrail check.
    Guardrail,
    /// Hook middleware execution.
    Hook,
    /// Cache hit/miss event.
    Cache,
}

/// Status of an individual observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservationStatus {
    Running,
    Completed,
    Failed,
}

/// A single observation in a trace's execution tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Unique observation ID.
    pub id: Uuid,
    /// Parent trace ID.
    pub trace_id: Uuid,
    /// Parent observation ID (for nesting).
    pub parent_id: Option<Uuid>,
    /// Session ID for direct session-level queries (denormalized from trace).
    pub session_id: Option<Uuid>,
    /// Kind of observation.
    pub obs_type: ObservationType,
    /// Human-readable name.
    pub name: String,
    /// Input to this step.
    pub input: serde_json::Value,
    /// Output from this step.
    pub output: serde_json::Value,
    /// Model used (for generations).
    pub model: Option<String>,
    /// Input token count.
    pub input_tokens: u64,
    /// Output token count.
    pub output_tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Status.
    pub status: ObservationStatus,
    /// Start time.
    pub started_at: DateTime<Utc>,
    /// End time.
    pub completed_at: Option<DateTime<Utc>>,
    /// Arbitrary metadata.
    pub metadata: serde_json::Value,
    /// Sort key for ordering within a level.
    pub sequence: u32,
    /// Tree depth from root (0 = root observation).
    pub depth: u32,
    /// Materialized path from root to this observation (UUIDs).
    pub path: Vec<Uuid>,
    /// Error message (if status is Failed).
    pub error_message: Option<String>,
}

impl Observation {
    /// Create a new running observation.
    pub fn new(trace_id: Uuid, obs_type: ObservationType, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            trace_id,
            parent_id: None,
            session_id: None,
            obs_type,
            name: name.into(),
            input: serde_json::Value::Null,
            output: serde_json::Value::Null,
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            status: ObservationStatus::Running,
            started_at: Utc::now(),
            completed_at: None,
            metadata: serde_json::Value::Null,
            sequence: 0,
            depth: 0,
            path: Vec::new(),
            error_message: None,
        }
    }

    /// Mark as completed.
    pub fn complete(&mut self) {
        self.status = ObservationStatus::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Duration in milliseconds (if completed).
    pub fn duration_ms(&self) -> Option<i64> {
        self.completed_at
            .map(|end| (end - self.started_at).num_milliseconds())
    }
}

// ─── Score ────────────────────────────────────────────────────

/// Source of a score evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreSource {
    /// Computed by the system automatically.
    System,
    /// Provided by an LLM evaluator.
    Llm,
    /// Provided by a human reviewer.
    Human,
    /// Provided via user feedback (thumbs up/down, etc.).
    UserFeedback,
    /// Imported from an external system (e.g. Langfuse annotations).
    External,
}

/// Value of a score — numeric, categorical, or boolean.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScoreValue {
    Numeric(f64),
    Boolean(bool),
    Categorical(String),
}

/// A quality / evaluation score attached to a trace or observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub id: Uuid,
    pub trace_id: Uuid,
    pub observation_id: Option<Uuid>,
    pub name: String,
    pub value: ScoreValue,
    pub source: ScoreSource,
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Score {
    /// Create a numeric score.
    pub fn numeric(
        trace_id: Uuid,
        name: impl Into<String>,
        value: f64,
        source: ScoreSource,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            trace_id,
            observation_id: None,
            name: name.into(),
            value: ScoreValue::Numeric(value),
            source,
            comment: None,
            created_at: Utc::now(),
        }
    }
}

// ─── Cost Record ──────────────────────────────────────────────

/// Aggregated cost record for a time window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

/// Daily cost summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCostSummary {
    pub date: chrono::NaiveDate,
    pub total_cost_usd: f64,
    pub total_traces: u64,
    pub by_model: Vec<CostRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_lifecycle() {
        let mut trace = Trace::new(Uuid::new_v4(), "test-trace");
        assert_eq!(trace.status, TraceStatus::Active);
        assert!(trace.completed_at.is_none());

        trace.complete();
        assert_eq!(trace.status, TraceStatus::Completed);
        assert!(trace.completed_at.is_some());
        assert!(trace.duration_ms().unwrap() >= 0);
    }

    #[test]
    fn test_observation_lifecycle() {
        let trace_id = Uuid::new_v4();
        let mut obs = Observation::new(trace_id, ObservationType::Generation, "llm-call");
        assert_eq!(obs.status, ObservationStatus::Running);
        assert_eq!(obs.trace_id, trace_id);

        obs.complete();
        assert_eq!(obs.status, ObservationStatus::Completed);
        assert!(obs.duration_ms().unwrap() >= 0);
    }

    #[test]
    fn test_score_creation() {
        let trace_id = Uuid::new_v4();
        let score = Score::numeric(trace_id, "accuracy", 0.95, ScoreSource::System);
        assert_eq!(score.trace_id, trace_id);
        assert!(matches!(score.value, ScoreValue::Numeric(v) if (v - 0.95).abs() < f64::EPSILON));
    }

    // ── Phase 1 tests ─────────────────────────────────────────────

    #[test]
    fn test_observation_type_roundtrip_all_variants() {
        let all_variants = [
            ObservationType::Generation,
            ObservationType::ToolCall,
            ObservationType::Span,
            ObservationType::UserInput,
            ObservationType::McpCall,
            ObservationType::Retrieval,
            ObservationType::Embedding,
            ObservationType::Reranking,
            ObservationType::SubAgent,
            ObservationType::Planning,
            ObservationType::Reflection,
            ObservationType::Guardrail,
            ObservationType::Hook,
            ObservationType::Cache,
        ];

        for variant in &all_variants {
            let json = serde_json::to_string(variant).unwrap();
            let deserialized: ObservationType = serde_json::from_str(&json).unwrap();
            assert_eq!(*variant, deserialized, "roundtrip failed for {json}");
        }

        assert_eq!(all_variants.len(), 14);
    }

    #[test]
    fn test_trace_new_fields_defaults() {
        let trace = Trace::new(Uuid::new_v4(), "test");
        assert!(trace.user_input.is_none());
        assert!(trace.total_duration_ms.is_none());
        assert_eq!(trace.llm_duration_ms, 0);
        assert_eq!(trace.tool_duration_ms, 0);
        assert!(trace.replay_context.is_none());
    }

    #[test]
    fn test_trace_complete_sets_duration() {
        let mut trace = Trace::new(Uuid::new_v4(), "test");
        assert!(trace.total_duration_ms.is_none());

        trace.complete();
        assert!(trace.total_duration_ms.is_some());
        assert_eq!(trace.status, TraceStatus::Completed);
    }

    #[test]
    fn test_trace_fail_sets_duration() {
        let mut trace = Trace::new(Uuid::new_v4(), "test");
        trace.fail();
        assert!(trace.total_duration_ms.is_some());
        assert_eq!(trace.status, TraceStatus::Failed);
    }

    #[test]
    fn test_replay_context_serialization() {
        let ctx = ReplayContext {
            system_prompt: "You are a helpful assistant.".into(),
            conversation_history: vec![serde_json::json!({"role": "user", "content": "hi"})],
            tool_definitions: vec![serde_json::json!({"name": "search"})],
            config_snapshot: serde_json::json!({"model": "gpt-4"}),
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: ReplayContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.system_prompt, "You are a helpful assistant.");
        assert_eq!(deserialized.conversation_history.len(), 1);
        assert_eq!(deserialized.tool_definitions.len(), 1);
    }

    #[test]
    fn test_observation_depth_path_defaults() {
        let obs = Observation::new(Uuid::new_v4(), ObservationType::Span, "root");
        assert_eq!(obs.depth, 0);
        assert!(obs.path.is_empty());
        assert!(obs.error_message.is_none());
    }

    #[test]
    fn test_observation_error_message() {
        let mut obs = Observation::new(Uuid::new_v4(), ObservationType::ToolCall, "fail-tool");
        obs.status = ObservationStatus::Failed;
        obs.error_message = Some("connection refused".into());
        assert_eq!(obs.error_message.as_deref(), Some("connection refused"));
    }

    #[test]
    fn test_boolean_score_creation() {
        let score = Score {
            id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            observation_id: None,
            name: "passed".into(),
            value: ScoreValue::Boolean(true),
            source: ScoreSource::System,
            comment: None,
            created_at: chrono::Utc::now(),
        };
        assert!(matches!(score.value, ScoreValue::Boolean(true)));
    }

    #[test]
    fn test_user_feedback_source() {
        let score = Score::numeric(Uuid::new_v4(), "rating", 5.0, ScoreSource::UserFeedback);
        assert_eq!(score.source, ScoreSource::UserFeedback);
    }
}
