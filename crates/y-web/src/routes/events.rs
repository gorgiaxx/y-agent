//! Server-Sent Events (SSE) infrastructure.
//!
//! Provides a broadcast-based SSE endpoint that mirrors the Tauri `emit()`
//! mechanism used by the GUI. Clients connect via `GET /api/v1/events` and
//! receive real-time events for chat progress, completions, errors,
//! permission requests, title updates, diagnostics, and knowledge ingestion.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use y_core::runtime::ToolRuntimeEvent;
use y_core::session_event::{PersistedSessionEvent, SessionEventKind};
use y_core::types::SessionId;
use y_service::{BackgroundWakeEvent, SessionEventService};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// SSE event types
// ---------------------------------------------------------------------------

/// Unified SSE event enum broadcast to all connected clients.
///
/// Each variant corresponds to a Tauri `app.emit()` event type used by
/// the GUI frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum SseEvent {
    /// `chat:started` -- maps `run_id` to `session_id`.
    ///
    /// `kind` distinguishes the run source: `chat` for a normal LLM turn,
    /// `plan_resume` for a background plan-execution retry.
    ChatStarted {
        run_id: String,
        session_id: String,
        #[serde(skip_serializing_if = "is_default_chat_kind")]
        kind: String,
    },
    /// `chat:progress` -- real-time turn diagnostics.
    ChatProgress {
        run_id: String,
        event: serde_json::Value,
        /// Originating sub-agent (child) session id, when the event came from a
        /// plan phase / loop round / plan-writer running under a child session.
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
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
    /// `chat:follow_up_queue` -- a session's follow-up queue changed.
    FollowUpQueueUpdated {
        session_id: String,
        queue: serde_json::Value,
    },
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
    /// `tool:runtime` -- process output and lifecycle notification.
    ToolRuntime(ToolRuntimeEvent),
}

/// Live SSE delivery envelope carrying the durable database cursor when present.
#[derive(Debug, Clone)]
pub struct SseEnvelope {
    pub event: SseEvent,
    pub event_id: Option<u64>,
    pub session_id: Option<String>,
}

impl SseEnvelope {
    pub fn new(event: SseEvent, event_id: Option<u64>) -> Self {
        let session_id = event.session_id().map(str::to_owned);
        Self {
            event,
            event_id,
            session_id,
        }
    }

    pub fn for_session(
        event: SseEvent,
        event_id: Option<u64>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            event,
            event_id,
            session_id: Some(session_id.into()),
        }
    }

    fn from_persisted(record: PersistedSessionEvent) -> Result<Self, serde_json::Error> {
        let event_type = match record.kind {
            SessionEventKind::ChatStarted => "ChatStarted",
            SessionEventKind::ChatProgress => "ChatProgress",
            SessionEventKind::ChatComplete => "ChatComplete",
            SessionEventKind::ChatError => "ChatError",
            SessionEventKind::AskUser => "AskUser",
            SessionEventKind::PermissionRequest => "PermissionRequest",
            SessionEventKind::PlanReviewRequest => "PlanReviewRequest",
            SessionEventKind::ToolRuntime => "ToolRuntime",
        };
        let event = serde_json::from_value(serde_json::json!({
            "type": event_type,
            "data": record.payload,
        }))?;
        Ok(Self::for_session(
            event,
            Some(record.event_id),
            record.session_id.0,
        ))
    }
}

impl From<SseEvent> for SseEnvelope {
    fn from(event: SseEvent) -> Self {
        Self::new(event, None)
    }
}

fn background_wake_envelope(event: &BackgroundWakeEvent) -> Result<SseEnvelope, serde_json::Error> {
    let event_type = match event.event_name() {
        "chat:started" => "ChatStarted",
        "chat:progress" => "ChatProgress",
        "chat:complete" => "ChatComplete",
        "chat:error" => "ChatError",
        "chat:AskUser" => "AskUser",
        "chat:PermissionRequest" => "PermissionRequest",
        "chat:PlanReview" => "PlanReviewRequest",
        "session:title_updated" => "TitleUpdated",
        _ => unreachable!("background wake emitted an unsupported chat event"),
    };
    let event_id = event.event_id();
    let session_id = event.session_id().to_string();
    let event = serde_json::from_value(serde_json::json!({
        "type": event_type,
        "data": event.payload(),
    }))?;
    Ok(SseEnvelope::for_session(event, event_id, session_id))
}

