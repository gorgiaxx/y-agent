//! Built-in tools shipped with y-agent.
//!
//! These are core tools implemented in Rust, registered at startup.

pub mod file_list;
pub mod file_read;
pub mod file_search;
pub mod file_write;
pub mod knowledge_search;
pub mod shell_exec;
pub mod tool_search;

use std::sync::{Arc, Mutex};
use y_knowledge::middleware::InjectKnowledge;
use y_knowledge::tokenizer::SimpleTokenizer;

use crate::registry::ToolRegistryImpl;

/// Optional knowledge handle for the knowledge search tool.
pub type KnowledgeHandle = Option<Arc<Mutex<InjectKnowledge<SimpleTokenizer>>>>;

/// Register all built-in tools into the given registry.
///
/// Called during service wiring to populate the tool registry with
/// the standard set of tools the agent can use.
///
/// If `knowledge` is `Some`, the `knowledge_search` tool is also registered.
pub async fn register_builtin_tools(
    registry: &ToolRegistryImpl,
    browser_config: y_browser::BrowserConfig,
    knowledge: KnowledgeHandle,
) {
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
            Arc::new(file_list::FileListTool::new()),
            file_list::FileListTool::tool_definition(),
        ),
        (
            Arc::new(shell_exec::ShellExecTool::new()),
            shell_exec::ShellExecTool::tool_definition(),
        ),
        (
            Arc::new(file_search::FileSearchTool::new()),
            file_search::FileSearchTool::tool_definition(),
        ),
        (
            Arc::new(tool_search::ToolSearchTool::new()),
            tool_search::ToolSearchTool::tool_definition(),
        ),
        (
            Arc::new(y_browser::BrowserTool::new(browser_config)),
            y_browser::BrowserTool::tool_definition(),
        ),
    ];

    // Register knowledge search tool if knowledge base is available.
    if let Some(kb) = knowledge {
        tools.push((
            Arc::new(knowledge_search::KnowledgeSearchTool::new(kb)),
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
        register_builtin_tools(&registry, y_browser::BrowserConfig::default(), None).await;
        // 5 core tools + tool_search + browser = 7
        assert_eq!(registry.len().await, 7);
    }

    #[tokio::test]
    async fn test_register_with_knowledge() {
        use y_knowledge::retrieval::{HybridRetriever, RetrievalConfig};

        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let config = RetrievalConfig::default();
        let retriever = HybridRetriever::with_config(SimpleTokenizer::new(), config);
        let knowledge = Arc::new(Mutex::new(InjectKnowledge::new(retriever)));
        register_builtin_tools(
            &registry,
            y_browser::BrowserConfig::default(),
            Some(knowledge),
        )
        .await;
        // 7 + knowledge_search = 8
        assert_eq!(registry.len().await, 8);
    }

    #[tokio::test]
    async fn test_registered_tools_appear_in_index() {
        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        register_builtin_tools(&registry, y_browser::BrowserConfig::default(), None).await;
        let index = registry.tool_index().await;
        let names: Vec<&str> = index.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"file_write"));
        assert!(names.contains(&"file_list"));
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"file_search"));
        assert!(names.contains(&"tool_search"));
    }
}
