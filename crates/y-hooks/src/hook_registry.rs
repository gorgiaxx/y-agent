//! Hook handler registration and dispatch.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::instrument;

use y_core::hook::{HookData, HookHandler, HookPoint};

use crate::error::HookError;

/// Registry for lifecycle hook handlers.
///
/// Handlers are read-only observers dispatched by hook point.
/// Handler panics are caught and logged; they do not propagate.
pub struct HookRegistry {
    handlers: RwLock<HashMap<HookPoint, Vec<Arc<dyn HookHandler>>>>,
}

impl HookRegistry {
    /// Create a new empty hook registry.
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a hook handler.
    ///
    /// The handler will be invoked for all hook points returned by
    /// `handler.hook_points()`.
    pub async fn register(&self, handler: Arc<dyn HookHandler>) -> Result<(), HookError> {
        let points = handler.hook_points();
        if points.is_empty() {
            return Err(HookError::RegistrationError {
                message: "handler has no hook points".into(),
            });
        }

        let mut map = self.handlers.write().await;
        for point in points {
            map.entry(point).or_default().push(Arc::clone(&handler));
        }

        Ok(())
    }

    /// Dispatch a hook event to all registered handlers for that hook point.
    ///
    /// Handler panics are caught and logged. Dispatch continues even if
    /// individual handlers fail.
    #[instrument(skip(self, data), fields(hook_point = %data.hook_point))]
    pub async fn dispatch(&self, data: &HookData) {
        let map = self.handlers.read().await;
        let handlers = match map.get(&data.hook_point) {
            Some(h) => h.clone(),
            None => return, // No handlers registered — no-op.
        };
        drop(map); // Release read lock before invoking handlers.

        let hook_point = data.hook_point;
        for handler in handlers {
            let data_clone = data.clone();
            let handle = tokio::spawn(async move {
                handler.handle(&data_clone).await;
            });

            if let Err(e) = handle.await {
                tracing::error!(
                    hook_point = %hook_point,
                    error = %e,
                    "hook handler panicked"
                );
            }
        }
    }

    /// Get the number of handlers registered for a specific hook point.
    pub async fn handler_count(&self, point: HookPoint) -> usize {
        let map = self.handlers.read().await;
        map.get(&point).map_or(0, Vec::len)
    }

    /// Get the total number of handler registrations across all hook points.
    pub async fn total_registrations(&self) -> usize {
        let map = self.handlers.read().await;
        map.values().map(Vec::len).sum()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookRegistry")
            .field("handlers", &"<RwLock<HashMap>>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingHandler {
        points: Vec<HookPoint>,
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl HookHandler for CountingHandler {
        async fn handle(&self, _data: &HookData) {
            self.call_count.fetch_add(1, Ordering::SeqCst);
        }

        fn hook_points(&self) -> Vec<HookPoint> {
            self.points.clone()
        }
    }

    struct PanickingHandler;

    #[async_trait]
    impl HookHandler for PanickingHandler {
        async fn handle(&self, _data: &HookData) {
            panic!("handler panic!");
        }

        fn hook_points(&self) -> Vec<HookPoint> {
            vec![HookPoint::PreToolExecute]
        }
    }

    fn make_hook_data(point: HookPoint) -> HookData {
        HookData {
            hook_point: point,
            payload: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn test_hook_register_handler() {
        let registry = HookRegistry::new();
        let count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler {
            points: vec![HookPoint::PreToolExecute],
            call_count: count,
        });

        registry.register(handler).await.unwrap();
        assert_eq!(registry.handler_count(HookPoint::PreToolExecute).await, 1);
    }

    #[tokio::test]
    async fn test_hook_dispatch_to_matching_handlers() {
        let registry = HookRegistry::new();
        let count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler {
            points: vec![HookPoint::PreToolExecute],
            call_count: Arc::clone(&count),
        });

        registry.register(handler).await.unwrap();
        registry
            .dispatch(&make_hook_data(HookPoint::PreToolExecute))
            .await;

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hook_dispatch_no_match() {
        let registry = HookRegistry::new();
        let count = Arc::new(AtomicU32::new(0));
        let handler = Arc::new(CountingHandler {
            points: vec![HookPoint::PreToolExecute],
            call_count: Arc::clone(&count),
        });

        registry.register(handler).await.unwrap();

        // Dispatch a different hook point.
        registry
            .dispatch(&make_hook_data(HookPoint::PostToolExecute))
            .await;

        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_hook_handler_panic_does_not_propagate() {
        let registry = HookRegistry::new();
        let handler = Arc::new(PanickingHandler);

        registry.register(handler).await.unwrap();

        // Should not panic — panic is caught inside dispatch.
        registry
            .dispatch(&make_hook_data(HookPoint::PreToolExecute))
            .await;
    }

    #[tokio::test]
    async fn test_hook_multiple_handlers_same_point() {
        let registry = HookRegistry::new();
        let count1 = Arc::new(AtomicU32::new(0));
        let count2 = Arc::new(AtomicU32::new(0));
        let count3 = Arc::new(AtomicU32::new(0));

        registry
            .register(Arc::new(CountingHandler {
                points: vec![HookPoint::PreToolExecute],
                call_count: Arc::clone(&count1),
            }))
            .await
            .unwrap();

        registry
            .register(Arc::new(CountingHandler {
                points: vec![HookPoint::PreToolExecute],
                call_count: Arc::clone(&count2),
            }))
            .await
            .unwrap();

        registry
            .register(Arc::new(CountingHandler {
                points: vec![HookPoint::PreToolExecute],
                call_count: Arc::clone(&count3),
            }))
            .await
            .unwrap();

        registry
            .dispatch(&make_hook_data(HookPoint::PreToolExecute))
            .await;

        assert_eq!(count1.load(Ordering::SeqCst), 1);
        assert_eq!(count2.load(Ordering::SeqCst), 1);
        assert_eq!(count3.load(Ordering::SeqCst), 1);
    }
}