fn is_default_chat_kind(kind: &str) -> bool {
    kind == "chat"
}

impl SseEvent {
    /// Return the SSE event name for this variant.
    fn event_name(&self) -> &'static str {
        match self {
            SseEvent::ChatStarted { .. } => "chat:started",
            SseEvent::ChatProgress { .. } => "chat:progress",
            SseEvent::ChatComplete(_) => "chat:complete",
            SseEvent::ChatError { .. } => "chat:error",
            SseEvent::FollowUpQueueUpdated { .. } => "chat:follow_up_queue",
            SseEvent::AskUser { .. } => "chat:AskUser",
            SseEvent::PermissionRequest { .. } => "chat:PermissionRequest",
            SseEvent::PlanReviewRequest { .. } => "chat:PlanReview",
            SseEvent::TitleUpdated { .. } => "session:title_updated",
            SseEvent::DiagnosticsEvent(_) => "diagnostics:event",
            SseEvent::KbBatchProgress { .. } => "kb:batch_progress",
            SseEvent::KbEntryIngested(_) => "kb:entry_ingested",
            SseEvent::ToolRuntime(_) => "tool:runtime",
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
            | SseEvent::TitleUpdated { session_id, .. }
            | SseEvent::FollowUpQueueUpdated { session_id, .. } => Some(session_id),
            SseEvent::ToolRuntime(event) => Some(event.session_id.as_str()),
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
    /// Durable event cursor used by clients that recreate `EventSource`.
    pub cursor: Option<u64>,
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
async fn event_stream(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(ref expected_token) = state.auth_token {
        let provided_token = query.token.as_deref();
        if provided_token != Some(expected_token.as_str()) {
            return ApiError::Unauthorized("Invalid or missing token".to_string()).into_response();
        }
    }

    let cursor = match resolve_cursor(query.cursor, &headers) {
        Ok(cursor) => cursor,
        Err(error) => return error.into_response(),
    };
    let filter_session = query.session_id.map(SessionId);

    let replay_floor = match cursor {
        Some(cursor) => cursor,
        None => match state
            .container
            .session_event_service
            .latest_event_id()
            .await
        {
            Ok(event_id) => event_id,
            Err(error) => return ApiError::Internal(error.to_string()).into_response(),
        },
    };
    // Establish the no-history floor first, then subscribe before querying
    // replay. Events racing either boundary are recovered from SQLite and
    // de-duplicated against both live channels.
    let mut chat_rx = state.event_tx.subscribe();
    let mut runtime_rx = state.container.tool_runtime_event_service.subscribe();
    let mut wake_rx = state.container.background_wake_service.subscribe();
    let pending = match cursor {
        Some(_) => Ok(Vec::new()),
        None => match filter_session.as_ref() {
            Some(session_id) => state
                .container
                .session_event_service
                .pending_events(&state.container.session_state, session_id)
                .await
                .map_err(|error| error.to_string()),
            None => Ok(Vec::new()),
        },
    };
    let pending = match pending {
        Ok(events) => events,
        Err(error) => return ApiError::Internal(error).into_response(),
    };
    let initial_replay = match load_replay(
        &state.container.session_event_service,
        replay_floor,
        filter_session.as_ref(),
    )
    .await
    {
        Ok(events) => events,
        Err(error) => return ApiError::Internal(error).into_response(),
    };
    let session_event_service = state.container.session_event_service.clone();

    let stream = async_stream::stream! {
        let mut replay_cursor = replay_floor;
        for record in pending {
            match SseEnvelope::from_persisted(record) {
                Ok(mut envelope) => {
                    envelope.event_id = None;
                    if let Some(event) = encode_event(&envelope) {
                        yield Ok::<_, std::convert::Infallible>(event);
                    }
                }
                Err(error) => tracing::error!(%error, "failed to decode replayed session event"),
            }
        }
        for record in initial_replay {
            match SseEnvelope::from_persisted(record) {
                Ok(envelope) => {
                    if advance_replay_cursor(&envelope, &mut replay_cursor) {
                        if let Some(event) = encode_event(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(event);
                        }
                    }
                }
                Err(error) => tracing::error!(%error, "failed to decode replayed session event"),
            }
        }

        macro_rules! replay_live_gap {
            () => {
                match load_replay(
                    &session_event_service,
                    replay_cursor,
                    filter_session.as_ref(),
                )
                .await
                {
                    Ok(records) => {
                        for record in records {
                            match SseEnvelope::from_persisted(record) {
                                Ok(envelope) => {
                                    if advance_replay_cursor(&envelope, &mut replay_cursor) {
                                        if let Some(event) = encode_event(&envelope) {
                                            yield Ok::<_, std::convert::Infallible>(event);
                                        }
                                    }
                                }
                                Err(error) => tracing::error!(
                                    %error,
                                    "failed to decode recovered session event"
                                ),
                            }
                        }
                    }
                    Err(error) => tracing::error!(
                        %error,
                        replay_cursor,
                        "failed to recover durable SSE events"
                    ),
                }
            };
        }

        let mut chat_open = true;
        let mut runtime_open = true;
        let mut wake_open = true;
        while chat_open || runtime_open || wake_open {
            tokio::select! {
                result = chat_rx.recv(), if chat_open => match result {
                    Ok(envelope) => {
                        if !matches_session(&envelope, filter_session.as_ref()) {
                            continue;
                        }
                        if let Some(event_id) = envelope.event_id {
                            if event_id > replay_cursor {
                                replay_live_gap!();
                            }
                        } else if let Some(event) = encode_event(&envelope) {
                            yield Ok::<_, std::convert::Infallible>(event);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, replay_cursor, "SSE chat channel lagged; replaying durable events");
                        replay_live_gap!();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => chat_open = false,
                },
                result = runtime_rx.recv(), if runtime_open => match result {
                    Ok(published) => {
                        let session_id = published.event.session_id.0.clone();
                        let envelope = SseEnvelope::for_session(
                            SseEvent::ToolRuntime(published.event),
                            Some(published.event_id),
                            session_id,
                        );
                        if matches_session(&envelope, filter_session.as_ref())
                            && published.event_id > replay_cursor
                        {
                            replay_live_gap!();
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, replay_cursor, "SSE runtime channel lagged; replaying durable events");
                        replay_live_gap!();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => runtime_open = false,
                },
                result = wake_rx.recv(), if wake_open => match result {
                    Ok(event) => match background_wake_envelope(&event) {
                        Ok(envelope) => {
                            if !matches_session(&envelope, filter_session.as_ref()) {
                                continue;
                            }
                            if let Some(event_id) = envelope.event_id {
                                if event_id > replay_cursor {
                                    replay_live_gap!();
                                }
                            } else if let Some(event) = encode_event(&envelope) {
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        }
                        Err(error) => tracing::error!(
                            %error,
                            "failed to translate background wake event"
                        ),
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, replay_cursor, "SSE background wake channel lagged; replaying durable events");
                        replay_live_gap!();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => wake_open = false,
                },
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn resolve_cursor(query_cursor: Option<u64>, headers: &HeaderMap) -> Result<Option<u64>, ApiError> {
    if query_cursor.is_some() {
        return Ok(query_cursor);
    }
    let Some(value) = headers.get("last-event-id") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::BadRequest("Last-Event-ID must be valid ASCII".to_string()))?;
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ApiError::BadRequest("Last-Event-ID must be an unsigned integer".to_string()))
}

async fn load_replay(
    service: &SessionEventService,
    mut cursor: u64,
    session_id: Option<&SessionId>,
) -> Result<Vec<PersistedSessionEvent>, String> {
    const BATCH_SIZE: usize = 1_000;
    let mut replay = Vec::new();
    loop {
        let batch = service
            .replay_after(cursor, session_id, BATCH_SIZE)
            .await
            .map_err(|error| error.to_string())?;
        let batch_len = batch.len();
        if let Some(last) = batch.last() {
            cursor = last.event_id;
        }
        replay.extend(batch);
        if batch_len < BATCH_SIZE {
            break;
        }
    }
    Ok(replay)
}

fn matches_session(envelope: &SseEnvelope, filter: Option<&SessionId>) -> bool {
    filter.is_none_or(|session_id| envelope.session_id.as_deref() == Some(session_id.as_str()))
}

fn advance_replay_cursor(envelope: &SseEnvelope, replay_cursor: &mut u64) -> bool {
    let Some(event_id) = envelope.event_id else {
        return true;
    };
    if event_id <= *replay_cursor {
        return false;
    }
    *replay_cursor = event_id;
    true
}

fn encode_event(envelope: &SseEnvelope) -> Option<Event> {
    let json = serde_json::to_string(&envelope.event).ok()?;
    let event = Event::default()
        .event(envelope.event.event_name())
        .data(json);
    Some(match envelope.event_id {
        Some(event_id) => event.id(event_id.to_string()),
        None => event,
    })
}

/// SSE route group.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/events", get(event_stream))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::Utc;
    use y_core::session_event::{PersistedSessionEvent, SessionEventKind, SessionEventRetention};
    use y_core::types::SessionId;
    use y_service::BackgroundWakeEvent;

    use super::{
        advance_replay_cursor, background_wake_envelope, resolve_cursor, SseEnvelope, SseEvent,
    };

    #[test]
    fn query_cursor_takes_precedence_over_last_event_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("41"));

        assert_eq!(resolve_cursor(Some(73), &headers).unwrap(), Some(73));
    }

    #[test]
    fn last_event_id_header_restores_cursor_when_query_is_absent() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("41"));

        assert_eq!(resolve_cursor(None, &headers).unwrap(), Some(41));
    }

    #[test]
    fn invalid_last_event_id_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("not-a-number"));

        assert!(resolve_cursor(None, &headers).is_err());
    }

