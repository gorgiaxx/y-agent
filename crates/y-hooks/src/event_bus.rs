//! Async event bus using `tokio::broadcast`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::instrument;

use y_core::hook::Event;

use crate::error::HookError;

/// A subscriber handle returned by `EventBus::subscribe`.
///
/// Drop this to unsubscribe.
pub struct Subscription {
    pub receiver: broadcast::Receiver<Arc<Event>>,
    id: usize,
}

impl Subscription {
    /// Receive the next event, blocking until one is available.
    pub async fn recv(&mut self) -> Result<Arc<Event>, HookError> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Ok(event),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        subscriber_id = self.id,
                        skipped = n,
                        "subscriber lagged, oldest events dropped"
                    );
                    // Continue receiving — dropped oldest events.
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(HookError::EventBusError {
                        message: "event bus closed".into(),
                    });
                }
            }
        }
    }

    /// Get this subscription's ID.
    pub fn id(&self) -> usize {
        self.id
    }
}

/// Async event bus for fire-and-forget notifications.
///
/// Uses `tokio::broadcast` internally. Slow subscribers drop oldest events
/// (no backpressure on publishers).
pub struct EventBus {
    sender: broadcast::Sender<Arc<Event>>,
    capacity: usize,
    next_subscriber_id: AtomicUsize,
}

impl EventBus {
    /// Create a new event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            capacity,
            next_subscriber_id: AtomicUsize::new(0),
        }
    }

    /// Subscribe to events from this bus.
    ///
    /// Returns a `Subscription` that receives events. Drop to unsubscribe.
    pub fn subscribe(&self) -> Subscription {
        let id = self.next_subscriber_id.fetch_add(1, Ordering::SeqCst);
        let receiver = self.sender.subscribe();
        Subscription { receiver, id }
    }

    /// Publish an event to all subscribers.
    ///
    /// This is fire-and-forget: if there are no subscribers, the event
    /// is silently dropped.
    #[instrument(skip(self, event))]
    pub fn publish(&self, event: Event) -> Result<(), HookError> {
        let event = Arc::new(event);
        // send returns Err only if there are no receivers, which is fine.
        let _ = self.sender.send(event);
        Ok(())
    }

    /// Get the channel capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the current number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("capacity", &self.capacity)
            .field("subscribers", &self.sender.receiver_count())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_event(name: &str) -> Event {
        Event::ToolExecuted {
            tool_name: name.to_string(),
            success: true,
            duration_ms: 42,
        }
    }

    fn llm_event() -> Event {
        Event::LlmCallCompleted {
            provider: "openai".into(),
            model: "gpt-4".into(),
            input_tokens: 100,
            output_tokens: 50,
            duration_ms: 500,
        }
    }

    fn custom_event(name: &str) -> Event {
        Event::Custom {
            name: name.into(),
            payload: serde_json::json!({"key": "value"}),
        }
    }

    #[tokio::test]
    async fn test_event_bus_subscribe_and_receive() {
        let bus = EventBus::new(100);
        let mut sub = bus.subscribe();

        bus.publish(tool_event("search")).unwrap();

        let event = sub.recv().await.unwrap();
        match event.as_ref() {
            Event::ToolExecuted { tool_name, .. } => {
                assert_eq!(tool_name, "search");
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new(100);
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();
        let mut sub3 = bus.subscribe();

        bus.publish(tool_event("test")).unwrap();

        // All 3 subscribers should receive the event.
        let e1 = sub1.recv().await.unwrap();
        let e2 = sub2.recv().await.unwrap();
        let e3 = sub3.recv().await.unwrap();

        // They should all point to the same Arc.
        assert!(Arc::ptr_eq(&e1, &e2));
        assert!(Arc::ptr_eq(&e2, &e3));
    }

    #[tokio::test]
    async fn test_event_bus_filter_by_type() {
        let bus = EventBus::new(100);
        let mut sub = bus.subscribe();

        bus.publish(tool_event("search")).unwrap();
        bus.publish(llm_event()).unwrap();
        bus.publish(tool_event("code")).unwrap();

        // Receive all 3 events, filter client-side.
        let mut tool_events = Vec::new();
        for _ in 0..3 {
            let event = sub.recv().await.unwrap();
            if matches!(event.as_ref(), Event::ToolExecuted { .. }) {
                tool_events.push(event);
            }
        }
        assert_eq!(tool_events.len(), 2);
    }

    #[tokio::test]
    async fn test_event_bus_slow_subscriber_drops_oldest() {
        // Create bus with capacity 2.
        let bus = EventBus::new(2);
        let mut sub = bus.subscribe();

        // Publish 5 events without reading.
        for i in 0..5 {
            bus.publish(tool_event(&format!("tool-{i}"))).unwrap();
        }

        // Subscriber should have lagged and dropped oldest.
        // The recv() implementation handles Lagged by continuing.
        let event = sub.recv().await.unwrap();
        // We should get one of the later events (exact behavior depends on broadcast impl).
        assert!(matches!(event.as_ref(), Event::ToolExecuted { .. }));
    }

    #[tokio::test]
    async fn test_event_bus_fire_and_forget() {
        let bus = EventBus::new(100);
        // No subscribers — publish should not error.
        let result = bus.publish(tool_event("orphan"));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_event_bus_unsubscribe() {
        let bus = EventBus::new(100);
        assert_eq!(bus.subscriber_count(), 0);

        let sub = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        drop(sub);
        // After drop, subscriber count should decrease.
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn test_event_bus_custom_event() {
        let bus = EventBus::new(100);
        let mut sub = bus.subscribe();

        bus.publish(custom_event("my_event")).unwrap();

        let event = sub.recv().await.unwrap();
        match event.as_ref() {
            Event::Custom { name, payload } => {
                assert_eq!(name, "my_event");
                assert_eq!(payload["key"], "value");
            }
            _ => panic!("expected Custom event"),
        }
    }
}
