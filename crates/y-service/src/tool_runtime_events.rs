//! Service-owned persistence and broadcast for tool runtime notifications.

use std::sync::Arc;

use tokio::sync::{broadcast, mpsc};
use y_core::runtime::{ToolRuntimeEvent, ToolRuntimeEventKind, ToolRuntimeEventSink};
use y_core::session_event::{SessionEventKind, SessionEventRetention};
use y_storage::StorageError;

use crate::SessionEventService;

const LIVE_CHANNEL_CAPACITY: usize = 512;
const MAX_SHORT_LIVED_EVENTS_PER_TASK: usize = 256;

#[derive(Debug, Clone)]
pub struct PublishedToolRuntimeEvent {
    pub event_id: u64,
    pub event: ToolRuntimeEvent,
}

#[derive(Clone)]
pub struct ToolRuntimeEventService {
    session_events: SessionEventService,
    broadcast_tx: broadcast::Sender<PublishedToolRuntimeEvent>,
}

struct QueuedToolRuntimeEventSink {
    tx: mpsc::UnboundedSender<ToolRuntimeEvent>,
}

impl ToolRuntimeEventSink for QueuedToolRuntimeEventSink {
    fn publish(&self, event: ToolRuntimeEvent) {
        if self.tx.send(event).is_err() {
            tracing::warn!("tool runtime event consumer is unavailable");
        }
    }
}

impl ToolRuntimeEventService {
    pub fn new(session_events: SessionEventService) -> (Self, Arc<dyn ToolRuntimeEventSink>) {
        let (broadcast_tx, _) = broadcast::channel(LIVE_CHANNEL_CAPACITY);
        let service = Self {
            session_events,
            broadcast_tx,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let consumer = service.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(error) = consumer.publish(event).await {
                    tracing::error!(%error, "failed to persist tool runtime event");
                }
            }
        });
        (service, Arc::new(QueuedToolRuntimeEventSink { tx }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PublishedToolRuntimeEvent> {
        self.broadcast_tx.subscribe()
    }

    pub async fn publish(
        &self,
        event: ToolRuntimeEvent,
    ) -> Result<PublishedToolRuntimeEvent, StorageError> {
        let retention = retention_for(&event.kind);
        let correlation_id = format!("runtime:{}", event.task_id);
        let persisted = self
            .session_events
            .publish(
                &event.session_id,
                SessionEventKind::ToolRuntime,
                serde_json::to_value(&event)?,
                retention,
                Some(&correlation_id),
            )
            .await?;

        if retention == SessionEventRetention::ShortLived {
            if let Err(error) = self
                .session_events
                .prune_short_lived_for_correlation(
                    &event.session_id,
                    &correlation_id,
                    MAX_SHORT_LIVED_EVENTS_PER_TASK,
                )
                .await
            {
                tracing::warn!(
                    %error,
                    session_id = %event.session_id,
                    task_id = %event.task_id,
                    "failed to prune short-lived tool runtime events"
                );
            }
        }

        let published = PublishedToolRuntimeEvent {
            event_id: persisted.event_id,
            event,
        };
        let _ = self.broadcast_tx.send(published.clone());
        Ok(published)
    }
}

fn retention_for(kind: &ToolRuntimeEventKind) -> SessionEventRetention {
    match kind {
        ToolRuntimeEventKind::OutputChunk { .. } => SessionEventRetention::ShortLived,
        ToolRuntimeEventKind::ProcessStarted { .. }
        | ToolRuntimeEventKind::ProcessCompleted { .. }
        | ToolRuntimeEventKind::ProcessFailed { .. }
        | ToolRuntimeEventKind::ProcessKilled { .. } => SessionEventRetention::Durable,
    }
}
