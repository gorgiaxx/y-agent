//! Knowledge context provider -- injects knowledge-base awareness into context.
//!
//! Priority 350 (between `InjectMemory` at 300 and `InjectSkills` at 400).
//!
//! When the user selects knowledge collections in the GUI, this provider
//! injects a prompt hint telling the LLM which collections are available
//! and that it can use the `KnowledgeSearch` tool to query them. The LLM
//! autonomously decides when and how to search -- no pre-search is performed.

use async_trait::async_trait;
use std::fmt::Write;

use crate::middleware_adapter::stage_priorities;
use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Context provider that injects knowledge-base awareness into the prompt.
///
/// When knowledge collections are selected by the user, a short prompt
/// hint is injected informing the LLM about the available collections
/// and the `KnowledgeSearch` tool. The LLM decides autonomously whether
/// and how to search -- no embedding or retrieval is performed at this
/// stage.
pub struct KnowledgeContextProvider;

impl KnowledgeContextProvider {
    /// Create a new knowledge context provider.
    pub fn new() -> Self {
        Self
    }

    /// Build the prompt hint for the LLM.
    fn build_knowledge_hint(collections: &[String]) -> String {
        let mut hint = String::from("<knowledge_context>\n");
        hint.push_str(
            "The user has selected the following knowledge base collections \
             for this session:\n",
        );
        for (i, name) in collections.iter().enumerate() {
            let _ = writeln!(&mut hint, "  {}. {}", i + 1, name);
        }
        hint.push_str(
            "\nYou have access to the `KnowledgeSearch` tool which can search \
             these collections. Use it when the user's question may benefit \
             from information stored in the knowledge base. Formulate your \
             search query as a natural-language description of the \
             information you need.\n",
        );
        hint.push_str("</knowledge_context>");
        hint
    }
}

impl Default for KnowledgeContextProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextProvider for KnowledgeContextProvider {
    fn name(&self) -> &'static str {
        "inject_knowledge"
    }

    fn priority(&self) -> u32 {
        stage_priorities::INJECT_KNOWLEDGE
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        // Extract collections from the request context.
        let collections = match &ctx.request {
            Some(req) if !req.knowledge_collections.is_empty() => req.knowledge_collections.clone(),
            _ => {
                tracing::debug!("knowledge context: skipped (no collections selected)");
                return Ok(());
            }
        };

        // Inject a prompt hint so the LLM knows it can use KnowledgeSearch.
        let content = Self::build_knowledge_hint(&collections);
        // Conservative token estimate: ~10 tokens per collection name + fixed overhead.
        let token_estimate = 60 + (collections.len() as u32) * 10;

        ctx.add(ContextItem {
            category: ContextCategory::Knowledge,
            content,
            token_estimate,
            priority: self.priority(),
        });

        tracing::info!(
            collections = ?collections,
            tokens = token_estimate,
            "knowledge context: injected awareness hint for KnowledgeSearch tool"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider() -> KnowledgeContextProvider {
        KnowledgeContextProvider::new()
    }

    #[tokio::test]
    async fn test_injects_hint_when_collections_selected() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "How does Rust handle errors?".to_string(),
                knowledge_collections: vec!["rust-docs".to_string()],
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert_eq!(ctx.items.len(), 1, "should inject exactly one hint item");
        assert_eq!(ctx.items[0].category, ContextCategory::Knowledge);
        assert!(
            ctx.items[0].content.contains("knowledge_context"),
            "should contain knowledge_context tags"
        );
        assert!(
            ctx.items[0].content.contains("rust-docs"),
            "should mention the collection name"
        );
        assert!(
            ctx.items[0].content.contains("KnowledgeSearch"),
            "should mention the KnowledgeSearch tool"
        );
    }

    #[tokio::test]
    async fn test_injects_hint_with_multiple_collections() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "test query".to_string(),
                knowledge_collections: vec![
                    "docs".to_string(),
                    "notes".to_string(),
                    "research".to_string(),
                ],
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert_eq!(ctx.items.len(), 1);
        let content = &ctx.items[0].content;
        assert!(content.contains("docs"), "should list all collections");
        assert!(content.contains("notes"));
        assert!(content.contains("research"));
    }

    #[tokio::test]
    async fn test_skips_when_no_collections_selected() {
        let provider = make_provider();
        let mut ctx = AssembledContext {
            items: Vec::new(),
            request: Some(crate::pipeline::ContextRequest {
                user_query: "How does Rust handle errors?".to_string(),
                knowledge_collections: vec![],
                ..Default::default()
            }),
        };

        provider.provide(&mut ctx).await.unwrap();
        assert!(
            ctx.items.is_empty(),
            "should skip when no collections selected"
        );
    }

    #[tokio::test]
    async fn test_skips_when_no_request() {
        let provider = make_provider();
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty(), "should skip without request");
    }

    #[test]
    fn test_priority_and_name() {
        let provider = make_provider();
        assert_eq!(provider.name(), "inject_knowledge");
        assert_eq!(provider.priority(), 350);
    }

    #[test]
    fn test_build_knowledge_hint_format() {
        let hint = KnowledgeContextProvider::build_knowledge_hint(&["my-collection".to_string()]);
        assert!(hint.starts_with("<knowledge_context>"));
        assert!(hint.ends_with("</knowledge_context>"));
        assert!(hint.contains("my-collection"));
        assert!(hint.contains("KnowledgeSearch"));
    }
}
