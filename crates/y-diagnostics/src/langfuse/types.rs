//! OTLP JSON types matching the OpenTelemetry OTLP/HTTP JSON schema.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTraceServiceRequest {
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpans {
    pub resource: Resource,
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSpans {
    pub scope: InstrumentationScope,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentationScope {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: SpanKind,
    pub start_time_unix_nano: String,
    pub end_time_unix_nano: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SpanEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<SpanLink>,
    pub status: SpanStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SpanKind {
    #[serde(rename = "SPAN_KIND_INTERNAL")]
    Internal,
    #[serde(rename = "SPAN_KIND_SERVER")]
    Server,
    #[serde(rename = "SPAN_KIND_CLIENT")]
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanEvent {
    pub name: String,
    pub time_unix_nano: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpanLink {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanStatus {
    pub code: SpanStatusCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SpanStatusCode {
    #[serde(rename = "STATUS_CODE_UNSET")]
    Unset,
    #[serde(rename = "STATUS_CODE_OK")]
    Ok,
    #[serde(rename = "STATUS_CODE_ERROR")]
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnyValue {
    StringValue(String),
    IntValue(String),
    DoubleValue(f64),
    BoolValue(bool),
    ArrayValue(ArrayValue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayValue {
    pub values: Vec<AnyValue>,
}

// ─── Helper constructors ─────────────────────────────────────

impl KeyValue {
    pub fn string(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::StringValue(value.into()),
        }
    }

    pub fn int(key: impl Into<String>, value: i64) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::IntValue(value.to_string()),
        }
    }

    pub fn double(key: impl Into<String>, value: f64) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::DoubleValue(value),
        }
    }

    pub fn bool(key: impl Into<String>, value: bool) -> Self {
        Self {
            key: key.into(),
            value: AnyValue::BoolValue(value),
        }
    }
}

impl SpanStatus {
    pub fn ok() -> Self {
        Self {
            code: SpanStatusCode::Ok,
            message: None,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            code: SpanStatusCode::Error,
            message: Some(msg.into()),
        }
    }

    pub fn unset() -> Self {
        Self {
            code: SpanStatusCode::Unset,
            message: None,
        }
    }
}
