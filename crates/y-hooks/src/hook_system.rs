//! Unified `HookSystem` facade.
//!
//! Aggregates all 5 middleware chains, the hook registry, the event bus,
//! the chain runner, and the optional hook handler executor into a single
//! entry point. Other modules interact with the hook system exclusively
//! through this facade.

use std::sync::Arc;

use y_core::hook::{
    ChainType, Event, EventFilter, EventSubscriber, HookData, HookHandler, Middleware,
    MiddlewareContext, MiddlewareError,
};

use crate::chain::MiddlewareChain;
use crate::chain_runner::ChainRunner;
use crate::config::HookConfig;
use crate::error::HookError;
use crate::event_bus::EventBus;
use crate::hook_registry::HookRegistry;

/// Unified hook system that aggregates middleware chains, hook registry,
/// event bus, chain runner, and hook handler executor.
///
/// This is the primary entry point for other modules to interact with
/// the hook/middleware/event system.
pub struct HookSystem {
    /// Context assembly pipeline (priorities 100-700).
    context_chain: MiddlewareChain,
    /// Tool execution pipeline (validation, journaling, guardrails).
    tool_chain: MiddlewareChain,
    /// LLM call pipeline (rate limiting, caching, auditing).
    llm_chain: MiddlewareChain,
    /// Context compaction pipeline.
    compaction_chain: MiddlewareChain,
    /// Memory storage pipeline.
    memory_chain: MiddlewareChain,
    /// Lifecycle hook handler registry.
    hooks: HookRegistry,
    /// Async event bus.
    events: EventBus,
    /// Timeout-guarded middleware runner.
    runner: ChainRunner,
    /// External hook handler executor (command/HTTP/prompt/agent).
    /// None if no handlers are configured or `handlers_enabled` = false.
    #[cfg(feature = "hook_handlers")]
    handler_executor: Option<crate::hook_handler::HookHandlerExecutor>,
}

