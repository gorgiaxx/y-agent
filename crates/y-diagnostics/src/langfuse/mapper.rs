//! Maps y-diagnostics domain types to Langfuse native ingestion events.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::types::{Observation, ObservationStatus, ObservationType, Score, Trace};

use super::config::LangfuseConfig;
use super::redaction::RedactionPipeline;
use super::types::{
    CreateGenerationBody, CreateSpanBody, IngestionBatchRequest, IngestionEvent, ObservationLevel,
    ScoreBody, ScoreDataType, TraceBody,
};

pub struct LangfuseIngestionMapper {
    config: LangfuseConfig,
    redaction: RedactionPipeline,
}

impl LangfuseIngestionMapper {
    pub fn new(config: LangfuseConfig) -> Self {
        let redaction = RedactionPipeline::new(&config.redaction);
        Self { config, redaction }
    }

    pub fn map_trace(
        &self,
        trace: &Trace,
        observations: &[Observation],
        scores: &[Score],
    ) -> IngestionBatchRequest {
        let mut batch = Vec::with_capacity(1 + observations.len() + scores.len());

        batch.push(Self::build_trace_event(trace));

        let obs_id_map: std::collections::HashMap<Uuid, String> = observations
            .iter()
            .map(|obs| (obs.id, obs.id.to_string()))
            .collect();

        for obs in observations {
            let parent_obs_id = obs.parent_id.and_then(|pid| obs_id_map.get(&pid).cloned());
            let event = match obs.obs_type {
                ObservationType::Generation => {
                    self.build_generation_event(trace, obs, parent_obs_id)
                }
                _ => self.build_span_event(trace, obs, parent_obs_id),
            };
            batch.push(event);
        }

        for score in scores {
            batch.push(Self::build_score_event(trace.id, score));
        }

        IngestionBatchRequest { batch }
    }

    pub fn map_trace_create(&self, trace: &Trace) -> IngestionEvent {
        Self::build_trace_event(trace)
    }

    pub fn map_trace_update(&self, trace: &Trace) -> IngestionEvent {
        let body = TraceBody {
            id: Some(trace.id.to_string()),
            timestamp: Some(to_iso8601(trace.started_at)),
            name: Some(trace.name.clone()),
            user_id: None,
            session_id: Some(trace.session_id.to_string()),
            input: None,
            output: trace.metadata.get("output").cloned(),
            metadata: if trace.metadata.is_null() {
                None
            } else {
                Some(trace.metadata.clone())
            },
            tags: if trace.tags.is_empty() {
                None
            } else {
                Some(trace.tags.clone())
            },
            release: Some(env!("CARGO_PKG_VERSION").to_string()),
            version: None,
            environment: None,
            is_public: None,
        };

        IngestionEvent {
            id: Uuid::new_v4().to_string(),
            event_type: "trace-create".to_string(),
            timestamp: to_iso8601(Utc::now()),
            body: serde_json::to_value(body).unwrap_or_default(),
        }
    }

    pub fn map_observation(&self, trace: &Trace, obs: &Observation) -> IngestionEvent {
        let parent_obs_id = obs.parent_id.map(|pid| pid.to_string());
        match obs.obs_type {
            ObservationType::Generation => self.build_generation_event(trace, obs, parent_obs_id),
            _ => self.build_span_event(trace, obs, parent_obs_id),
        }
    }

    fn build_trace_event(trace: &Trace) -> IngestionEvent {
        let input = trace
            .user_input
            .as_ref()
            .map(|s| serde_json::Value::String(s.clone()));

        let output = trace.metadata.get("output").cloned();

        let body = TraceBody {
            id: Some(trace.id.to_string()),
            timestamp: Some(to_iso8601(trace.started_at)),
            name: Some(trace.name.clone()),
            user_id: None,
            session_id: Some(trace.session_id.to_string()),
            input,
            output,
            metadata: if trace.metadata.is_null() {
                None
            } else {
                Some(trace.metadata.clone())
            },
            tags: if trace.tags.is_empty() {
                None
            } else {
                Some(trace.tags.clone())
            },
            release: Some(env!("CARGO_PKG_VERSION").to_string()),
            version: None,
            environment: None,
            is_public: None,
        };

        IngestionEvent {
            id: Uuid::new_v4().to_string(),
            event_type: "trace-create".to_string(),
            timestamp: to_iso8601(Utc::now()),
            body: serde_json::to_value(body).unwrap_or_default(),
        }
    }

