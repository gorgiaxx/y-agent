//! Diagnostics subscriber: captures events from the hook system.
//!
//! Each `on_*` method records a diagnostics event — trace start, LLM
//! generation, tool execution, or trace end — and populates both metrics
//! (token counts, cost, duration) and content (input/output payloads).

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::trace_store::TraceStore;
use crate::types::{Observation, ObservationStatus, ObservationType, Trace};

/// Parameters for recording a generation (LLM call) observation.
pub struct GenerationParams {
    /// The trace this generation belongs to.
    pub trace_id: Uuid,
    /// Optional parent observation ID.
    pub parent_id: Option<Uuid>,
    /// Optional session ID.
    pub session_id: Option<Uuid>,
    /// Model name used.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens produced.
    pub output_tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Full request payload.
    pub input: serde_json::Value,
    /// Full response payload.
    pub output: serde_json::Value,
    /// Wall-clock execution time in milliseconds.
    pub duration_ms: u64,
}

/// Subscriber that listens to y-hooks events and auto-captures diagnostics.
pub struct DiagnosticsSubscriber<S: ?Sized> {
    store: Arc<S>,
}

impl<S: TraceStore + ?Sized> DiagnosticsSubscriber<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Get a reference to the underlying trace store.
    pub fn store(&self) -> Arc<S> {
        Arc::clone(&self.store)
    }

    /// Handle a new trace starting (e.g. a user message arrives).
    ///
    /// `user_input` is the original user message text that triggered this trace.
    pub async fn on_trace_start(
        &self,
        session_id: Uuid,
        name: &str,
        user_input: &str,
    ) -> Result<Uuid, crate::trace_store::TraceStoreError> {
        let mut trace = Trace::new(session_id, name);
        trace.user_input = Some(user_input.to_string());
        let trace_id = trace.id;
        self.store.insert_trace(trace).await?;
        Ok(trace_id)
    }

    /// Handle a generation observation (LLM call).
    ///
    /// `input` and `output` are the full JSON payloads of the LLM request and
    /// response, captured for diagnostics and debugging replay.
    pub async fn on_generation(
        &self,
        params: GenerationParams,
    ) -> Result<Uuid, crate::trace_store::TraceStoreError> {
        let mut obs = Observation::new(params.trace_id, ObservationType::Generation, "llm-generation");
        obs.parent_id = params.parent_id;
        obs.session_id = params.session_id;
        obs.model = Some(params.model.to_string());
        obs.input_tokens = params.input_tokens;
        obs.output_tokens = params.output_tokens;
        obs.cost_usd = params.cost_usd;
        obs.input = params.input;
        obs.output = params.output;
        obs.status = ObservationStatus::Completed;
        obs.completed_at = Some(Utc::now());
        obs.metadata = serde_json::json!({ "duration_ms": params.duration_ms });
        let obs_id = obs.id;
        self.store.insert_observation(obs).await?;
        Ok(obs_id)
    }

    /// Handle a tool call observation (recorded after execution).
    ///
    /// `input` is the tool arguments, `output` is the tool result.
    /// `duration_ms` is wall-clock execution time.
    /// `success` indicates whether the tool executed without error.
    pub async fn on_tool_call(
        &self,
        trace_id: Uuid,
        parent_id: Option<Uuid>,
        session_id: Option<Uuid>,
        tool_name: &str,
        input: serde_json::Value,
        output: serde_json::Value,
        duration_ms: u64,
        success: bool,
    ) -> Result<Uuid, crate::trace_store::TraceStoreError> {
        let mut obs = Observation::new(trace_id, ObservationType::ToolCall, tool_name);
        obs.parent_id = parent_id;
        obs.session_id = session_id;
        obs.input = input;
        obs.output = output;
        obs.status = if success {
            ObservationStatus::Completed
        } else {
            ObservationStatus::Failed
        };
        obs.completed_at = Some(Utc::now());
        obs.metadata = serde_json::json!({ "duration_ms": duration_ms });
        let obs_id = obs.id;
        self.store.insert_observation(obs).await?;
        Ok(obs_id)
    }

    /// Handle trace completion.
    ///
    /// `output` is the final assistant response text (if any).
    pub async fn on_trace_end(
        &self,
        trace_id: Uuid,
        success: bool,
        output: Option<&str>,
    ) -> Result<(), crate::trace_store::TraceStoreError> {
        let mut trace = self.store.get_trace(trace_id).await?;
        if success {
            trace.complete();
        } else {
            trace.fail();
        }

        // Accumulate totals from observations.
        let observations = self.store.get_observations(trace_id).await?;
        trace.total_input_tokens = observations.iter().map(|o| o.input_tokens).sum();
        trace.total_output_tokens = observations.iter().map(|o| o.output_tokens).sum();
        trace.total_cost_usd = observations.iter().map(|o| o.cost_usd).sum();

        // Accumulate duration breakdowns from observation metadata.
        for obs in &observations {
            if let Some(dur) = obs
                .metadata
                .get("duration_ms")
                .and_then(serde_json::Value::as_u64)
            {
                match obs.obs_type {
                    ObservationType::Generation => trace.llm_duration_ms += dur,
                    ObservationType::ToolCall => trace.tool_duration_ms += dur,
                    _ => {}
                }
            }
        }

        // Merge the final output into trace metadata (preserve existing keys).
        if let Some(text) = output {
            if let serde_json::Value::Object(ref mut map) = trace.metadata {
                map.insert(
                    "output".to_string(),
                    serde_json::Value::String(text.to_string()),
                );
            } else {
                trace.metadata = serde_json::json!({ "output": text });
            }
        }

        self.store.update_trace(trace).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_store::InMemoryTraceStore;
    use crate::types::*;

    #[tokio::test]
    async fn test_subscriber_auto_capture() {
        let store = Arc::new(InMemoryTraceStore::new());
        let subscriber = DiagnosticsSubscriber::new(store.clone());
        let session = Uuid::new_v4();

        // Simulate a chat turn.
        let trace_id = subscriber
            .on_trace_start(session, "chat-turn", "What is Rust?")
            .await
            .unwrap();

        // LLM call.
        let gen_id = subscriber
            .on_generation(GenerationParams {
                trace_id,
                parent_id: None,
                session_id: None,
                model: "gpt-4".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.005,
                input: serde_json::json!({"messages": [{"role": "user", "content": "What is Rust?"}]}),
                output: serde_json::json!({"content": "Rust is a systems programming language."}),
                duration_ms: 250,
            })
            .await
            .unwrap();

        // Tool call from LLM.
        let _tool_id = subscriber
            .on_tool_call(
                trace_id,
                Some(gen_id),
                None,
                "web_search",
                serde_json::json!({"query": "Rust programming language"}),
                serde_json::json!({"results": ["https://rust-lang.org"]}),
                120,
                true,
            )
            .await
            .unwrap();

        // Complete trace.
        subscriber
            .on_trace_end(
                trace_id,
                true,
                Some("Rust is a systems programming language."),
            )
            .await
            .unwrap();

        // Verify trace-level fields.
        let trace = store.get_trace(trace_id).await.unwrap();
        assert_eq!(trace.status, TraceStatus::Completed);
        assert_eq!(trace.user_input.as_deref(), Some("What is Rust?"));
        assert_eq!(trace.total_input_tokens, 100);
        assert_eq!(trace.total_output_tokens, 50);
        assert!((trace.total_cost_usd - 0.005).abs() < f64::EPSILON);
        assert_eq!(trace.llm_duration_ms, 250);
        assert_eq!(trace.tool_duration_ms, 120);
        assert_eq!(
            trace.metadata.get("output").and_then(|v| v.as_str()),
            Some("Rust is a systems programming language.")
        );

        // Verify observation-level input/output.
        let obs = store.get_observations(trace_id).await.unwrap();
        assert_eq!(obs.len(), 2);

        let gen_obs = obs
            .iter()
            .find(|o| o.obs_type == ObservationType::Generation)
            .unwrap();
        assert!(gen_obs.input.get("messages").is_some());
        assert_eq!(
            gen_obs.output.get("content").and_then(|v| v.as_str()),
            Some("Rust is a systems programming language.")
        );
        assert_eq!(gen_obs.status, ObservationStatus::Completed);

        let tool_obs = obs
            .iter()
            .find(|o| o.obs_type == ObservationType::ToolCall)
            .unwrap();
        assert_eq!(
            tool_obs.input.get("query").and_then(|v| v.as_str()),
            Some("Rust programming language")
        );
        assert!(tool_obs.output.get("results").is_some());
        assert_eq!(tool_obs.status, ObservationStatus::Completed);
    }

    #[tokio::test]
    async fn test_subscriber_failed_trace() {
        let store = Arc::new(InMemoryTraceStore::new());
        let subscriber = DiagnosticsSubscriber::new(store.clone());
        let session = Uuid::new_v4();

        let trace_id = subscriber
            .on_trace_start(session, "chat-turn", "hello")
            .await
            .unwrap();

        subscriber
            .on_trace_end(trace_id, false, None)
            .await
            .unwrap();

        let trace = store.get_trace(trace_id).await.unwrap();
        assert_eq!(trace.status, TraceStatus::Failed);
        assert_eq!(trace.user_input.as_deref(), Some("hello"));
        // No output metadata when trace fails without output.
        assert!(trace.metadata.is_null() || trace.metadata.get("output").is_none());
    }

    #[tokio::test]
    async fn test_subscriber_tool_call_failure() {
        let store = Arc::new(InMemoryTraceStore::new());
        let subscriber = DiagnosticsSubscriber::new(store.clone());
        let session = Uuid::new_v4();

        let trace_id = subscriber
            .on_trace_start(session, "chat-turn", "run ls")
            .await
            .unwrap();

        let _tool_id = subscriber
            .on_tool_call(
                trace_id,
                None,
                None,
                "shell_exec",
                serde_json::json!({"command": "ls"}),
                serde_json::json!({"error": "permission denied"}),
                50,
                false,
            )
            .await
            .unwrap();

        let obs = store.get_observations(trace_id).await.unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].status, ObservationStatus::Failed);
        assert_eq!(
            obs[0].output.get("error").and_then(|v| v.as_str()),
            Some("permission denied")
        );
    }
}
