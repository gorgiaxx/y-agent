//! Adapter that bridges `ContextProvider` stages to `y-hooks` `Middleware`.
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Each `ContextProvider` is wrapped as a `Middleware` so the context
//! assembly pipeline can participate in the middleware chain. The adapter
//! registers stages by priority (100–700) matching the design spec:
//!
//! | Priority | Stage               |
//! |----------|---------------------|
//! | 100      | BuildSystemPrompt   |
//! | 200      | InjectBootstrap     |
//! | 300      | InjectMemory        |
//! | 400      | InjectSkills        |
//! | 500      | InjectTools         |
//! | 600      | LoadHistory         |
//! | 700      | InjectContextStatus |

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};

use crate::pipeline::{AssembledContext, ContextPipelineError, ContextProvider};

/// Wraps a `ContextProvider` as a `y-hooks` `Middleware`.
///
/// The assembled context is stored in a shared `Arc<Mutex<AssembledContext>>`
/// so that the middleware chain can collect context from multiple stages
/// and the final result can be extracted after chain execution.
pub struct ContextMiddlewareAdapter {
    provider: Box<dyn ContextProvider>,
    shared_context: Arc<Mutex<AssembledContext>>,
}

impl ContextMiddlewareAdapter {
    /// Create a new adapter wrapping a `ContextProvider`.
    pub fn new(
        provider: Box<dyn ContextProvider>,
        shared_context: Arc<Mutex<AssembledContext>>,
    ) -> Self {
        Self {
            provider,
            shared_context,
        }
    }
}

#[async_trait]
impl Middleware for ContextMiddlewareAdapter {
    async fn execute(
        &self,
        _ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let mut assembled = self.shared_context.lock().await;
        match self.provider.provide(&mut assembled).await {
            Ok(()) => Ok(MiddlewareResult::Continue),
            Err(ContextPipelineError::ProviderFailed { name, message }) => {
                tracing::warn!(
                    provider = %name,
                    error = %message,
                    "context provider failed in middleware chain; continuing"
                );
                // Fail-open: context pipeline continues even if a stage fails.
                Ok(MiddlewareResult::Continue)
            }
        }
    }

    fn chain_type(&self) -> ChainType {
        ChainType::Context
    }

    fn priority(&self) -> u32 {
        self.provider.priority()
    }

    fn name(&self) -> &str {
        self.provider.name()
    }
}

/// Standard pipeline stage priorities matching the design specification.
pub mod stage_priorities {
    /// `BuildSystemPrompt`: assembles the base system prompt.
    pub const BUILD_SYSTEM_PROMPT: u32 = 100;
    /// `InjectBootstrap`: injects bootstrap context.
    pub const INJECT_BOOTSTRAP: u32 = 200;
    /// `InjectMemory`: adds long-term and working memory.
    pub const INJECT_MEMORY: u32 = 300;
    /// `InjectKnowledge`: adds relevant knowledge from the knowledge base.
    pub const INJECT_KNOWLEDGE: u32 = 350;
    /// `InjectSkills`: adds available skill descriptions.
    pub const INJECT_SKILLS: u32 = 400;
    /// `InjectTools`: adds tool schema summaries.
    pub const INJECT_TOOLS: u32 = 500;
    /// `LoadHistory`: loads conversation history.
    pub const LOAD_HISTORY: u32 = 600;
    /// `InjectContextStatus`: adds context budget status.
    pub const INJECT_CONTEXT_STATUS: u32 = 700;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{AssembledContext, ContextCategory, ContextItem, ContextProvider};

    struct MockContextProvider {
        name: String,
        priority: u32,
        content: String,
    }

    #[async_trait]
    impl ContextProvider for MockContextProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> u32 {
            self.priority
        }
        async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.content.clone(),
                token_estimate: 10,
                priority: self.priority,
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_adapter_provides_context() {
        let shared = Arc::new(Mutex::new(AssembledContext::default()));
        let adapter = ContextMiddlewareAdapter::new(
            Box::new(MockContextProvider {
                name: "test_provider".into(),
                priority: 100,
                content: "test content".into(),
            }),
            Arc::clone(&shared),
        );

        let mut mw_ctx = MiddlewareContext {
            chain_type: ChainType::Context,
            payload: serde_json::json!({}),
            metadata: serde_json::json!([]),
            aborted: false,
            abort_reason: None,
        };

        let result = adapter.execute(&mut mw_ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));

        let assembled = shared.lock().await;
        assert_eq!(assembled.items.len(), 1);
        assert_eq!(assembled.items[0].content, "test content");
    }

    #[tokio::test]
    async fn test_adapter_preserves_priority_and_name() {
        let shared = Arc::new(Mutex::new(AssembledContext::default()));
        let adapter = ContextMiddlewareAdapter::new(
            Box::new(MockContextProvider {
                name: "inject_memory".into(),
                priority: 300,
                content: "memory data".into(),
            }),
            Arc::clone(&shared),
        );

        assert_eq!(adapter.name(), "inject_memory");
        assert_eq!(adapter.priority(), 300);
        assert_eq!(adapter.chain_type(), ChainType::Context);
    }

    #[tokio::test]
    async fn test_adapter_fail_open_on_provider_error() {
        struct FailingProvider;

        #[async_trait]
        impl ContextProvider for FailingProvider {
            fn name(&self) -> &'static str {
                "failing"
            }
            fn priority(&self) -> u32 {
                100
            }
            async fn provide(
                &self,
                _ctx: &mut AssembledContext,
            ) -> Result<(), ContextPipelineError> {
                Err(ContextPipelineError::ProviderFailed {
                    name: "failing".into(),
                    message: "something went wrong".into(),
                })
            }
        }

        let shared = Arc::new(Mutex::new(AssembledContext::default()));
        let adapter = ContextMiddlewareAdapter::new(Box::new(FailingProvider), Arc::clone(&shared));

        let mut mw_ctx = MiddlewareContext {
            chain_type: ChainType::Context,
            payload: serde_json::json!({}),
            metadata: serde_json::json!([]),
            aborted: false,
            abort_reason: None,
        };

        // Should not propagate the error — fail open.
        let result = adapter.execute(&mut mw_ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
    }

    #[test]
    fn test_stage_priorities_order() {
        use stage_priorities::*;
        assert!(BUILD_SYSTEM_PROMPT < INJECT_BOOTSTRAP);
        assert!(INJECT_BOOTSTRAP < INJECT_MEMORY);
        assert!(INJECT_MEMORY < INJECT_KNOWLEDGE);
        assert!(INJECT_KNOWLEDGE < INJECT_SKILLS);
        assert!(INJECT_SKILLS < INJECT_TOOLS);
        assert!(INJECT_TOOLS < LOAD_HISTORY);
        assert!(LOAD_HISTORY < INJECT_CONTEXT_STATUS);
    }
}
