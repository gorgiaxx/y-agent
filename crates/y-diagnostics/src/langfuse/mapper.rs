//! Maps y-diagnostics Trace + Observations to OTLP JSON spans.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::types::{Observation, ObservationStatus, ObservationType, Score, Trace, TraceStatus};

use super::config::LangfuseConfig;
use super::redaction::RedactionPipeline;
use super::types::{
    ExportTraceServiceRequest, InstrumentationScope, KeyValue, Resource, ResourceSpans, ScopeSpans,
    Span, SpanEvent, SpanKind, SpanStatus,
};

pub struct OtelSpanMapper {
    config: LangfuseConfig,
    redaction: RedactionPipeline,
}

impl OtelSpanMapper {
    pub fn new(config: LangfuseConfig) -> Self {
        let redaction = RedactionPipeline::new(&config.redaction);
        Self { config, redaction }
    }

    pub fn map_trace(
        &self,
        trace: &Trace,
        observations: &[Observation],
        scores: &[Score],
    ) -> ExportTraceServiceRequest {
        let trace_id_hex = format!("{:032x}", trace.id.as_u128());
        let root_span_id = generate_span_id();

        let mut obs_span_ids: HashMap<Uuid, String> = HashMap::new();
        let mut spans = Vec::with_capacity(observations.len() + 1);

        // Root span for the trace.
        let root_span = self.build_root_span(trace, &trace_id_hex, &root_span_id, scores);
        spans.push(root_span);

        // Child spans for each observation.
        for obs in observations {
            let span_id = generate_span_id();
            obs_span_ids.insert(obs.id, span_id.clone());

            let parent_id = obs
                .parent_id
                .and_then(|pid| obs_span_ids.get(&pid).cloned())
                .unwrap_or_else(|| root_span_id.clone());

            let span = self.build_observation_span(obs, &trace_id_hex, &span_id, &parent_id);
            spans.push(span);
        }

        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Resource {
                    attributes: vec![
                        KeyValue::string("service.name", "y-agent"),
                        KeyValue::string("service.version", env!("CARGO_PKG_VERSION")),
                    ],
                },
                scope_spans: vec![ScopeSpans {
                    scope: InstrumentationScope {
                        name: "y-diagnostics.langfuse".to_string(),
                        version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    },
                    spans,
                }],
            }],
        }
    }

    fn build_root_span(
        &self,
        trace: &Trace,
        trace_id_hex: &str,
        span_id: &str,
        scores: &[Score],
    ) -> Span {
        let mut attributes = vec![
            KeyValue::string("langfuse.trace.id", trace.id.to_string()),
            KeyValue::string("langfuse.trace.name", &trace.name),
            KeyValue::string("langfuse.trace.session_id", trace.session_id.to_string()),
            KeyValue::string("langfuse.trace.base_url", &self.config.base_url),
            KeyValue::int("gen_ai.usage.input_tokens", trace.total_input_tokens.cast_signed()),
            KeyValue::int(
                "gen_ai.usage.output_tokens",
                trace.total_output_tokens.cast_signed(),
            ),
            KeyValue::double("langfuse.trace.cost_usd", trace.total_cost_usd),
        ];

        for tag in &trace.tags {
            attributes.push(KeyValue::string("langfuse.trace.tag", tag));
        }

        // Scores as attributes.
        for score in scores {
            let key = format!("langfuse.score.{}", score.name);
            match &score.value {
                crate::types::ScoreValue::Numeric(v) => {
                    attributes.push(KeyValue::double(&key, *v));
                }
                crate::types::ScoreValue::Boolean(v) => {
                    attributes.push(KeyValue::bool(&key, *v));
                }
                crate::types::ScoreValue::Categorical(v) => {
                    attributes.push(KeyValue::string(&key, v));
                }
            }
        }

        let status = match trace.status {
            TraceStatus::Completed => SpanStatus::ok(),
            TraceStatus::Failed => SpanStatus::error("trace failed"),
            _ => SpanStatus::unset(),
        };

        Span {
            trace_id: trace_id_hex.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: None,
            name: trace.name.clone(),
            kind: SpanKind::Server,
            start_time_unix_nano: datetime_to_nanos(trace.started_at),
            end_time_unix_nano: datetime_to_nanos(
                trace.completed_at.unwrap_or_else(Utc::now),
            ),
            attributes,
            events: Vec::new(),
            links: Vec::new(),
            status,
        }
    }

    fn build_observation_span(
        &self,
        obs: &Observation,
        trace_id_hex: &str,
        span_id: &str,
        parent_span_id: &str,
    ) -> Span {
        let mut attributes = vec![
            KeyValue::string("langfuse.observation.id", obs.id.to_string()),
            KeyValue::string("langfuse.observation.type", format!("{:?}", obs.obs_type)),
        ];

        let (name, kind) = match obs.obs_type {
            ObservationType::Generation => {
                self.add_generation_attributes(obs, &mut attributes);
                (
                    obs.model
                        .as_deref()
                        .unwrap_or("llm-generation")
                        .to_string(),
                    SpanKind::Client,
                )
            }
            ObservationType::ToolCall => {
                self.add_tool_attributes(obs, &mut attributes);
                (format!("tool:{}", obs.name), SpanKind::Internal)
            }
            _ => (obs.name.clone(), SpanKind::Internal),
        };

        let mut events = Vec::new();
        if self.config.content.capture_input && !obs.input.is_null() {
            events.extend(self.build_content_events(obs, trace_id_hex));
        }

        let started = obs.started_at;
        let ended = obs.completed_at.unwrap_or(started);

        let status = match obs.status {
            ObservationStatus::Completed => SpanStatus::ok(),
            ObservationStatus::Failed => SpanStatus::error("observation failed"),
            ObservationStatus::Running => SpanStatus::unset(),
        };

        Span {
            trace_id: trace_id_hex.to_string(),
            span_id: span_id.to_string(),
            parent_span_id: Some(parent_span_id.to_string()),
            name,
            kind,
            start_time_unix_nano: datetime_to_nanos(started),
            end_time_unix_nano: datetime_to_nanos(ended),
            attributes,
            events,
            links: Vec::new(),
            status,
        }
    }

    fn add_generation_attributes(&self, obs: &Observation, attrs: &mut Vec<KeyValue>) {
        if let Some(model) = &obs.model {
            attrs.push(KeyValue::string("gen_ai.request.model", model));
            attrs.push(KeyValue::string("gen_ai.response.model", model));
        }
        attrs.push(KeyValue::int(
            "gen_ai.usage.input_tokens",
            obs.input_tokens.cast_signed(),
        ));
        attrs.push(KeyValue::int(
            "gen_ai.usage.output_tokens",
            obs.output_tokens.cast_signed(),
        ));
        attrs.push(KeyValue::double("gen_ai.cost_usd", obs.cost_usd));
        if let Some(dur) = obs.metadata.get("duration_ms").and_then(serde_json::Value::as_u64) {
            attrs.push(KeyValue::int("gen_ai.duration_ms", dur.cast_signed()));
        }
        attrs.push(KeyValue::string("langfuse.base_url", &self.config.base_url));
    }

    fn add_tool_attributes(&self, obs: &Observation, attrs: &mut Vec<KeyValue>) {
        attrs.push(KeyValue::string("tool.name", &obs.name));
        if let Some(dur) = obs.metadata.get("duration_ms").and_then(serde_json::Value::as_u64) {
            attrs.push(KeyValue::int("tool.duration_ms", dur.cast_signed()));
        }
        let success = obs.status == ObservationStatus::Completed;
        attrs.push(KeyValue::bool("tool.success", success));
        attrs.push(KeyValue::string("langfuse.base_url", &self.config.base_url));
    }

    fn build_content_events(&self, obs: &Observation, _trace_id_hex: &str) -> Vec<SpanEvent> {
        let mut events = Vec::new();
        let time = datetime_to_nanos(obs.started_at);

        if self.config.content.capture_input && !obs.input.is_null() {
            if let Some(messages) = obs.input.get("messages").and_then(|m| m.as_array()) {
                for msg in messages {
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("unknown");
                    let content = msg
                        .get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or_default();
                    let redacted = self.maybe_redact(content);
                    let event_name = format!("gen_ai.{role}.message");
                    events.push(SpanEvent {
                        name: event_name,
                        time_unix_nano: time.clone(),
                        attributes: vec![KeyValue::string("gen_ai.content", redacted)],
                    });
                }
            }
        }

        if self.config.content.capture_output && !obs.output.is_null() {
            let output_str = if let Some(content) = obs.output.get("content").and_then(|c| c.as_str()) {
                content.to_string()
            } else {
                obs.output.to_string()
            };
            let redacted = self.maybe_redact(&output_str);
            let end_time = obs
                .completed_at
                .map_or_else(|| time.clone(), datetime_to_nanos);
            events.push(SpanEvent {
                name: "gen_ai.choice".to_string(),
                time_unix_nano: end_time,
                attributes: vec![KeyValue::string("gen_ai.content", redacted)],
            });
        }

        events
    }

    fn maybe_redact(&self, content: &str) -> String {
        let redacted = if self.config.redaction.enabled {
            self.redaction.redact(content)
        } else {
            content.to_string()
        };
        if redacted.len() > self.config.content.max_content_length {
            redacted[..self.config.content.max_content_length].to_string()
        } else {
            redacted
        }
    }
}