impl HookSystem {
    /// Create a new hook system from configuration.
    pub fn new(config: &HookConfig) -> Self {
        #[cfg(feature = "hook_handlers")]
        let handler_executor = if config.handlers_enabled && !config.hook_handlers.is_empty() {
            match crate::hook_handler::HookHandlerExecutor::from_config(config) {
                Ok(executor) => Some(executor),
                Err(e) => {
                    tracing::error!(error = %e, "failed to initialize hook handlers");
                    None
                }
            }
        } else {
            None
        };

        Self {
            context_chain: MiddlewareChain::new(ChainType::Context),
            tool_chain: MiddlewareChain::new(ChainType::Tool),
            llm_chain: MiddlewareChain::new(ChainType::Llm),
            compaction_chain: MiddlewareChain::new(ChainType::Compaction),
            memory_chain: MiddlewareChain::new(ChainType::Memory),
            hooks: HookRegistry::new(),
            events: EventBus::new(config.event_channel_capacity),
            runner: ChainRunner::new(config.middleware_timeout()),
            #[cfg(feature = "hook_handlers")]
            handler_executor,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(&HookConfig::default())
    }

    // --- Middleware chain accessors ---

    /// Get a reference to the context middleware chain.
    pub fn context_chain(&self) -> &MiddlewareChain {
        &self.context_chain
    }

    /// Get a mutable reference to the context middleware chain.
    pub fn context_chain_mut(&mut self) -> &mut MiddlewareChain {
        &mut self.context_chain
    }

    /// Get a reference to the tool middleware chain.
    pub fn tool_chain(&self) -> &MiddlewareChain {
        &self.tool_chain
    }

    /// Get a mutable reference to the tool middleware chain.
    pub fn tool_chain_mut(&mut self) -> &mut MiddlewareChain {
        &mut self.tool_chain
    }

    /// Get a reference to the LLM middleware chain.
    pub fn llm_chain(&self) -> &MiddlewareChain {
        &self.llm_chain
    }

    /// Get a mutable reference to the LLM middleware chain.
    pub fn llm_chain_mut(&mut self) -> &mut MiddlewareChain {
        &mut self.llm_chain
    }

    /// Get a reference to the compaction middleware chain.
    pub fn compaction_chain(&self) -> &MiddlewareChain {
        &self.compaction_chain
    }

    /// Get a mutable reference to the compaction middleware chain.
    pub fn compaction_chain_mut(&mut self) -> &mut MiddlewareChain {
        &mut self.compaction_chain
    }

    /// Get a reference to the memory middleware chain.
    pub fn memory_chain(&self) -> &MiddlewareChain {
        &self.memory_chain
    }

    /// Get a mutable reference to the memory middleware chain.
    pub fn memory_chain_mut(&mut self) -> &mut MiddlewareChain {
        &mut self.memory_chain
    }

    /// Get a middleware chain by type.
    pub fn chain(&self, chain_type: ChainType) -> &MiddlewareChain {
        match chain_type {
            ChainType::Context => &self.context_chain,
            ChainType::Tool => &self.tool_chain,
            ChainType::Llm => &self.llm_chain,
            ChainType::Compaction => &self.compaction_chain,
            ChainType::Memory => &self.memory_chain,
        }
    }

    /// Get a mutable middleware chain by type.
    pub fn chain_mut(&mut self, chain_type: ChainType) -> &mut MiddlewareChain {
        match chain_type {
            ChainType::Context => &mut self.context_chain,
            ChainType::Tool => &mut self.tool_chain,
            ChainType::Llm => &mut self.llm_chain,
            ChainType::Compaction => &mut self.compaction_chain,
            ChainType::Memory => &mut self.memory_chain,
        }
    }

    // --- Middleware registration ---

    /// Register a middleware into the appropriate chain based on its `chain_type()`.
    pub fn register_middleware(
        &mut self,
        middleware: Arc<dyn Middleware>,
    ) -> Result<(), HookError> {
        let chain = self.chain_mut(middleware.chain_type());
        chain.register(middleware)
    }

    // --- Chain execution ---

    /// Execute a middleware chain with timeout-guarded per-middleware execution.
    pub async fn execute_chain(
        &self,
        chain_type: ChainType,
        ctx: &mut MiddlewareContext,
    ) -> Result<(), MiddlewareError> {
        let chain = self.chain(chain_type);
        chain.execute(ctx).await
    }

    // --- Hook system ---

    /// Get a reference to the hook registry.
    pub fn hooks(&self) -> &HookRegistry {
        &self.hooks
    }

    /// Register a hook handler.
    pub async fn register_hook(&self, handler: Arc<dyn HookHandler>) -> Result<(), HookError> {
        self.hooks.register(handler).await
    }

    /// Dispatch a hook event.
    pub async fn dispatch_hook(&self, data: &HookData) {
        self.hooks.dispatch(data).await;
    }

    // --- Event bus ---

    /// Get a reference to the event bus.
    pub fn events(&self) -> &EventBus {
        &self.events
    }

    /// Publish an event to the event bus.
    pub async fn publish_event(&self, event: Event) -> Result<(), HookError> {
        self.events.publish(event).await
    }

    /// Subscribe to events with a filter.
    pub async fn subscribe_events(&self, filter: EventFilter) -> crate::event_bus::Subscription {
        self.events.subscribe(filter).await
    }

    /// Subscribe with a trait-based event subscriber.
    pub async fn subscribe_handler(&self, handler: Arc<dyn EventSubscriber>) {
        self.events.subscribe_handler(handler).await;
    }

    // --- Runner ---

    /// Get a reference to the chain runner.
    pub fn runner(&self) -> &ChainRunner {
        &self.runner
    }

    // --- Hook handler executor ---

    /// Execute external hook handlers for a hook point.
    /// Returns the aggregate decision. Noop if no executor or no handlers for this point.
    #[cfg(feature = "hook_handlers")]
    pub async fn execute_hook_handlers(
        &self,
        hook_point: y_core::hook::HookPoint,
        input: &crate::hook_handler::HookInput,
    ) -> crate::hook_handler::HookHandlerResult {
        match &self.handler_executor {
            Some(executor) => executor.execute(hook_point, input).await,
            None => crate::hook_handler::HookHandlerResult::default(),
        }
    }

    /// Get the handler executor (for diagnostics/metrics).
    #[cfg(feature = "hook_handlers")]
    pub fn handler_executor(&self) -> Option<&crate::hook_handler::HookHandlerExecutor> {
        self.handler_executor.as_ref()
    }

    /// Hot-reload the hook system configuration.
    ///
    /// Rebuilds the handler executor from the new config (re-validating hook
    /// handlers, recompiling matchers) and updates the chain runner timeout.
    /// Existing middleware registrations and event subscriptions are preserved.
    pub fn reload_config(&mut self, new_config: &HookConfig) {
        // Update chain runner timeout.
        self.runner = ChainRunner::new(new_config.middleware_timeout());

        // Rebuild event bus with new capacity.
        self.events = EventBus::new(new_config.event_channel_capacity);

        // Rebuild handler executor.
        #[cfg(feature = "hook_handlers")]
        {
            self.handler_executor =
                if new_config.handlers_enabled && !new_config.hook_handlers.is_empty() {
                    match crate::hook_handler::HookHandlerExecutor::from_config(new_config) {
                        Ok(executor) => Some(executor),
                        Err(e) => {
                            tracing::error!(error = %e, "failed to reinitialize hook handlers");
                            None
                        }
                    }
                } else {
                    None
                };
        }

        tracing::info!("Hook system config hot-reloaded");
    }

    /// Inject an LLM runner into the hook handler executor.
    ///
    /// Called during application startup after `ProviderPool` is initialized.
    /// If no executor exists (no handlers configured), this is a no-op.
    #[cfg(all(feature = "hook_handlers", feature = "llm_hooks"))]
    pub fn set_llm_runner(&mut self, runner: std::sync::Arc<dyn y_core::hook::HookLlmRunner>) {
        if let Some(ref mut executor) = self.handler_executor {
            executor.set_llm_runner(runner);
        }
    }

    /// Inject an agent runner into the hook handler executor.
    ///
    /// Called during application startup after the agent loop is initialized.
    /// If no executor exists (no handlers configured), this is a no-op.
    #[cfg(all(feature = "hook_handlers", feature = "llm_hooks"))]
    pub fn set_agent_runner(&mut self, runner: std::sync::Arc<dyn y_core::hook::HookAgentRunner>) {
        if let Some(ref mut executor) = self.handler_executor {
            executor.set_agent_runner(runner);
        }
    }
}

impl std::fmt::Debug for HookSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("HookSystem");
        s.field("context_chain", &self.context_chain)
            .field("tool_chain", &self.tool_chain)
            .field("llm_chain", &self.llm_chain)
            .field("compaction_chain", &self.compaction_chain)
            .field("memory_chain", &self.memory_chain)
            .field("hooks", &self.hooks)
            .field("events", &self.events);
        #[cfg(feature = "hook_handlers")]
        s.field("handler_executor", &self.handler_executor.is_some());
        s.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use y_core::hook::MiddlewareResult;

