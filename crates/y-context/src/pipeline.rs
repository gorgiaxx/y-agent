//! Context assembly pipeline: ordered sequence of context providers.

use async_trait::async_trait;
use y_core::types::SessionId;

/// A context item contributed by a pipeline stage.
#[derive(Debug, Clone)]
pub struct ContextItem {
    /// Category of this context (for budget tracking).
    pub category: ContextCategory,
    /// Content to include in the prompt.
    pub content: String,
    /// Estimated token count.
    pub token_estimate: u32,
    /// Priority for eviction (lower = evict first).
    pub priority: u32,
}

/// Context category for budget allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextCategory {
    SystemPrompt,
    Bootstrap,
    Memory,
    Skills,
    Tools,
    History,
    Status,
}

/// Request context passed into the pipeline so stages can access
/// session, user query, agent mode, and enabled tools.
#[derive(Debug, Clone, Default)]
pub struct ContextRequest {
    /// Active session identifier.
    pub session_id: Option<SessionId>,
    /// Current user query / message.
    pub user_query: String,
    /// Agent mode (e.g. "general", "build", "plan", "explore").
    pub agent_mode: String,
    /// Tool names currently enabled for the agent.
    pub tools_enabled: Vec<String>,
}

/// Assembled context ready for the LLM.
#[derive(Debug, Clone, Default)]
pub struct AssembledContext {
    /// All context items in pipeline order.
    pub items: Vec<ContextItem>,
    /// Optional request context that pipeline stages can read.
    pub request: Option<ContextRequest>,
}

impl AssembledContext {
    /// Total estimated tokens across all items.
    pub fn total_tokens(&self) -> u32 {
        self.items.iter().map(|i| i.token_estimate).sum()
    }

    /// Tokens used by a specific category.
    pub fn tokens_for(&self, category: ContextCategory) -> u32 {
        self.items
            .iter()
            .filter(|i| i.category == category)
            .map(|i| i.token_estimate)
            .sum()
    }

    /// Add a context item.
    pub fn add(&mut self, item: ContextItem) {
        self.items.push(item);
    }
}

/// A stage in the context assembly pipeline.
///
/// Each stage contributes context items to the assembled context.
/// Implemented as trait objects for extensibility.
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Provider name (for logging).
    fn name(&self) -> &str;

    /// Pipeline priority (lower executes first).
    fn priority(&self) -> u32;

    /// Contribute context items.
    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError>;
}

/// Error from a pipeline stage.
#[derive(Debug, thiserror::Error)]
pub enum ContextPipelineError {
    #[error("provider {name} failed: {message}")]
    ProviderFailed { name: String, message: String },
}

/// The context pipeline runs providers in priority order.
pub struct ContextPipeline {
    providers: Vec<Box<dyn ContextProvider>>,
}

impl ContextPipeline {
    /// Create a new empty pipeline.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a context provider.
    pub fn register(&mut self, provider: Box<dyn ContextProvider>) {
        self.providers.push(provider);
        self.providers.sort_by_key(|p| p.priority());
    }

    /// Run all providers in priority order.
    pub async fn assemble(&self) -> Result<AssembledContext, ContextPipelineError> {
        self.assemble_with_request(None).await
    }

    /// Run all providers with a request context available to each stage.
    pub async fn assemble_with_request(
        &self,
        request: Option<ContextRequest>,
    ) -> Result<AssembledContext, ContextPipelineError> {
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request,
        };
        for provider in &self.providers {
            tracing::debug!(provider = %provider.name(), priority = provider.priority(), "running context provider");
            // Fail-open: log error but continue with other providers.
            if let Err(e) = provider.provide(&mut ctx).await {
                tracing::warn!(provider = %provider.name(), error = %e, "context provider failed; skipping");
            }
        }
        Ok(ctx)
    }

    /// Number of registered providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl Default for ContextPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        name: String,
        priority: u32,
        content: String,
    }

    #[async_trait]
    impl ContextProvider for TestProvider {
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
    async fn test_pipeline_executes_in_priority_order() {
        let mut pipeline = ContextPipeline::new();
        pipeline.register(Box::new(TestProvider {
            name: "second".into(),
            priority: 200,
            content: "B".into(),
        }));
        pipeline.register(Box::new(TestProvider {
            name: "first".into(),
            priority: 100,
            content: "A".into(),
        }));

        let ctx = pipeline.assemble().await.unwrap();
        assert_eq!(ctx.items.len(), 2);
        assert_eq!(ctx.items[0].content, "A");
        assert_eq!(ctx.items[1].content, "B");
    }

    #[test]
    fn test_assembled_context_tokens() {
        let mut ctx = AssembledContext::default();
        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "system".into(),
            token_estimate: 100,
            priority: 0,
        });
        ctx.add(ContextItem {
            category: ContextCategory::History,
            content: "history".into(),
            token_estimate: 500,
            priority: 0,
        });
        assert_eq!(ctx.total_tokens(), 600);
        assert_eq!(ctx.tokens_for(ContextCategory::SystemPrompt), 100);
        assert_eq!(ctx.tokens_for(ContextCategory::History), 500);
    }

    #[tokio::test]
    async fn test_pipeline_fail_open() {
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
                    message: "test failure".into(),
                })
            }
        }

        let mut pipeline = ContextPipeline::new();
        pipeline.register(Box::new(FailingProvider));
        pipeline.register(Box::new(TestProvider {
            name: "good".into(),
            priority: 200,
            content: "OK".into(),
        }));

        // Should succeed despite the failing provider.
        let ctx = pipeline.assemble().await.unwrap();
        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].content, "OK");
    }
}
