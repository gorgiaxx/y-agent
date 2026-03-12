//! `InjectTools` pipeline stage (priority 500).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Implements Tool Lazy Loading with dual-mode support:
//!
//! - **`PromptBased`** (default): Injects a compact taxonomy root summary
//!   (~100 tokens) plus any currently-activated tool schemas. The agent
//!   uses `tool_search` to load specific tools on demand.
//!
//! - **Native**: Injects a flat list of tool names plus a `tool_search`
//!   meta-tool definition (backward compatibility).

use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use y_core::provider::ToolCallingMode;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// `InjectTools` — injects tool discovery info into the context.
///
/// Runs at priority 500 (`INJECT_TOOLS`).
pub struct InjectTools {
    /// Available tool names (used in Native mode).
    tool_names: Vec<String>,
    /// Whether to include the `tool_search` meta-tool definition.
    include_tool_search: bool,
    /// Tool calling mode (determines injection strategy).
    mode: ToolCallingMode,
    /// Taxonomy root summary (used in `PromptBased` mode).
    taxonomy_summary: Option<String>,
    /// Currently activated tool schemas (used in `PromptBased` mode).
    /// Static schemas set at construction time.
    activated_tool_schemas: Vec<String>,
    /// Shared dynamic schemas — updated by the service layer when
    /// tools are activated via `tool_search`. Read at `provide()` time.
    dynamic_schemas: Option<Arc<RwLock<Vec<String>>>>,
}

impl InjectTools {
    /// Create a new `InjectTools` provider in Native mode.
    pub fn new(tool_names: Vec<String>) -> Self {
        Self {
            tool_names,
            include_tool_search: true,
            mode: ToolCallingMode::Native,
            taxonomy_summary: None,
            activated_tool_schemas: Vec::new(),
            dynamic_schemas: None,
        }
    }

    /// Create without the `tool_search` meta-tool (Native mode).
    pub fn without_tool_search(tool_names: Vec<String>) -> Self {
        Self {
            tool_names,
            include_tool_search: false,
            mode: ToolCallingMode::Native,
            taxonomy_summary: None,
            activated_tool_schemas: Vec::new(),
            dynamic_schemas: None,
        }
    }

    /// Create in `PromptBased` mode with taxonomy summary.
    pub fn with_taxonomy(taxonomy_summary: String) -> Self {
        Self {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: Some(taxonomy_summary),
            activated_tool_schemas: Vec::new(),
            dynamic_schemas: None,
        }
    }

    /// Create in `PromptBased` mode with taxonomy summary and dynamic schemas.
    ///
    /// The `dynamic_schemas` arc is read at `provide()` time to inject
    /// currently activated tool schemas. The service layer updates this
    /// when tools are activated via `tool_search`.
    pub fn with_taxonomy_and_dynamic_schemas(
        taxonomy_summary: String,
        dynamic_schemas: Arc<RwLock<Vec<String>>>,
    ) -> Self {
        Self {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: Some(taxonomy_summary),
            activated_tool_schemas: Vec::new(),
            dynamic_schemas: Some(dynamic_schemas),
        }
    }

    /// Set the tool calling mode.
    pub fn set_mode(&mut self, mode: ToolCallingMode) {
        self.mode = mode;
    }

    /// Set activated tool schemas (`PromptBased` mode).
    pub fn set_activated_schemas(&mut self, schemas: Vec<String>) {
        self.activated_tool_schemas = schemas;
    }
}

#[async_trait]
impl ContextProvider for InjectTools {
    fn name(&self) -> &'static str {
        "inject_tools"
    }

    fn priority(&self) -> u32 {
        500
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        match self.mode {
            ToolCallingMode::PromptBased => self.provide_prompt_based(ctx),
            ToolCallingMode::Native => self.provide_native(ctx),
        }
    }
}

