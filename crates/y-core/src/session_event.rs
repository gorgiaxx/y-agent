//! Shared contracts for durable, replayable session events.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::SessionId;

/// Product-level event kinds persisted independently of presentation transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    ChatStarted,
    ChatProgress,
    ChatComplete,
    ChatError,
    AskUser,
    PermissionRequest,
    PlanReviewRequest,
    ToolRuntime,
}

impl SessionEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatStarted => "chat_started",
            Self::ChatProgress => "chat_progress",
            Self::ChatComplete => "chat_complete",
            Self::ChatError => "chat_error",
            Self::AskUser => "ask_user",
            Self::PermissionRequest => "permission_request",
            Self::PlanReviewRequest => "plan_review_request",
            Self::ToolRuntime => "tool_runtime",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat_started" => Some(Self::ChatStarted),
            "chat_progress" => Some(Self::ChatProgress),
            "chat_complete" => Some(Self::ChatComplete),
            "chat_error" => Some(Self::ChatError),
            "ask_user" => Some(Self::AskUser),
            "permission_request" => Some(Self::PermissionRequest),
            "plan_review_request" => Some(Self::PlanReviewRequest),
            "tool_runtime" => Some(Self::ToolRuntime),
            _ => None,
        }
    }
}

/// Retention class for persisted session events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventRetention {
    Durable,
    ShortLived,
    Reconstructable,
}

impl SessionEventRetention {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::ShortLived => "short_lived",
            Self::Reconstructable => "reconstructable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "durable" => Some(Self::Durable),
            "short_lived" => Some(Self::ShortLived),
            "reconstructable" => Some(Self::Reconstructable),
            _ => None,
        }
    }
}

/// Input for appending one session event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSessionEvent {
    pub session_id: SessionId,
    pub kind: SessionEventKind,
    pub payload: serde_json::Value,
    pub retention: SessionEventRetention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
}

/// Persisted event envelope with global and session-local ordering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedSessionEvent {
    pub event_id: u64,
    pub session_id: SessionId,
    pub seq: u64,
    pub kind: SessionEventKind,
    pub payload: serde_json::Value,
    pub retention: SessionEventRetention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
