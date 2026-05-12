//! Server-Sent Events (SSE) infrastructure.
//!
//! Provides a broadcast-based SSE endpoint that mirrors the Tauri `emit()`
//! mechanism used by the GUI. Clients connect via `GET /api/v1/events` and
//! receive real-time events for chat progress, completions, errors,
//! permission requests, title updates, diagnostics, and knowledge ingestion.

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// SSE event types
// ---------------------------------------------------------------------------

/// Unified SSE event enum broadcast to all connected clients.
///
/// Each variant corresponds to a Tauri `app.emit()` event type used by
/// the GUI frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum SseEvent {
    /// `chat:started` -- maps `run_id` to `session_id`.
    ChatStarted { run_id: String, session_id: String },
    /// `chat:progress` -- real-time turn diagnostics.
    ChatProgress {
        run_id: String,
        event: serde_json::Value,
    },
    /// `chat:complete` -- full response on success.
    ChatComplete(serde_json::Value),
    /// `chat:error` -- error details on failure.
    ChatError {
        run_id: String,
        session_id: String,
        error: String,
    },
    /// `chat:AskUser` -- LLM needs user input.
    AskUser {
        run_id: String,
        session_id: String,
        interaction_id: String,
        questions: serde_json::Value,
    },
    /// `chat:PermissionRequest` -- tool needs user approval.
    PermissionRequest {
        run_id: String,
        session_id: String,
        request_id: String,
        tool_name: String,
        action_description: String,
        reason: String,
        content_preview: Option<String>,
    },
    /// `chat:PlanReview` -- plan orchestrator needs user approval.
    PlanReviewRequest {
        run_id: String,
        session_id: String,
        review_id: String,
        plan: serde_json::Value,
    },
    /// `session:title_updated` -- auto-generated title is ready.
    TitleUpdated { session_id: String, title: String },
    /// `diagnostics:event` -- provider/tool/agent gateway event.
    DiagnosticsEvent(serde_json::Value),
    /// `kb:batch_progress` -- before each file in a batch ingest.
    KbBatchProgress {
        current: usize,
        total: usize,
        source: String,
    },
    /// `kb:entry_ingested` -- after each successfully ingested file.
    KbEntryIngested(serde_json::Value),
}

impl SseEvent {
    /// Return the SSE event name for this variant.
    fn event_name(&self) -> &'static str {
        match self {
            SseEvent::ChatStarted { .. } => "chat:started",
            SseEvent::ChatProgress { .. } => "chat:progress",
            SseEvent::ChatComplete(_) => "chat:complete",
            SseEvent::ChatError { .. } => "chat:error",
            SseEvent::AskUser { .. } => "chat:AskUser",
            SseEvent::PermissionRequest { .. } => "chat:PermissionRequest",
            SseEvent::PlanReviewRequest { .. } => "chat:PlanReview",
            SseEvent::TitleUpdated { .. } => "session:title_updated",
            SseEvent::DiagnosticsEvent(_) => "diagnostics:event",
            SseEvent::KbBatchProgress { .. } => "kb:batch_progress",
            SseEvent::KbEntryIngested(_) => "kb:entry_ingested",
        }
    }

    /// Extract the `session_id` if present (used for filtering).
    fn session_id(&self) -> Option<&str> {
        match self {
            SseEvent::ChatStarted { session_id, .. }
            | SseEvent::ChatError { session_id, .. }
            | SseEvent::AskUser { session_id, .. }
            | SseEvent::PermissionRequest { session_id, .. }
            | SseEvent::PlanReviewRequest { session_id, .. }
            | SseEvent::TitleUpdated { session_id, .. } => Some(session_id),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Optional query filter for the SSE endpoint.
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// When set, only events for this session are forwarded.
    pub session_id: Option<String>,
    /// Optional bearer token for authentication (alternative to header).
    pub token: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /api/v1/events` -- SSE stream of real-time events.
///
/// Clients receive all broadcast events (or a filtered subset when
/// `?session_id=xxx` is provided). The stream sends keep-alive comments
/// every 15 seconds to prevent proxy/load-balancer timeouts.
///
/// Authentication: supports `?token=xxx` query parameter as an alternative
/// to the `Authorization: Bearer` header (useful for `EventSource` which
/// cannot set custom headers).
async fn event_stream(State(state): State<AppState>, Query(query): Query<EventsQuery>) -> Response {
    // Validate token if auth is enabled.
    if let Some(ref expected_token) = state.auth_token {
        let provided_token = query.token.as_deref();
        if provided_token != Some(expected_token.as_str()) {
            return ApiError::Unauthorized("Invalid or missing token".to_string()).into_response();
        }
    }

    let mut rx = state.event_tx.subscribe();
    let filter_session = query.session_id;

    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Apply optional session filter.
                    if let Some(ref filter) = filter_session {
                        if let Some(sid) = event.session_id() {
                            if sid != filter {
                                continue;
                            }
                        }
                    }

                    let name = event.event_name().to_string();
                    if let Ok(json) = serde_json::to_string(&event) {
                        yield Ok::<_, std::convert::Infallible>(Event::default().event(name).data(json));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(lagged = n, "SSE client fell behind, skipped events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// SSE route group.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/events", get(event_stream))
}
