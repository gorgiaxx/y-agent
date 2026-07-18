//! Service-owned publication and replay of durable session events.

use y_core::session_event::{
    NewSessionEvent, PersistedSessionEvent, SessionEventKind, SessionEventRetention,
};
use y_core::types::SessionId;
use y_storage::{SqliteSessionEventStore, StorageError};

use crate::chat_types::TurnEvent;
use crate::container::SessionState;

#[derive(Clone)]
pub struct SessionEventService {
    store: SqliteSessionEventStore,
}

impl SessionEventService {
    pub fn new(store: SqliteSessionEventStore) -> Self {
        Self { store }
    }

    pub async fn publish(
        &self,
        session_id: &SessionId,
        kind: SessionEventKind,
        payload: serde_json::Value,
        retention: SessionEventRetention,
        correlation_id: Option<&str>,
    ) -> Result<PersistedSessionEvent, StorageError> {
        self.store
            .append(&NewSessionEvent {
                session_id: session_id.clone(),
                kind,
                payload,
                retention,
                correlation_id: correlation_id.map(str::to_string),
            })
            .await
    }

    pub async fn publish_turn_event(
        &self,
        session_id: &SessionId,
        run_id: &str,
        event: &TurnEvent,
        child_session_id: Option<&SessionId>,
    ) -> Result<Option<PersistedSessionEvent>, StorageError> {
        if !is_durable_turn_event(event) {
            return Ok(None);
        }
        let payload = serde_json::json!({
            "run_id": run_id,
            "event": event,
            "session_id": child_session_id.map(SessionId::as_str),
        });
        self.publish(
            session_id,
            SessionEventKind::ChatProgress,
            payload,
            SessionEventRetention::Durable,
            None,
        )
        .await
        .map(Some)
    }

    pub async fn replay_after(
        &self,
        event_id: u64,
        session_id: Option<&SessionId>,
        limit: usize,
    ) -> Result<Vec<PersistedSessionEvent>, StorageError> {
        self.store
            .list_after_event_id(event_id, session_id, limit)
            .await
    }

    pub async fn latest_event_id(&self) -> Result<u64, StorageError> {
        self.store.latest_event_id().await
    }

    pub async fn prune_short_lived_for_correlation(
        &self,
        session_id: &SessionId,
        correlation_id: &str,
        keep_latest: usize,
    ) -> Result<u64, StorageError> {
        self.store
            .prune_short_lived_for_correlation(session_id, correlation_id, keep_latest)
            .await
    }

    pub async fn pending_events(
        &self,
        state: &SessionState,
        session_id: &SessionId,
    ) -> Result<Vec<PersistedSessionEvent>, StorageError> {
        let mut correlations = Vec::new();
        {
            let pending = state.pending_interactions.lock().await;
            correlations.extend(
                pending
                    .iter()
                    .filter(|(_, request)| request.session_id() == session_id)
                    .map(|(id, _)| id.clone()),
            );
        }
        {
            let pending = state.pending_permissions.lock().await;
            correlations.extend(
                pending
                    .iter()
                    .filter(|(_, request)| request.session_id() == session_id)
                    .map(|(id, _)| id.clone()),
            );
        }
        {
            let pending = state.pending_plan_reviews.lock().await;
            correlations.extend(
                pending
                    .iter()
                    .filter(|(_, request)| request.session_id() == session_id)
                    .map(|(id, _)| id.clone()),
            );
        }
        correlations.sort();
        correlations.dedup();
        self.store
            .latest_for_correlations(session_id, &correlations)
            .await
    }
}

fn is_durable_turn_event(event: &TurnEvent) -> bool {
    matches!(
        event,
        TurnEvent::ToolStart { .. }
            | TurnEvent::ToolResult { .. }
            | TurnEvent::LoopLimitHit { .. }
            | TurnEvent::LlmError { .. }
            | TurnEvent::UserInteractionRequest { .. }
            | TurnEvent::PermissionRequest { .. }
            | TurnEvent::PlanReviewRequest { .. }
            | TurnEvent::SteerInjected { .. }
            | TurnEvent::FollowUpInjected { .. }
    )
}