    #[test]
    fn persisted_events_restore_transport_shape_and_root_session() {
        let envelope = SseEnvelope::from_persisted(PersistedSessionEvent {
            event_id: 41,
            session_id: SessionId("session-1".into()),
            seq: 3,
            kind: SessionEventKind::ChatProgress,
            payload: serde_json::json!({
                "run_id": "run-1",
                "event": { "type": "ToolStart" },
                "session_id": "child-1",
            }),
            retention: SessionEventRetention::Durable,
            correlation_id: None,
            created_at: Utc::now(),
        })
        .unwrap();

        assert_eq!(envelope.event_id, Some(41));
        assert_eq!(envelope.session_id.as_deref(), Some("session-1"));
        assert!(matches!(
            envelope.event,
            SseEvent::ChatProgress {
                session_id: Some(ref session_id),
                ..
            } if session_id == "child-1"
        ));
    }

    #[test]
    fn replay_cursor_deduplicates_durable_events_but_allows_ephemeral_events() {
        let durable = SseEnvelope::for_session(
            SseEvent::ChatComplete(serde_json::json!({})),
            Some(41),
            "session-1",
        );
        let ephemeral = SseEnvelope::for_session(
            SseEvent::ChatProgress {
                run_id: "run-1".into(),
                event: serde_json::json!({ "type": "StreamDelta" }),
                session_id: None,
            },
            None,
            "session-1",
        );
        let mut cursor = 40;

        assert!(advance_replay_cursor(&durable, &mut cursor));
        assert_eq!(cursor, 41);
        assert!(!advance_replay_cursor(&durable, &mut cursor));
        assert!(advance_replay_cursor(&ephemeral, &mut cursor));
    }

