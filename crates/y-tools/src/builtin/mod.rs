//! Built-in tools shipped with y-agent.
//!
//! These are core tools implemented in Rust, registered at startup.

pub mod file_list;
pub mod file_read;
pub mod file_search;
pub mod file_write;
pub mod shell_exec;
pub mod tool_search;

use std::sync::Arc;

use crate::registry::ToolRegistryImpl;

/// Register all built-in tools into the given registry.
///
/// Called during service wiring to populate the tool registry with
/// the standard set of tools the agent can use.
pub async fn register_builtin_tools(registry: &ToolRegistryImpl) {
    let tools: Vec<(Arc<dyn y_core::tool::Tool>, y_core::tool::ToolDefinition)> = vec![
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
    ];

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
        register_builtin_tools(&registry).await;
        // 5 core tools + tool_search = 6
        assert_eq!(registry.len().await, 6);
    }

    #[tokio::test]
    async fn test_registered_tools_appear_in_index() {
        let registry = ToolRegistryImpl::new(ToolRegistryConfig::default());
        register_builtin_tools(&registry).await;
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