fn generate_span_id() -> String {
    let bytes: [u8; 8] = rand_bytes();
    hex::encode(bytes)
}

fn rand_bytes() -> [u8; 8] {
    let id = Uuid::new_v4();
    let bytes = id.as_bytes();
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes[..8]);
    out
}

fn datetime_to_nanos(dt: DateTime<Utc>) -> String {
    let secs = dt.timestamp() as u128;
    let nanos = u128::from(dt.timestamp_subsec_nanos());
    (secs * 1_000_000_000 + nanos).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Observation, ObservationType, Trace};

    #[test]
    fn test_map_empty_trace() {
        let config = LangfuseConfig::default();
        let mapper = OtelSpanMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");
        let result = mapper.map_trace(&trace, &[], &[]);
        assert_eq!(result.resource_spans.len(), 1);
        assert_eq!(result.resource_spans[0].scope_spans[0].spans.len(), 1);
    }

    #[test]
    fn test_map_trace_with_observations() {
        let config = LangfuseConfig::default();
        let mapper = OtelSpanMapper::new(config);
        let trace = Trace::new(Uuid::new_v4(), "test");
        let mut obs = Observation::new(trace.id, ObservationType::Generation, "gpt-4");
        obs.model = Some("gpt-4".to_string());
        obs.input_tokens = 100;
        obs.output_tokens = 50;
        let result = mapper.map_trace(&trace, &[obs], &[]);
        assert_eq!(result.resource_spans[0].scope_spans[0].spans.len(), 2);
    }
}
