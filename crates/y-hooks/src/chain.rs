//! Priority-sorted middleware chain.

use std::sync::Arc;

use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareResult};

use crate::error::HookError;

/// A named, priority-sorted middleware entry.
struct MiddlewareEntry {
    middleware: Arc<dyn Middleware>,
    /// Insertion order for stable sorting at equal priorities.
    insertion_order: usize,
}

/// A chain of middleware sorted by priority (ascending).
///
/// Middleware with lower priority numbers execute first.
/// At equal priorities, insertion order is preserved.
pub struct MiddlewareChain {
    chain_type: ChainType,
    entries: Vec<MiddlewareEntry>,
    next_insertion_order: usize,
}

impl MiddlewareChain {
    /// Create a new empty middleware chain.
    pub fn new(chain_type: ChainType) -> Self {
        Self {
            chain_type,
            entries: Vec::new(),
            next_insertion_order: 0,
        }
    }

    /// Register a middleware into the chain.
    ///
    /// The middleware will be positioned based on its priority.
    pub fn register(&mut self, middleware: Arc<dyn Middleware>) -> Result<(), HookError> {
        let middleware_chain_type = middleware.chain_type();

        if middleware_chain_type != self.chain_type {
            return Err(HookError::RegistrationError {
                message: format!(
                    "middleware '{}' belongs to {middleware_chain_type:?}, cannot register in {:?} chain",
                    middleware.name(),
                    self.chain_type
                ),
            });
        }

        if self
            .entries
            .iter()
            .any(|e| e.middleware.name() == middleware.name())
        {
            return Err(HookError::MiddlewareAlreadyRegistered {
                name: middleware.name().to_string(),
            });
        }

        let insertion_order = self.next_insertion_order;
        self.next_insertion_order += 1;

        self.entries.push(MiddlewareEntry {
            middleware,
            insertion_order,
        });

        // Re-sort by priority (ascending), then by insertion order for stability.
        self.entries
            .sort_by_key(|e| (e.middleware.priority(), e.insertion_order));

        Ok(())
    }

