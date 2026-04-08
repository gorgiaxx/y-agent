//! `InjectTools` pipeline stage (priority 500).
//!
//! Design reference: context-session-design.md [Pipeline Stages]
//!
//! Implements Tool Lazy Loading with dual-mode support:
//!
//! - **`PromptBased`** (default): Injects a compact taxonomy root summary
//!   (~100 tokens). The agent uses `ToolSearch` to load specific tool
//!   schemas on demand; full schemas appear in conversation history from
//!   the `ToolSearch` result and are NOT re-injected into the context.
//!
//! - **Native**: Injects a flat list of tool names plus a `ToolSearch`
//!   meta-tool definition (backward compatibility).
//!
//! ## Dynamic mode selection
//!
//! When constructed via [`InjectTools::dynamic`], the provider carries data
//! for **both** modes and reads the active `ToolCallingMode` from a shared
//! `PromptContext` at each `provide()` call. This allows the mode to change
//! per-request (based on which provider is selected) and supports hot-reload.

use std::fmt::Write;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use y_core::provider::ToolCallingMode;
use y_prompt::PromptContext;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Tools blocked during plan mode -- excluded from the prompt tool list.
const PLAN_MODE_BLOCKED_TOOLS: &[&str] = &["FileWrite", "FileEdit", "Task"];

/// `InjectTools` -- injects tool discovery info into the context.
///
/// Runs at priority 500 (`INJECT_TOOLS`).
pub struct InjectTools {
    /// Available tool names (used in Native mode).
    tool_names: Vec<String>,
    /// Whether to include the `ToolSearch` meta-tool definition.
    include_tool_search: bool,
    /// Static tool calling mode (used when `prompt_context` is `None`).
    mode: ToolCallingMode,
    /// Taxonomy root summary (used in `PromptBased` mode).
    taxonomy_summary: Option<String>,
    /// Compact core-tools summary (always-active tools, dynamically generated).
    core_tools_summary: Option<String>,
    /// Shared prompt context for dynamic mode selection.
    ///
    /// When `Some`, `provide()` reads `config_flags["tool_calling.prompt_based"]`
    /// to decide which mode to use, ignoring the static `mode` field.
    /// This enables per-request mode switching driven by the service layer.
    prompt_context: Option<Arc<RwLock<PromptContext>>>,
}

impl InjectTools {
    /// Create a new `InjectTools` provider in Native mode.
    pub fn new(tool_names: Vec<String>) -> Self {
        Self {
            tool_names,
            include_tool_search: true,
            mode: ToolCallingMode::Native,
            taxonomy_summary: None,
            core_tools_summary: None,
            prompt_context: None,
        }
    }

    /// Create without the `ToolSearch` meta-tool (Native mode).
    pub fn without_tool_search(tool_names: Vec<String>) -> Self {
        Self {
            tool_names,
            include_tool_search: false,
            mode: ToolCallingMode::Native,
            taxonomy_summary: None,
            core_tools_summary: None,
            prompt_context: None,
        }
    }

    /// Create in `PromptBased` mode with taxonomy summary.
    pub fn with_taxonomy(taxonomy_summary: String) -> Self {
        Self {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: Some(taxonomy_summary),
            core_tools_summary: None,
            prompt_context: None,
        }
    }

    /// Create in `PromptBased` mode with taxonomy summary and core-tools summary.
    ///
    /// The `core_tools_summary` is a compact description of always-active tools
    /// generated at startup from the `ToolActivationSet`.
    pub fn with_taxonomy_and_core_tools(
        taxonomy_summary: String,
        core_tools_summary: String,
    ) -> Self {
        Self {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: Some(taxonomy_summary),
            core_tools_summary: Some(core_tools_summary),
            prompt_context: None,
        }
    }

    /// Create a **dynamic** `InjectTools` that carries data for both modes
    /// and reads the active mode from `PromptContext` at each `provide()` call.
    ///
    /// The service layer sets `config_flags["tool_calling.prompt_based"]` on
    /// `PromptContext` before each context pipeline assembly, so the injection
    /// strategy matches the provider actually selected for that turn.
    ///
    /// This also supports hot-reload: when providers are reloaded, the
    /// per-request flag is re-evaluated automatically.
    pub fn dynamic(
        tool_names: Vec<String>,
        taxonomy_summary: String,
        core_tools_summary: String,
        prompt_context: Arc<RwLock<PromptContext>>,
    ) -> Self {
        Self {
            tool_names,
            include_tool_search: true,
            // Fallback mode when prompt_context read fails (should not happen).
            mode: ToolCallingMode::Native,
            taxonomy_summary: Some(taxonomy_summary),
            core_tools_summary: Some(core_tools_summary),
            prompt_context: Some(prompt_context),
        }
    }