    fn build_generation_event(
        &self,
        trace: &Trace,
        obs: &Observation,
        parent_obs_id: Option<String>,
    ) -> IngestionEvent {
        let input = self.prepare_content(&obs.input, true);
        let output = self.prepare_content(&obs.output, false);

        let mut usage_details = std::collections::HashMap::new();
        usage_details.insert("input".to_string(), obs.input_tokens);
        usage_details.insert("output".to_string(), obs.output_tokens);
        usage_details.insert("total".to_string(), obs.input_tokens + obs.output_tokens);

        let cost_details = if obs.cost_usd > 0.0 {
            let mut m = std::collections::HashMap::new();
            m.insert("total".to_string(), obs.cost_usd);
            Some(m)
        } else {
            None
        };

        let level = match obs.status {
            ObservationStatus::Failed => Some(ObservationLevel::Error),
            _ => None,
        };

        let status_message = obs.error_message.clone();

        let model_parameters = obs.metadata.get("model_parameters").and_then(|v| {
            serde_json::from_value::<std::collections::HashMap<String, serde_json::Value>>(
                v.clone(),
            )
            .ok()
        });

        let body = CreateGenerationBody {
            id: Some(obs.id.to_string()),
            trace_id: Some(trace.id.to_string()),
            parent_observation_id: parent_obs_id,
            name: obs
                .model
                .clone()
                .or_else(|| Some("llm-generation".to_string())),
            start_time: Some(to_iso8601(obs.started_at)),
            end_time: obs.completed_at.map(to_iso8601),
            completion_start_time: None,
            model: obs.model.clone(),
            model_parameters,
            input,
            output,
            usage_details: Some(usage_details),
            cost_details,
            level,
            status_message,
            metadata: Self::observation_metadata(obs),
            version: None,
            environment: None,
        };

        IngestionEvent {
            id: Uuid::new_v4().to_string(),
            event_type: "generation-create".to_string(),
            timestamp: to_iso8601(Utc::now()),
            body: serde_json::to_value(body).unwrap_or_default(),
        }
    }

    fn build_span_event(
        &self,
        trace: &Trace,
        obs: &Observation,
        parent_obs_id: Option<String>,
    ) -> IngestionEvent {
        let input = self.prepare_content(&obs.input, true);
        let output = self.prepare_content(&obs.output, false);

        let level = match obs.status {
            ObservationStatus::Failed => Some(ObservationLevel::Error),
            _ => None,
        };

        let body = CreateSpanBody {
            id: Some(obs.id.to_string()),
            trace_id: Some(trace.id.to_string()),
            parent_observation_id: parent_obs_id,
            name: Some(obs.name.clone()),
            start_time: Some(to_iso8601(obs.started_at)),
            end_time: obs.completed_at.map(to_iso8601),
            input,
            output,
            level,
            status_message: obs.error_message.clone(),
            metadata: Self::observation_metadata(obs),
            version: None,
            environment: None,
        };

        IngestionEvent {
            id: Uuid::new_v4().to_string(),
            event_type: "span-create".to_string(),
            timestamp: to_iso8601(Utc::now()),
            body: serde_json::to_value(body).unwrap_or_default(),
        }
    }