    struct TestMW {
        name: String,
        priority: u32,
        chain: ChainType,
    }

    #[async_trait]
    impl Middleware for TestMW {
        async fn execute(
            &self,
            ctx: &mut MiddlewareContext,
        ) -> Result<MiddlewareResult, MiddlewareError> {
            if let Some(arr) = ctx.metadata.as_array_mut() {
                arr.push(serde_json::json!(self.name));
            }
            Ok(MiddlewareResult::Continue)
        }

        fn chain_type(&self) -> ChainType {
            self.chain
        }

        fn priority(&self) -> u32 {
            self.priority
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    fn test_hook_system_creation() {
        let system = HookSystem::with_defaults();
        assert!(system.context_chain().is_empty());
        assert!(system.tool_chain().is_empty());
        assert!(system.llm_chain().is_empty());
        assert!(system.compaction_chain().is_empty());
        assert!(system.memory_chain().is_empty());
    }

    #[test]
    fn test_hook_system_register_middleware() {
        let mut system = HookSystem::with_defaults();

        let mw: Arc<dyn Middleware> = Arc::new(TestMW {
            name: "test-ctx".into(),
            priority: 100,
            chain: ChainType::Context,
        });
        system.register_middleware(mw).unwrap();
        assert_eq!(system.context_chain().len(), 1);

        let mw: Arc<dyn Middleware> = Arc::new(TestMW {
            name: "test-tool".into(),
            priority: 100,
            chain: ChainType::Tool,
        });
        system.register_middleware(mw).unwrap();
        assert_eq!(system.tool_chain().len(), 1);
    }

    #[tokio::test]
    async fn test_hook_system_execute_chain() {
        let mut system = HookSystem::with_defaults();
        let mw: Arc<dyn Middleware> = Arc::new(TestMW {
            name: "a".into(),
            priority: 100,
            chain: ChainType::Context,
        });
        system.register_middleware(mw).unwrap();

        let mut ctx = MiddlewareContext {
            chain_type: ChainType::Context,
            payload: serde_json::json!({}),
            metadata: serde_json::json!([]),
            aborted: false,
            abort_reason: None,
        };
        system
            .execute_chain(ChainType::Context, &mut ctx)
            .await
            .unwrap();

        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["a"]);
    }

    #[tokio::test]
    async fn test_hook_system_events() {
        let system = HookSystem::with_defaults();
        let mut sub = system.subscribe_events(EventFilter::all()).await;

        system
            .publish_event(Event::ToolExecuted {
                tool_name: "search".into(),
                success: true,
                duration_ms: 42,
            })
            .await
            .unwrap();

        let event = sub.recv().await.unwrap();
        assert!(matches!(event.as_ref(), Event::ToolExecuted { .. }));
    }

    #[tokio::test]
    async fn test_hook_system_chain_by_type() {
        let mut system = HookSystem::with_defaults();

        for chain_type in [
            ChainType::Context,
            ChainType::Tool,
            ChainType::Llm,
            ChainType::Compaction,
            ChainType::Memory,
        ] {
            assert!(system.chain(chain_type).is_empty());
            let mw: Arc<dyn Middleware> = Arc::new(TestMW {
                name: format!("{chain_type:?}"),
                priority: 100,
                chain: chain_type,
            });
            system.chain_mut(chain_type).register(mw).unwrap();
            assert_eq!(system.chain(chain_type).len(), 1);
        }
    }
}