    /// Unregister a middleware by name.
    pub fn unregister(&mut self, name: &str) -> Result<(), HookError> {
        let before = self.entries.len();
        self.entries.retain(|e| e.middleware.name() != name);
        if self.entries.len() == before {
            return Err(HookError::MiddlewareNotFound {
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// Execute all middleware in the chain in priority order.
    ///
    /// Returns the final context after all middleware have executed.
    /// Stops early on `ShortCircuit` or abort.
    pub async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<(), y_core::hook::MiddlewareError> {
        for entry in &self.entries {
            if ctx.aborted {
                break;
            }

            let result = entry.middleware.execute(ctx).await?;

            match result {
                MiddlewareResult::Continue => {}
                MiddlewareResult::ShortCircuit => break,
            }
        }

        Ok(())
    }

    /// Get the chain type.
    pub fn chain_type(&self) -> ChainType {
        self.chain_type
    }

    /// Get the number of middleware in the chain.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the names of all middleware in execution order.
    pub fn middleware_names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.middleware.name()).collect()
    }

    /// Get a middleware by name (for diagnostics).
    pub fn get_middleware(&self, name: &str) -> Option<&Arc<dyn Middleware>> {
        self.entries
            .iter()
            .find(|e| e.middleware.name() == name)
            .map(|e| &e.middleware)
    }
}

impl std::fmt::Debug for MiddlewareChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MiddlewareChain")
            .field("chain_type", &self.chain_type)
            .field("count", &self.entries.len())
            .field("middleware", &self.middleware_names())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Test middleware that records its name in the payload.
    struct TestMiddleware {
        name: String,
        priority: u32,
        behavior: TestBehavior,
    }

    enum TestBehavior {
        Continue,
        ShortCircuit,
        Abort(String),
    }

    impl TestMiddleware {
        fn new(name: &str, priority: u32) -> Self {
            Self {
                name: name.to_string(),
                priority,
                behavior: TestBehavior::Continue,
            }
        }

        fn with_behavior(mut self, behavior: TestBehavior) -> Self {
            self.behavior = behavior;
            self
        }
    }

    #[async_trait]
    impl Middleware for TestMiddleware {
        async fn execute(
            &self,
            ctx: &mut MiddlewareContext,
        ) -> Result<MiddlewareResult, y_core::hook::MiddlewareError> {
            // Record execution in metadata.
            if let Some(arr) = ctx.metadata.as_array_mut() {
                arr.push(serde_json::Value::String(self.name.clone()));
            }

            match &self.behavior {
                TestBehavior::Continue => Ok(MiddlewareResult::Continue),
                TestBehavior::ShortCircuit => Ok(MiddlewareResult::ShortCircuit),
                TestBehavior::Abort(reason) => {
                    ctx.abort(reason.clone());
                    Ok(MiddlewareResult::Continue)
                }
            }
        }

        fn chain_type(&self) -> ChainType {
            ChainType::Context
        }

        fn priority(&self) -> u32 {
            self.priority
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn new_ctx() -> MiddlewareContext {
        MiddlewareContext {
            chain_type: ChainType::Context,
            payload: serde_json::json!({}),
            metadata: serde_json::json!([]),
            aborted: false,
            abort_reason: None,
        }
    }

    #[tokio::test]
    async fn test_chain_executes_in_priority_order() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("C", 300)))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("A", 100)))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("B", 200)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["A", "B", "C"]);
    }

    #[tokio::test]
    async fn test_chain_passes_context_between_middleware() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("first", 100)))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("second", 200)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        // Both middleware recorded their names.
        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order.len(), 2);
    }

    #[tokio::test]
    async fn test_chain_short_circuit_stops_execution() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("A", 100)))
            .unwrap();
        chain
            .register(Arc::new(
                TestMiddleware::new("B", 200).with_behavior(TestBehavior::ShortCircuit),
            ))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("C", 300)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["A", "B"]);
        // C should NOT have executed.
    }

    #[tokio::test]
    async fn test_chain_abort_stops_execution() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("A", 100)))
            .unwrap();
        chain
            .register(Arc::new(
                TestMiddleware::new("B", 200)
                    .with_behavior(TestBehavior::Abort("guardrail triggered".into())),
            ))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("C", 300)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        assert!(ctx.aborted);
        assert_eq!(ctx.abort_reason, Some("guardrail triggered".to_string()));

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn test_chain_empty_is_noop() {
        let chain = MiddlewareChain::new(ChainType::Context);
        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert!(order.is_empty());
        assert!(!ctx.aborted);
    }

    #[tokio::test]
    async fn test_chain_single_middleware() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("only", 100)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["only"]);
    }

    #[tokio::test]
    async fn test_chain_same_priority_stable_order() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("first", 100)))
            .unwrap();
        chain
            .register(Arc::new(TestMiddleware::new("second", 100)))
            .unwrap();

        let mut ctx = new_ctx();
        chain.execute(&mut ctx).await.unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["first", "second"]);
    }

    #[test]
    fn test_chain_register_middleware() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        assert!(chain.is_empty());

        chain
            .register(Arc::new(TestMiddleware::new("test", 100)))
            .unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.middleware_names(), vec!["test"]);
    }

    #[test]
    fn test_chain_unregister_middleware() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("test", 100)))
            .unwrap();
        assert_eq!(chain.len(), 1);

        chain.unregister("test").unwrap();
        assert!(chain.is_empty());
    }

    #[test]
    fn test_chain_register_duplicate_fails() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        chain
            .register(Arc::new(TestMiddleware::new("dup", 100)))
            .unwrap();

        let result = chain.register(Arc::new(TestMiddleware::new("dup", 200)));
        assert!(result.is_err());
    }

    #[test]
    fn test_chain_rejects_wrong_chain_type() {
        struct ToolMiddleware {
            inner: TestMiddleware,
        }

        #[async_trait]
        impl Middleware for ToolMiddleware {
            async fn execute(
                &self,
                ctx: &mut MiddlewareContext,
            ) -> Result<MiddlewareResult, y_core::hook::MiddlewareError> {
                self.inner.execute(ctx).await
            }

            fn chain_type(&self) -> ChainType {
                ChainType::Tool
            }

            fn priority(&self) -> u32 {
                self.inner.priority()
            }

            fn name(&self) -> &str {
                self.inner.name()
            }
        }

        let mut chain = MiddlewareChain::new(ChainType::Context);
        let result = chain.register(Arc::new(ToolMiddleware {
            inner: TestMiddleware::new("wrong-chain", 100),
        }));

        assert!(result.is_err());
        assert!(chain.is_empty());
    }

    #[test]
    fn test_chain_unregister_nonexistent_fails() {
        let mut chain = MiddlewareChain::new(ChainType::Context);
        let result = chain.unregister("nonexistent");
        assert!(result.is_err());
    }
}