    #[test]
    fn persisted_tool_runtime_event_restores_named_sse_payload() {
        let envelope = SseEnvelope::from_persisted(PersistedSessionEvent {
            event_id: 52,
            session_id: SessionId("session-1".into()),
            seq: 4,
            kind: SessionEventKind::ToolRuntime,
            payload: serde_json::json!({
                "session_id": "session-1",
                "task_id": "process-1",
                "tool_name": "ShellExec",
                "backend": "native",
                "occurred_at": "2026-07-17T00:00:00Z",
                "type": "process_completed",
                "exit_code": 0,
                "duration_ms": 125,
            }),
            retention: SessionEventRetention::Durable,
            correlation_id: Some("runtime:process-1".into()),
            created_at: Utc::now(),
        })
        .unwrap();

        assert!(matches!(
            envelope.event,
            SseEvent::ToolRuntime(ref event) if event.task_id == "process-1"
        ));
    }

    #[test]
    fn background_wake_event_maps_to_standard_chat_sse_shape() {
        let envelope = background_wake_envelope(&BackgroundWakeEvent::Started {
            run_id: "wake-run".into(),
            session_id: "session-1".into(),
            event_id: Some(61),
        })
        .unwrap();

        assert_eq!(envelope.event_id, Some(61));
        assert_eq!(envelope.session_id.as_deref(), Some("session-1"));
        assert!(matches!(
            envelope.event,
            SseEvent::ChatStarted { ref kind, .. } if kind == "background_auto_wake"
        ));
    }
}
