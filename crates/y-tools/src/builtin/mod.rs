//! Built-in tools shipped with y-agent.
//!
//! These are core tools implemented in Rust, registered at startup.

mod path_utils;

pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod knowledge_search;
pub mod loop_tool;
pub mod plan;
pub mod shell_exec;
pub mod task;
pub mod tool_search;
pub mod user_interaction;
pub mod web_fetch;
pub mod workflow;

use std::sync::{Arc, Mutex};
use y_core::embedding::EmbeddingProvider;
use y_knowledge::middleware::InjectKnowledge;
use y_knowledge::tokenizer::AutoTokenizer;

use crate::registry::ToolRegistryImpl;

/// Optional knowledge handle for the knowledge search tool.
pub type KnowledgeHandle = Option<Arc<Mutex<InjectKnowledge<AutoTokenizer>>>>;

/// Optional embedding provider for the knowledge search tool.
pub type EmbeddingHandle = Option<Arc<dyn EmbeddingProvider>>;

/// Register all built-in tools into the given registry.
///
/// Called during service wiring to populate the tool registry with
/// the standard set of tools the agent can use.
///
/// If `knowledge` is `Some`, the `KnowledgeSearch` tool is also registered.
/// When `embedding` is also `Some`, the tool uses cosine similarity for
/// query matching instead of text-based fallback.
pub async fn register_builtin_tools(
    registry: &ToolRegistryImpl,
    browser_config: y_browser::BrowserConfig,
    knowledge: KnowledgeHandle,
    embedding: EmbeddingHandle,
) {
    // Browser tool is shared between `Browser` and `WebFetch` via Arc
    // so both use the same Chrome session.
    let browser_tool = Arc::new(y_browser::BrowserTool::new(browser_config));

    let mut tools: Vec<(Arc<dyn y_core::tool::Tool>, y_core::tool::ToolDefinition)> = vec![
        (
            Arc::new(file_read::FileReadTool::new()),
            file_read::FileReadTool::tool_definition(),
        ),
        (
            Arc::new(file_write::FileWriteTool::new()),
            file_write::FileWriteTool::tool_definition(),
        ),
        (
            Arc::new(file_edit::FileEditTool::new()),
            file_edit::FileEditTool::tool_definition(),
        ),
        (
            Arc::new(shell_exec::ShellExecTool::new()),
            shell_exec::ShellExecTool::tool_definition(),
        ),
        (
            Arc::new(task::TaskTool::new()),
            task::TaskTool::tool_definition(),
        ),
        (
            Arc::new(user_interaction::AskUserTool::new()),
            user_interaction::AskUserTool::tool_definition(),
        ),
        (
            Arc::new(tool_search::ToolSearchTool::new()),
            tool_search::ToolSearchTool::tool_definition(),
        ),
        (
            Arc::new(glob::GlobTool::new()),
            glob::GlobTool::tool_definition(),
        ),
        (
            Arc::new(grep::GrepTool::new()),
            grep::GrepTool::tool_definition(),
        ),
        (
            Arc::clone(&browser_tool) as Arc<dyn y_core::tool::Tool>,
            y_browser::BrowserTool::tool_definition(),
        ),
        (
            Arc::new(web_fetch::WebFetchTool::new(Arc::clone(&browser_tool))),
            web_fetch::WebFetchTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowCreateTool::new()),
            workflow::WorkflowCreateTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowListTool::new()),
            workflow::WorkflowListTool::tool_definition(),
        ),
        (
            Arc::new(workflow::ScheduleCreateTool::new()),
            workflow::ScheduleCreateTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowGetTool::new()),
            workflow::WorkflowGetTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowUpdateTool::new()),
            workflow::WorkflowUpdateTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowDeleteTool::new()),
            workflow::WorkflowDeleteTool::tool_definition(),
        ),
        (
            Arc::new(workflow::WorkflowValidateTool::new()),
            workflow::WorkflowValidateTool::tool_definition(),
        ),
        (
            Arc::new(workflow::ScheduleListTool::new()),
            workflow::ScheduleListTool::tool_definition(),
        ),
        (
            Arc::new(workflow::SchedulePauseTool::new()),
            workflow::SchedulePauseTool::tool_definition(),
        ),
        (
            Arc::new(workflow::ScheduleResumeTool::new()),
            workflow::ScheduleResumeTool::tool_definition(),
        ),
        (
            Arc::new(workflow::ScheduleDeleteTool::new()),
            workflow::ScheduleDeleteTool::tool_definition(),
        ),
        (
            Arc::new(plan::PlanTool::new()),
            plan::PlanTool::tool_definition(),
        ),
        (
            Arc::new(loop_tool::LoopTool::new()),
            loop_tool::LoopTool::tool_definition(),
        ),
    ];

    // Register knowledge search tool if knowledge base is available.
    if let Some(kb) = knowledge {
        let tool = if let Some(emb) = embedding {
            knowledge_search::KnowledgeSearchTool::with_embedding(kb, emb)
        } else {
            knowledge_search::KnowledgeSearchTool::new(kb)
        };
        tools.push((
            Arc::new(tool),
            knowledge_search::KnowledgeSearchTool::tool_definition(),
        ));
    }

    for (tool, def) in tools {
        if let Err(e) = registry.register_tool(tool, def).await {
            tracing::warn!(error = %e, "failed to register built-in tool");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolRegistryConfig;
    use y_core::tool::ToolRegistry;

    #[tokio::test]
    async fn test_register_builtin_tools_populates_registry() {
        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        register_builtin_tools(&registry, y_browser::BrowserConfig::default(), None, None).await;
        // 3 core + file_edit + Task + ToolSearch + Glob + Grep + AskUser + Browser + WebFetch
        // + 11 workflow/schedule + 1 plan = 23
        assert_eq!(registry.len().await, 23);
    }

    #[tokio::test]
    async fn test_register_with_knowledge() {
        use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};

        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let config = RetrievalConfig::default();
        let retriever = HybridRetriever::with_config(AutoTokenizer::new(), config);
        let knowledge = Arc::new(Mutex::new(InjectKnowledge::new(retriever)));
        register_builtin_tools(
            &registry,
            y_browser::BrowserConfig::default(),
            Some(knowledge),
            None,
        )
        .await;
        // 23 + KnowledgeSearch = 24
        assert_eq!(registry.len().await, 24);
    }

    #[tokio::test]
    async fn test_registered_tools_appear_in_index() {
        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        register_builtin_tools(&registry, y_browser::BrowserConfig::default(), None, None).await;
        let index = registry.tool_index().await;
        let names: Vec<&str> = index.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"FileRead"));
        assert!(names.contains(&"FileWrite"));
        assert!(names.contains(&"FileEdit"));
        assert!(names.contains(&"ShellExec"));
        assert!(names.contains(&"ToolSearch"));
        assert!(names.contains(&"WebFetch"));
        assert!(names.contains(&"Glob"));
        assert!(names.contains(&"Grep"));
        assert!(names.contains(&"Task"));
    }
}
