//! Integration test: hooks dispatching events to the event bus.

use std::sync::Arc;

use async_trait::async_trait;
use y_core::hook::{Event, EventFilter, HookData, HookHandler, HookPoint};
use y_hooks::event_bus::EventBus;
use y_hooks::hook_registry::HookRegistry;

/// A hook handler that publishes an event to the event bus when invoked.
struct EventPublishingHandler {
    bus: Arc<EventBus>,
}

#[async_trait]
impl HookHandler for EventPublishingHandler {
    async fn handle(&self, data: &HookData) {
        let event = Event::Custom {
            name: format!("hook_{}", data.hook_point),
            payload: data.payload.clone(),
        };
        let _ = self.bus.publish(event).await;
    }

    fn hook_points(&self) -> Vec<HookPoint> {
        vec![HookPoint::PostToolExecute]
    }
}

// T-HOOK-INT-05: Hook handler fires event, subscriber receives it.
#[tokio::test]
async fn test_hook_and_event_combined() {
    let bus = Arc::new(EventBus::new(100));
    let registry = HookRegistry::new();

    // Subscribe to events.
    let mut sub = bus.subscribe(EventFilter::all()).await;

    // Register a handler that publishes to the bus.
    let handler: Arc<dyn HookHandler> = Arc::new(EventPublishingHandler {
        bus: Arc::clone(&bus),
    });
    registry.register(handler).await.unwrap();

    // Dispatch a hook event.
    let hook_data = HookData {
        hook_point: HookPoint::PostToolExecute,
        payload: serde_json::json!({"tool": "search", "result": "success"}),
    };
    registry.dispatch(&hook_data).await;

    // Give the spawned handler task time to complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Subscriber should have received the event.
    let event = sub.recv().await.unwrap();
    match event.as_ref() {
        Event::Custom { name, payload } => {
            assert!(name.contains("PostToolExecute"));
            assert_eq!(payload["tool"], "search");
        }
        _ => panic!("expected Custom event from hook handler"),
    }
}
