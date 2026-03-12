//! Tracing-to-Toast bridge layer.
//!
//! `ToastBridgeLayer` is a `tracing_subscriber::Layer` that captures
//! WARN and ERROR level events and forwards them as `Toast` entries
//! through an `mpsc::UnboundedSender`. The TUI event loop drains the
//! corresponding receiver on each tick, feeding toasts into `AppState`.
//!
//! This module is feature-gated behind `tui`.

use tokio::sync::mpsc;
use tracing::Level;
use tracing_subscriber::Layer;

use crate::tui::state::{Toast, ToastLevel};

/// A `tracing_subscriber::Layer` that bridges WARN/ERROR events to the
/// TUI toast system via an mpsc channel.
pub struct ToastBridgeLayer {
    tx: mpsc::UnboundedSender<Toast>,
    /// Monotonic counter for toast IDs within the bridge.
    /// (Separate from `AppState`'s counter; `AppState` re-assigns on drain.)
    counter: std::sync::atomic::AtomicU64,
}

impl ToastBridgeLayer {
    /// Create a new bridge layer with the given sender.
    pub fn new(tx: mpsc::UnboundedSender<Toast>) -> Self {
        Self {
            tx,
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl<S> Layer<S> for ToastBridgeLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let level = *meta.level();

        // Only forward WARN and ERROR.
        let toast_level = match level {
            Level::ERROR => ToastLevel::Error,
            Level::WARN => ToastLevel::Warning,
            _ => return,
        };

        // Extract the message from the event.
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let message = if visitor.message.is_empty() {
            meta.name().to_string()
        } else {
            visitor.message
        };

        let id = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Best-effort send — if the receiver is dropped, we silently drop the toast.
        let _ = self.tx.send(Toast {
            message,
            level: toast_level,
            ticks_remaining: toast_level.default_ticks(),
            id,
        });
    }
}

/// Visitor that extracts the `message` field from a tracing event.
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    // T-BRIDGE-01: ToastBridgeLayer can be constructed.
    #[test]
    fn test_bridge_layer_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let layer = ToastBridgeLayer::new(tx);
        assert_eq!(
            layer.counter.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    // T-BRIDGE-02: WARN events are forwarded as Warning toasts.
    #[test]
    fn test_bridge_forwards_warn() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bridge = ToastBridgeLayer::new(tx);

        let subscriber = tracing_subscriber::registry().with(bridge);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("something went wrong");
        });

        let toast = rx.try_recv().expect("should have received a toast");
        assert_eq!(toast.level, ToastLevel::Warning);
        assert!(toast.message.contains("something went wrong"));
    }

    // T-BRIDGE-03: ERROR events are forwarded as Error toasts.
    #[test]
    fn test_bridge_forwards_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bridge = ToastBridgeLayer::new(tx);

        let subscriber = tracing_subscriber::registry().with(bridge);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!("critical failure");
        });

        let toast = rx.try_recv().expect("should have received a toast");
        assert_eq!(toast.level, ToastLevel::Error);
        assert!(toast.message.contains("critical failure"));
    }

    // T-BRIDGE-04: INFO/DEBUG/TRACE events are NOT forwarded.
    #[test]
    fn test_bridge_ignores_info_debug_trace() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bridge = ToastBridgeLayer::new(tx);

        let subscriber = tracing_subscriber::registry().with(bridge);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("info message");
            tracing::debug!("debug message");
            tracing::trace!("trace message");
        });

        assert!(
            rx.try_recv().is_err(),
            "should not forward info/debug/trace"
        );
    }

    // T-BRIDGE-05: Toast IDs are monotonically increasing.
    #[test]
    fn test_bridge_toast_ids_monotonic() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let bridge = ToastBridgeLayer::new(tx);

        let subscriber = tracing_subscriber::registry().with(bridge);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("first");
            tracing::error!("second");
        });

        let t1 = rx.try_recv().unwrap();
        let t2 = rx.try_recv().unwrap();
        assert!(t2.id > t1.id);
    }
}
