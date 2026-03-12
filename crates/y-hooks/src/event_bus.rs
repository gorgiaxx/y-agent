//! Async event bus with per-subscriber channels.
//!
//! Design reference: hooks-plugin-design.md §Event Bus
//!
//! Architecture: channel-per-subscriber (not broadcast).
//! Each subscriber gets its own `mpsc` channel. The publisher pre-filters
//! events via `EventFilter` before sending, so only matching events enter
//! a subscriber's channel. If a subscriber's channel is full, events are
//! dropped for that subscriber (with a metric increment) rather than
//! applying backpressure to the publisher.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::instrument;

use y_core::hook::{Event, EventFilter, EventSubscriber};

use crate::error::HookError;

/// Metrics counters for the event bus.
#[derive(Debug, Default)]
pub struct EventBusMetrics {
    /// Total events published.
    pub published: AtomicU64,
    /// Total events delivered (across all subscribers).
    pub delivered: AtomicU64,
    /// Total events dropped (across all subscribers due to full channels).
    pub dropped: AtomicU64,
}

impl EventBusMetrics {
    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> EventBusMetricsSnapshot {
        EventBusMetricsSnapshot {
            published: self.published.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time snapshot of event bus metrics.
#[derive(Debug, Clone)]
pub struct EventBusMetricsSnapshot {
    pub published: u64,
    pub delivered: u64,
    pub dropped: u64,
}

/// Internal subscriber entry.
struct SubscriberEntry {
    id: usize,
    filter: EventFilter,
    sender: mpsc::Sender<Arc<Event>>,
}

/// A subscriber handle returned by `EventBus::subscribe`.
///
/// Drop this to unsubscribe (the sender side will detect the closed channel).
pub struct Subscription {
    pub receiver: mpsc::Receiver<Arc<Event>>,
    id: usize,
}

impl Subscription {
    /// Receive the next event, blocking until one is available.
    pub async fn recv(&mut self) -> Option<Arc<Event>> {
        self.receiver.recv().await
    }

    /// Get this subscription's ID.
    pub fn id(&self) -> usize {
        self.id
    }
}

/// Async event bus for fire-and-forget notifications.
///
/// Uses per-subscriber `mpsc` channels internally. Slow subscribers drop
/// events (no backpressure on publishers). Events are pre-filtered per
/// subscriber before entering their channel.
pub struct EventBus {
    subscribers: RwLock<Vec<SubscriberEntry>>,
    capacity: usize,
    next_subscriber_id: AtomicUsize,
    metrics: EventBusMetrics,
}

impl EventBus {
    /// Create a new event bus with the given per-subscriber channel capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            capacity,
            next_subscriber_id: AtomicUsize::new(0),
            metrics: EventBusMetrics::default(),
        }
    }

    /// Subscribe to events from this bus with the given filter.
    ///
    /// Returns a `Subscription` whose `recv()` method yields only events
    /// matching the filter. Drop the `Subscription` to unsubscribe.
    pub async fn subscribe(&self, filter: EventFilter) -> Subscription {
        let id = self.next_subscriber_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = mpsc::channel(self.capacity);

        let entry = SubscriberEntry {
            id,
            filter,
            sender,
        };

        self.subscribers.write().await.push(entry);

        Subscription { receiver, id }
    }

    /// Subscribe with a trait-based event subscriber.
    ///
    /// Spawns a background task that consumes from the per-subscriber channel
    /// and calls `on_event()`. The subscriber's `event_filter()` is used for
    /// pre-filtering.
    pub async fn subscribe_handler(&self, handler: Arc<dyn EventSubscriber>) {
        let filter = handler.event_filter();
        let mut sub = self.subscribe(filter).await;

        tokio::spawn(async move {
            while let Some(event) = sub.recv().await {
                handler.on_event(&event).await;
            }
        });
    }

    /// Publish an event to all matching subscribers.
    ///
    /// This is fire-and-forget: if no subscribers match, the event is
    /// silently dropped. If a subscriber's channel is full, the event
    /// is dropped for that subscriber and the `dropped` metric is incremented.
    #[instrument(skip(self, event), fields(category = ?event.category()))]
    pub async fn publish(&self, event: Event) -> Result<(), HookError> {
        self.metrics.published.fetch_add(1, Ordering::Relaxed);
        let event = Arc::new(event);

        // Clean up closed subscribers while publishing.
        let mut to_remove = Vec::new();

        let subscribers = self.subscribers.read().await;
        for entry in subscribers.iter() {
            if entry.sender.is_closed() {
                to_remove.push(entry.id);
                continue;
            }

            if !entry.filter.matches(&event) {
                continue;
            }

            match entry.sender.try_send(Arc::clone(&event)) {
                Ok(()) => {
                    self.metrics.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(subscriber_id = entry.id, "subscriber channel full, event dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    to_remove.push(entry.id);
                }
            }
        }
        drop(subscribers);

        // Remove closed subscribers.
        if !to_remove.is_empty() {
            let mut subs = self.subscribers.write().await;
            subs.retain(|e| !to_remove.contains(&e.id));
        }

        Ok(())
    }

    /// Get the channel capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the current number of active subscribers.
    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.read().await.len()
    }

    /// Get the metrics counters.
    pub fn metrics(&self) -> &EventBusMetrics {
        &self.metrics
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::hook::EventCategory;

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
        let mut sub = bus.subscribe(EventFilter::all()).await;

        bus.publish(tool_event("search")).await.unwrap();

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
        let mut sub1 = bus.subscribe(EventFilter::all()).await;
        let mut sub2 = bus.subscribe(EventFilter::all()).await;
        let mut sub3 = bus.subscribe(EventFilter::all()).await;

        bus.publish(tool_event("test")).await.unwrap();

        // All 3 subscribers should receive the event.
        let e1 = sub1.recv().await.unwrap();
        let e2 = sub2.recv().await.unwrap();
        let e3 = sub3.recv().await.unwrap();

        // All should be ToolExecuted events.
        assert!(matches!(e1.as_ref(), Event::ToolExecuted { .. }));
        assert!(matches!(e2.as_ref(), Event::ToolExecuted { .. }));
        assert!(matches!(e3.as_ref(), Event::ToolExecuted { .. }));
    }

    #[tokio::test]
    async fn test_event_bus_filter_by_category() {
        let bus = EventBus::new(100);
        let mut tool_sub = bus
            .subscribe(EventFilter::categories(vec![EventCategory::Tool]))
            .await;
        let mut all_sub = bus.subscribe(EventFilter::all()).await;

        bus.publish(tool_event("search")).await.unwrap();
        bus.publish(llm_event()).await.unwrap();
        bus.publish(tool_event("code")).await.unwrap();

        // tool_sub should only get Tool events.
        let e1 = tool_sub.recv().await.unwrap();
        assert!(matches!(e1.as_ref(), Event::ToolExecuted { .. }));
        let e2 = tool_sub.recv().await.unwrap();
        assert!(matches!(e2.as_ref(), Event::ToolExecuted { .. }));

        // all_sub gets all 3.
        let _ = all_sub.recv().await.unwrap();
        let _ = all_sub.recv().await.unwrap();
        let _ = all_sub.recv().await.unwrap();
    }

    #[tokio::test]
    async fn test_event_bus_slow_subscriber_drops_events() {
        // Create bus with capacity 2.
        let bus = EventBus::new(2);
        let mut sub = bus.subscribe(EventFilter::all()).await;

        // Publish 5 events without reading — channel holds only 2.
        for i in 0..5 {
            bus.publish(tool_event(&format!("tool-{i}"))).await.unwrap();
        }

        // Should have dropped some events.
        let metrics = bus.metrics().snapshot();
        assert_eq!(metrics.published, 5);
        assert!(metrics.dropped > 0, "should have dropped events");

        // Can still receive remaining events.
        let event = sub.recv().await.unwrap();
        assert!(matches!(event.as_ref(), Event::ToolExecuted { .. }));
    }

    #[tokio::test]
    async fn test_event_bus_fire_and_forget() {
        let bus = EventBus::new(100);
        // No subscribers — publish should not error.
        let result = bus.publish(tool_event("orphan")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_event_bus_unsubscribe() {
        let bus = EventBus::new(100);
        assert_eq!(bus.subscriber_count().await, 0);

        let sub = bus.subscribe(EventFilter::all()).await;
        assert_eq!(bus.subscriber_count().await, 1);

        drop(sub);
        // Publish triggers cleanup of closed subscribers.
        let _ = bus.publish(tool_event("cleanup")).await;
        assert_eq!(bus.subscriber_count().await, 0);
    }

    #[tokio::test]
    async fn test_event_bus_custom_event() {
        let bus = EventBus::new(100);
        let mut sub = bus.subscribe(EventFilter::all()).await;

        bus.publish(custom_event("my_event")).await.unwrap();

        let event = sub.recv().await.unwrap();
        match event.as_ref() {
            Event::Custom { name, payload } => {
                assert_eq!(name, "my_event");
                assert_eq!(payload["key"], "value");
            }
            _ => panic!("expected Custom event"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_metrics() {
        let bus = EventBus::new(100);
        let _sub = bus.subscribe(EventFilter::all()).await;

        bus.publish(tool_event("a")).await.unwrap();
        bus.publish(tool_event("b")).await.unwrap();

        let metrics = bus.metrics().snapshot();
        assert_eq!(metrics.published, 2);
        assert_eq!(metrics.delivered, 2);
        assert_eq!(metrics.dropped, 0);
    }

    #[tokio::test]
    async fn test_event_bus_subscribe_handler() {
        use async_trait::async_trait;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingSubscriber {
            count: Arc<AtomicU32>,
        }

        #[async_trait]
        impl EventSubscriber for CountingSubscriber {
            async fn on_event(&self, _event: &Event) {
                self.count.fetch_add(1, Ordering::SeqCst);
            }

            fn event_filter(&self) -> EventFilter {
                EventFilter::all()
            }
        }

        let bus = EventBus::new(100);
        let counter = Arc::new(AtomicU32::new(0));
        let handler: Arc<dyn EventSubscriber> = Arc::new(CountingSubscriber {
            count: Arc::clone(&counter),
        });

        bus.subscribe_handler(handler).await;

        bus.publish(tool_event("test")).await.unwrap();

        // Give the background task time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