impl InjectTools {
    /// `PromptBased` mode: inject taxonomy summary + activated tool schemas.
    fn provide_prompt_based(
        &self,
        ctx: &mut AssembledContext,
    ) -> Result<(), ContextPipelineError> {
        let mut content = String::new();

        // Inject taxonomy root summary.
        if let Some(ref summary) = self.taxonomy_summary {
            content.push_str(summary);
        }

        // Collect schemas: static first, then dynamic.
        let mut all_schemas = self.activated_tool_schemas.clone();
        if let Some(ref dynamic) = self.dynamic_schemas {
            // Try to read dynamic schemas; skip if lock is held.
            if let Ok(guard) = dynamic.try_read() {
                all_schemas.extend(guard.iter().cloned());
            }
        }

        // Inject any activated tool schemas.
        if !all_schemas.is_empty() {
            content.push_str("\n\n## Activated Tools\n\n");
            for schema in &all_schemas {
                content.push_str(schema);
                content.push('\n');
            }
        }

        if content.is_empty() {
            return Ok(());
        }

        let tokens = estimate_tokens(&content);
        ctx.add(ContextItem {
            category: ContextCategory::Tools,
            content,
            token_estimate: tokens,
            priority: 500,
        });

        tracing::debug!(
            mode = "prompt_based",
            has_taxonomy = self.taxonomy_summary.is_some(),
            activated_tools = self.activated_tool_schemas.len(),
            tokens,
            "tool context injected (PromptBased mode)"
        );

        Ok(())
    }

    /// Native mode: inject flat tool name list + `tool_search`.
    fn provide_native(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        if self.tool_names.is_empty() {
            return Ok(());
        }

        let mut index = String::from("## Available Tools\n\n");
        for name in &self.tool_names {
            let _ = writeln!(index, "- {name}");
        }

        if self.include_tool_search {
            index.push_str(
                "\n### Meta-Tool: tool_search\n\
                 Use `tool_search(query)` to find and load the full schema \
                 of a specific tool before invoking it.\n",
            );
        }

        let tokens = estimate_tokens(&index);

        ctx.add(ContextItem {
            category: ContextCategory::Tools,
            content: index,
            token_estimate: tokens,
            priority: 500,
        });

        tracing::debug!(
            mode = "native",
            tools = self.tool_names.len(),
            include_tool_search = self.include_tool_search,
            tokens,
            "tool index injected (Native mode)"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Native mode tests (backward compatibility)
    // -----------------------------------------------------------------------

    /// T-P1-06: Provider name and priority; produces compact tool index.
    #[tokio::test]
    async fn test_provider_name_priority_and_index() {
        let provider = InjectTools::new(vec![
            "read_file".into(),
            "write_file".into(),
            "run_command".into(),
        ]);

        assert_eq!(provider.name(), "inject_tools");
        assert_eq!(provider.priority(), 500);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].category, ContextCategory::Tools);
    }

    /// T-P1-07: Tool index contains tool names and tool_search.
    #[tokio::test]
    async fn test_tool_index_contains_names_and_search() {
        let provider = InjectTools::new(vec!["read_file".into(), "write_file".into()]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("read_file"));
        assert!(content.contains("write_file"));
        assert!(content.contains("tool_search"));
    }

    /// Tool index without tool_search meta-tool.
    #[tokio::test]
    async fn test_without_tool_search() {
        let provider = InjectTools::without_tool_search(vec!["read_file".into()]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("read_file"));
        assert!(!content.contains("tool_search"));
    }

    /// Empty tools produce no items.
    #[tokio::test]
    async fn test_empty_tools() {
        let provider = InjectTools::new(vec![]);
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    // -----------------------------------------------------------------------
    // PromptBased mode tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_prompt_based_injects_taxonomy_summary() {
        let provider = InjectTools::with_taxonomy(
            "## Tool Categories\n| file | File ops |\n| shell | Shell exec |".to_string(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        let content = &ctx.items[0].content;
        assert!(content.contains("Tool Categories"));
        assert!(content.contains("file"));
    }

    #[tokio::test]
    async fn test_prompt_based_with_activated_schemas() {
        let mut provider = InjectTools::with_taxonomy("## Categories".to_string());
        provider.set_activated_schemas(vec![
            "file_read: Read a file by path".to_string(),
            "shell_exec: Execute a shell command".to_string(),
        ]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("Activated Tools"));
        assert!(content.contains("file_read"));
        assert!(content.contains("shell_exec"));
    }

    #[tokio::test]
    async fn test_prompt_based_empty_taxonomy_no_items() {
        let provider = InjectTools {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: None,
            activated_tool_schemas: Vec::new(),
            dynamic_schemas: None,
        };

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    #[tokio::test]
    async fn test_set_mode_switches_behavior() {
        let mut provider = InjectTools::new(vec!["read_file".into()]);
        assert_eq!(provider.mode, ToolCallingMode::Native);

        provider.set_mode(ToolCallingMode::PromptBased);
        provider.taxonomy_summary = Some("## Categories".to_string());

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        // Should use PromptBased injection (no "Available Tools" header).
        let content = &ctx.items[0].content;
        assert!(!content.contains("Available Tools"));
        assert!(content.contains("Categories"));
    }
}