    fn build_score_event(trace_id: Uuid, score: &Score) -> IngestionEvent {
        let (value, data_type) = match &score.value {
            crate::types::ScoreValue::Numeric(v) => {
                (serde_json::json!(*v), Some(ScoreDataType::Numeric))
            }
            crate::types::ScoreValue::Boolean(v) => {
                let num = if *v { 1.0 } else { 0.0 };
                (serde_json::json!(num), Some(ScoreDataType::Boolean))
            }
            crate::types::ScoreValue::Categorical(v) => {
                (serde_json::json!(v), Some(ScoreDataType::Categorical))
            }
        };

        let body = ScoreBody {
            id: Some(score.id.to_string()),
            trace_id: Some(trace_id.to_string()),
            observation_id: score.observation_id.map(|id| id.to_string()),
            name: score.name.clone(),
            value,
            data_type,
            comment: score.comment.clone(),
            metadata: None,
            environment: None,
            source: Some(format!("{:?}", score.source)),
        };

        IngestionEvent {
            id: Uuid::new_v4().to_string(),
            event_type: "score-create".to_string(),
            timestamp: to_iso8601(Utc::now()),
            body: serde_json::to_value(body).unwrap_or_default(),
        }
    }

    fn observation_metadata(obs: &Observation) -> Option<serde_json::Value> {
        if obs.metadata.is_null() {
            return None;
        }
        Some(obs.metadata.clone())
    }

    fn prepare_content(
        &self,
        value: &serde_json::Value,
        is_input: bool,
    ) -> Option<serde_json::Value> {
        if value.is_null() {
            return None;
        }

        let should_capture = if is_input {
            self.config.content.capture_input
        } else {
            self.config.content.capture_output
        };

        if !should_capture {
            return None;
        }

        if !self.config.redaction.enabled {
            return Some(truncate_json_value(
                value,
                self.config.content.max_content_length,
            ));
        }

        Some(truncate_json_value(
            &redact_json_value(value, &self.redaction),
            self.config.content.max_content_length,
        ))
    }
}

fn redact_json_value(
    value: &serde_json::Value,
    redaction: &RedactionPipeline,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redaction.redact(s)),
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| redact_json_value(v, redaction))
                .collect(),
        ),
        serde_json::Value::Object(map) => {
            let redacted: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), redact_json_value(v, redaction)))
                .collect();
            serde_json::Value::Object(redacted)
        }
        other => other.clone(),
    }
}

fn truncate_json_value(value: &serde_json::Value, max_len: usize) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) if s.len() > max_len => {
            serde_json::Value::String(s[..max_len].to_string())
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|v| truncate_json_value(v, max_len))
                .collect(),
        ),
        serde_json::Value::Object(map) => {
            let truncated: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), truncate_json_value(v, max_len)))
                .collect();
            serde_json::Value::Object(truncated)
        }
        other => other.clone(),
    }
}

