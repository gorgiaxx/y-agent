//! Integration tests for the event bus under load.

use y_core::hook::{Event, EventCategory, EventFilter, LlmEvent, ToolEvent};
use y_hooks::event_bus::EventBus;

// T-HOOK-INT-04: Event bus under load.
#[tokio::test]
async fn test_event_bus_under_load() {
    let bus = EventBus::new(2048);

    // Create 10 subscribers with various filters.
    let mut subs = Vec::new();
    for i in 0..10 {
        let filter = if i % 2 == 0 {
            EventFilter::all()
        } else {
            EventFilter::categories(vec![EventCategory::Tool])
        };
        subs.push(bus.subscribe(filter).await);
    }

    // Publish 1000 mixed events.
    for i in 0..1000 {
        let event = if i % 3 == 0 {
            Event::Tool(ToolEvent::Executed {
                tool_name: format!("tool-{i}"),
                success: true,
                duration_ms: 42,
            })
        } else {
            Event::Llm(LlmEvent::CallCompleted {
                provider: "openai".into(),
                model: "gpt-4".into(),
                input_tokens: 100,
                output_tokens: 50,
                duration_ms: 500,
            })
        };
        bus.publish(event).await.unwrap();
    }

    // Verify metrics.
    let metrics = bus.metrics().snapshot();
    assert_eq!(metrics.published, 1000);
    // With 10 subscribers, but some filtered, delivered < 10*1000.
    assert!(metrics.delivered > 0);
    assert_eq!(metrics.dropped, 0); // capacity is 2048 > 1000

    // Verify all-event subscribers got all 1000.
    for i in (0..10).step_by(2) {
        let mut count = 0;
        while let Ok(event) = subs[i].receiver.try_recv() {
            let _ = event;
            count += 1;
        }
        assert_eq!(
            count, 1000,
            "all-event subscriber {i} should get 1000 events"
        );
    }

    // Verify tool-only subscribers got only tool events (~334).
    let tool_event_count = 1000 / 3 + i32::from(1000 % 3 > 0);
    for i in (1..10).step_by(2) {
        let mut count = 0;
        while let Ok(event) = subs[i].receiver.try_recv() {
            assert!(matches!(
                event.as_ref(),
                Event::Tool(ToolEvent::Executed { .. })
            ));
            count += 1;
        }
        assert_eq!(
            count, tool_event_count,
            "tool-only subscriber {i} should get {tool_event_count} events"
        );
    }
}