    /// Set the tool calling mode.
    pub fn set_mode(&mut self, mode: ToolCallingMode) {
        self.mode = mode;
    }

    /// Resolve the effective tool calling mode.
    ///
    /// When a shared `PromptContext` is available (dynamic mode), reads
    /// the `tool_calling.prompt_based` config flag. Otherwise falls back
    /// to the static `mode` field.
    async fn resolve_mode(&self) -> ToolCallingMode {
        if let Some(ref ctx) = self.prompt_context {
            let pctx = ctx.read().await;
            if pctx
                .config_flags
                .get("tool_calling.prompt_based")
                .copied()
                .unwrap_or(false)
            {
                return ToolCallingMode::PromptBased;
            }
            return ToolCallingMode::Native;
        }
        self.mode
    }

    /// Check whether plan mode is currently active.
    async fn is_plan_mode_active(&self) -> bool {
        if let Some(ref ctx) = self.prompt_context {
            let pctx = ctx.read().await;
            return pctx
                .config_flags
                .get("plan_mode.active")
                .copied()
                .unwrap_or(false);
        }
        false
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
        let mode = self.resolve_mode().await;
        let plan_mode_active = self.is_plan_mode_active().await;
        match mode {
            ToolCallingMode::PromptBased => self.provide_prompt_based(ctx),
            ToolCallingMode::Native => self.provide_native(ctx, plan_mode_active),
        }
        Ok(())
    }
}

impl InjectTools {
    /// `PromptBased` mode: inject taxonomy summary only.
    ///
    /// Full tool schemas are loaded lazily via `ToolSearch` and appear
    /// in the conversation history; they are NOT re-injected here.
    fn provide_prompt_based(&self, ctx: &mut AssembledContext) {
        let has_taxonomy = self
            .taxonomy_summary
            .as_ref()
            .is_some_and(|s| !s.is_empty());
        let has_core = self
            .core_tools_summary
            .as_ref()
            .is_some_and(|s| !s.is_empty());

        if !has_taxonomy && !has_core {
            return;
        }

        let mut content = y_tools::parser::PROMPT_TOOL_CALL_SYNTAX.to_string();

        if let Some(ref summary) = self.taxonomy_summary {
            if !summary.is_empty() {
                content.push_str("\n\n");
                content.push_str(summary);
            }
        }

        // Append core-tools summary (always-active tools).
        if let Some(ref core) = self.core_tools_summary {
            if !core.is_empty() {
                content.push_str("\n\n");
                content.push_str(core);
            }
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
            has_core_tools = self.core_tools_summary.is_some(),
            tokens,
            "tool context injected (PromptBased mode)"
        );
    }