fn to_iso8601(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Observation, ObservationType, Trace};

    #[test]
    fn test_map_empty_trace() {
        let mut config = LangfuseConfig::default();
        config.content.capture_input = true;
        config.content.capture_output = true;
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");
        let result = mapper.map_trace(&trace, &[], &[]);
        assert_eq!(result.batch.len(), 1);
        assert_eq!(result.batch[0].event_type, "trace-create");
    }

    #[test]
    fn test_map_trace_with_generation() {
        let mut config = LangfuseConfig::default();
        config.content.capture_input = true;
        config.content.capture_output = true;
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");

        let mut obs = Observation::new(trace.id, ObservationType::Generation, "gpt-4");
        obs.model = Some("gpt-4".to_string());
        obs.input_tokens = 100;
        obs.output_tokens = 50;
        obs.input = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        obs.output = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hi there!"
                }
            }]
        });

        let result = mapper.map_trace(&trace, &[obs], &[]);
        assert_eq!(result.batch.len(), 2);
        assert_eq!(result.batch[0].event_type, "trace-create");
        assert_eq!(result.batch[1].event_type, "generation-create");

        let gen_body = &result.batch[1].body;
        assert_eq!(gen_body["model"], "gpt-4");
        assert!(gen_body["input"]["messages"].is_array());
        assert!(gen_body["output"]["choices"].is_array());
        assert_eq!(gen_body["usageDetails"]["input"], 100);
        assert_eq!(gen_body["usageDetails"]["output"], 50);
        assert_eq!(gen_body["usageDetails"]["total"], 150);
    }

    #[test]
    fn test_map_trace_with_tool_call() {
        let mut config = LangfuseConfig::default();
        config.content.capture_input = true;
        config.content.capture_output = true;
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");

        let gen_obs = Observation::new(trace.id, ObservationType::Generation, "gpt-4");
        let mut tool_obs = Observation::new(trace.id, ObservationType::ToolCall, "WebSearch");
        tool_obs.parent_id = Some(gen_obs.id);
        tool_obs.input = serde_json::json!({"query": "rust language"});
        tool_obs.output = serde_json::json!({"results": ["https://rust-lang.org"]});

        let result = mapper.map_trace(&trace, &[gen_obs, tool_obs], &[]);
        assert_eq!(result.batch.len(), 3);
        assert_eq!(result.batch[2].event_type, "span-create");

        let span_body = &result.batch[2].body;
        assert_eq!(span_body["name"], "WebSearch");
        assert_eq!(span_body["input"]["query"], "rust language");
        assert!(span_body["output"]["results"].is_array());
        assert_eq!(
            span_body["parentObservationId"].as_str().unwrap(),
            result.batch[1].body["id"].as_str().unwrap()
        );
    }

    #[test]
    fn test_map_trace_with_subagent_span_child_trace_metadata() {
        let mut config = LangfuseConfig::default();
        config.content.capture_input = true;
        config.content.capture_output = true;
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "chat-turn");
        let child_trace_id = Uuid::new_v4();

        let mut subagent_obs = Observation::new(
            trace.id,
            ObservationType::SubAgent,
            "agent.delegate.plan-writer",
        );
        subagent_obs.input = serde_json::json!({"task": "make a plan"});
        subagent_obs.output = serde_json::json!({"content": "plan complete"});
        subagent_obs.metadata = serde_json::json!({
            "agent_name": "plan-writer",
            "child_trace_id": child_trace_id.to_string(),
        });

        let result = mapper.map_trace(&trace, &[subagent_obs], &[]);
        assert_eq!(result.batch.len(), 2);
        assert_eq!(result.batch[1].event_type, "span-create");

        let span_body = &result.batch[1].body;
        assert_eq!(span_body["name"], "agent.delegate.plan-writer");
        assert_eq!(span_body["input"]["task"], "make a plan");
        assert_eq!(
            span_body["metadata"]["child_trace_id"],
            child_trace_id.to_string()
        );
    }

    #[test]
    fn test_map_trace_with_generation_tool_calls_in_output() {
        let mut config = LangfuseConfig::default();
        config.content.capture_input = true;
        config.content.capture_output = true;
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");

        let mut obs = Observation::new(trace.id, ObservationType::Generation, "gpt-4");
        obs.model = Some("gpt-4".to_string());
        obs.output = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\": \"NYC\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let result = mapper.map_trace(&trace, &[obs], &[]);
        let gen_body = &result.batch[1].body;
        let tool_calls = &gen_body["output"]["choices"][0]["message"]["tool_calls"];
        assert!(tool_calls.is_array());
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_content_not_captured_when_disabled() {
        let config = LangfuseConfig::default();
        let mapper = LangfuseIngestionMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");

        let mut obs = Observation::new(trace.id, ObservationType::Generation, "gpt-4");
        obs.input = serde_json::json!({"messages": [{"role": "user", "content": "secret"}]});
        obs.output = serde_json::json!({"content": "response"});

        let result = mapper.map_trace(&trace, &[obs], &[]);
        let gen_body = &result.batch[1].body;
        assert!(gen_body.get("input").is_none() || gen_body["input"].is_null());
        assert!(gen_body.get("output").is_none() || gen_body["output"].is_null());
    }
}