    /// Native mode: inject flat tool name list + `ToolSearch`.
    fn provide_native(&self, ctx: &mut AssembledContext, plan_mode_active: bool) {
        if self.tool_names.is_empty() {
            return;
        }

        let mut index = String::from("## Available Tools\n\n");
        for name in &self.tool_names {
            if plan_mode_active && PLAN_MODE_BLOCKED_TOOLS.contains(&name.as_str()) {
                continue;
            }
            let _ = writeln!(index, "- {name}");
        }

        if self.include_tool_search {
            index.push_str(
                "\n### Meta-Tool: ToolSearch\n\
                 Use `ToolSearch(query)` to find and load the full schema \
                 of a specific tool before invoking it.\n\n\
                 Only tools with schemas in the API are directly callable. \
                 For all other tools listed above, call ToolSearch first \
                 to load their schema.\n",
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

    /// T-P1-07: Tool index contains tool names and `ToolSearch`.
    #[tokio::test]
    async fn test_tool_index_contains_names_and_search() {
        let provider = InjectTools::new(vec!["read_file".into(), "write_file".into()]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("read_file"));
        assert!(content.contains("write_file"));
        assert!(content.contains("ToolSearch"));
    }

    /// Tool index without `ToolSearch` meta-tool.
    #[tokio::test]
    async fn test_without_tool_search() {
        let provider = InjectTools::without_tool_search(vec!["read_file".into()]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("read_file"));
        assert!(!content.contains("ToolSearch"));
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
    async fn test_prompt_based_taxonomy_only_no_schemas() {
        let provider = InjectTools::with_taxonomy("## Categories\n- file: File ops".to_string());

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        let content = &ctx.items[0].content;
        assert!(content.contains("Categories"));
        // No "Activated Tools" section -- schemas are lazily loaded.
        assert!(!content.contains("Activated Tools"));
    }

    #[tokio::test]
    async fn test_prompt_based_empty_taxonomy_no_items() {
        let provider = InjectTools {
            tool_names: Vec::new(),
            include_tool_search: true,
            mode: ToolCallingMode::PromptBased,
            taxonomy_summary: None,
            core_tools_summary: None,
            prompt_context: None,
        };

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    #[tokio::test]
    async fn test_prompt_based_with_core_tools() {
        let provider = InjectTools::with_taxonomy_and_core_tools(
            "## Tool Categories\n| file | File ops |".to_string(),
            "## Core Tools\n- FileRead: Read a file\n- ShellExec: Run a command".to_string(),
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        let content = &ctx.items[0].content;
        assert!(content.contains("Tool Categories"));
        assert!(content.contains("Core Tools"));
        assert!(content.contains("FileRead"));
        assert!(content.contains("ShellExec"));
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

    // -----------------------------------------------------------------------
    // Dynamic mode tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_dynamic_mode_reads_prompt_context_native() {
        // PromptContext has no prompt_based flag -> should use Native mode.
        let pctx = Arc::new(RwLock::new(PromptContext::default()));
        let provider = InjectTools::dynamic(
            vec!["read_file".into(), "write_file".into()],
            "## Tool Categories\n| file | File ops |".to_string(),
            "## Core Tools\n- read_file: Read".to_string(),
            pctx,
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        let content = &ctx.items[0].content;
        // Native mode: "Available Tools" header.
        assert!(content.contains("Available Tools"));
        assert!(content.contains("read_file"));
    }

    #[tokio::test]
    async fn test_dynamic_mode_reads_prompt_context_prompt_based() {
        // PromptContext has prompt_based flag -> should use PromptBased mode.
        let pctx = Arc::new(RwLock::new(PromptContext {
            config_flags: {
                let mut m = std::collections::HashMap::new();
                m.insert("tool_calling.prompt_based".into(), true);
                m
            },
            ..Default::default()
        }));
        let provider = InjectTools::dynamic(
            vec!["read_file".into(), "write_file".into()],
            "## Tool Categories\n| file | File ops |".to_string(),
            "## Core Tools\n- read_file: Read".to_string(),
            pctx,
        );

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        let content = &ctx.items[0].content;
        // PromptBased mode: taxonomy content, NOT "Available Tools".
        assert!(content.contains("Tool Categories"));
        assert!(content.contains("Core Tools"));
        assert!(!content.contains("Available Tools"));
    }

    #[tokio::test]
    async fn test_dynamic_mode_switches_between_calls() {
        let pctx = Arc::new(RwLock::new(PromptContext::default()));
        let provider = InjectTools::dynamic(
            vec!["read_file".into()],
            "## Taxonomy Summary".to_string(),
            "## Core Tools\n- read_file: Read".to_string(),
            Arc::clone(&pctx),
        );

        // First call: Native (no flag set).
        {
            let mut ctx = AssembledContext::default();
            provider.provide(&mut ctx).await.unwrap();
            assert!(ctx.items[0].content.contains("Available Tools"));
        }

        // Set flag to prompt_based.
        {
            let mut p = pctx.write().await;
            p.config_flags
                .insert("tool_calling.prompt_based".into(), true);
        }

        // Second call: PromptBased.
        {
            let mut ctx = AssembledContext::default();
            provider.provide(&mut ctx).await.unwrap();
            assert!(ctx.items[0].content.contains("Taxonomy Summary"));
            assert!(!ctx.items[0].content.contains("Available Tools"));
        }

        // Clear flag back to native.
        {
            let mut p = pctx.write().await;
            p.config_flags.remove("tool_calling.prompt_based");
        }

        // Third call: Native again.
        {
            let mut ctx = AssembledContext::default();
            provider.provide(&mut ctx).await.unwrap();
            assert!(ctx.items[0].content.contains("Available Tools"));
        }
    }
}
