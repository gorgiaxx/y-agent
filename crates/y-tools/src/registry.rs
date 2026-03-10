//! `ToolRegistryImpl`: manages tool registration, lookup, and search.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolIndexEntry, ToolRegistry,
};
use y_core::types::ToolName;

use crate::config::ToolRegistryConfig;
use crate::error::ToolRegistryError;
use crate::index::ToolIndex;

/// Main implementation of the [`ToolRegistry`] trait.
///
/// Stores tool instances and their definitions, maintains a compact index,
/// and supports category/keyword search for lazy loading.
///
/// Uses interior mutability (`RwLock`) so the trait's `&self` methods work.
pub struct ToolRegistryImpl {
    inner: RwLock<RegistryInner>,
    config: ToolRegistryConfig,
}

struct RegistryInner {
    /// Tool instances keyed by name.
    tools: HashMap<ToolName, Arc<dyn Tool>>,
    /// Tool definitions keyed by name.
    definitions: HashMap<ToolName, ToolDefinition>,
    /// Compact index for LLM context injection.
    index: ToolIndex,
}

impl ToolRegistryImpl {
    /// Create a new tool registry with the given configuration.
    pub fn new(config: ToolRegistryConfig) -> Self {
        Self {
            inner: RwLock::new(RegistryInner {
                tools: HashMap::new(),
                definitions: HashMap::new(),
                index: ToolIndex::new(),
            }),
            config,
        }
    }

    /// Register a tool with its definition (direct method, not trait).
    pub async fn register_tool(
        &self,
        tool: Arc<dyn Tool>,
        definition: ToolDefinition,
    ) -> Result<(), ToolRegistryError> {
        let mut inner = self.inner.write().await;
        if inner.tools.contains_key(&definition.name) {
            return Err(ToolRegistryError::DuplicateName {
                name: definition.name.as_str().to_string(),
            });
        }

        inner.index.add(&definition);
        inner.tools.insert(definition.name.clone(), tool);
        inner.definitions.insert(definition.name.clone(), definition);
        Ok(())
    }

    /// Get a tool instance by name.
    pub async fn get_tool(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        let inner = self.inner.read().await;
        inner.tools.get(name).cloned()
    }

    /// Get a tool definition by name.
    pub async fn get_definition(&self, name: &ToolName) -> Option<ToolDefinition> {
        let inner = self.inner.read().await;
        inner.definitions.get(name).cloned()
    }

    /// Search for tools by keyword in name/description and optional category.
    pub async fn search_tools(
        &self,
        query: &str,
        category: Option<&ToolCategory>,
    ) -> Vec<ToolDefinition> {
        let inner = self.inner.read().await;
        let query_lower = query.to_lowercase();
        let limit = self.config.search_limit;

        inner
            .index
            .entries()
            .iter()
            .filter(|entry| {
                let name_match = entry.name.as_str().to_lowercase().contains(&query_lower);
                let desc_match = entry.description.to_lowercase().contains(&query_lower);
                let cat_match = category.is_none_or(|c| &entry.category == c);
                (name_match || desc_match) && cat_match
            })
            .filter_map(|entry| inner.definitions.get(&entry.name).cloned())
            .take(limit)
            .collect()
    }

    /// Number of registered tools.
    pub async fn len(&self) -> usize {
        self.inner.read().await.tools.len()
    }

    /// Whether the registry is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.tools.is_empty()
    }
}

#[async_trait]
impl ToolRegistry for ToolRegistryImpl {
    async fn tool_index(&self) -> Vec<ToolIndexEntry> {
        self.inner.read().await.index.entries()
    }

    async fn search(&self, query: &str) -> Result<Vec<ToolDefinition>, ToolError> {
        Ok(self.search_tools(query, None).await)
    }

    async fn get(&self, name: &ToolName) -> Result<Box<dyn Tool>, ToolError> {
        let inner = self.inner.read().await;
        inner
            .tools
            .get(name)
            .map(|t| {
                // Wrap in a thin forwarding impl to convert Arc<dyn Tool> to Box<dyn Tool>.
                Box::new(ToolRef(t.clone())) as Box<dyn Tool>
            })
            .ok_or_else(|| ToolError::NotFound {
                name: name.as_str().to_string(),
            })
    }

    async fn register(&self, definition: ToolDefinition) -> Result<(), ToolError> {
        let mut inner = self.inner.write().await;
        if inner.definitions.contains_key(&definition.name) {
            return Err(ToolError::Other {
                message: format!("duplicate tool name: {}", definition.name.as_str()),
            });
        }
        inner.index.add(&definition);
        inner.definitions.insert(definition.name.clone(), definition);
        Ok(())
    }

    async fn unregister(&self, name: &ToolName) -> Result<(), ToolError> {
        let mut inner = self.inner.write().await;
        if inner.definitions.remove(name).is_none() {
            return Err(ToolError::NotFound {
                name: name.as_str().to_string(),
            });
        }
        inner.tools.remove(name);
        inner.index.remove(name);
        Ok(())
    }
}

/// Thin wrapper to convert `Arc<dyn Tool>` to `Box<dyn Tool>`.
struct ToolRef(Arc<dyn Tool>);

#[async_trait]
impl Tool for ToolRef {
    async fn execute(
        &self,
        input: y_core::tool::ToolInput,
    ) -> Result<y_core::tool::ToolOutput, ToolError> {
        self.0.execute(input).await
    }

    fn definition(&self) -> &ToolDefinition {
        self.0.definition()
    }
}

#[cfg(test)]
mod tests {
    use y_core::runtime::RuntimeCapability;
    use y_core::tool::{ToolInput, ToolOutput, ToolType};

    use super::*;

    /// A no-op tool for testing.
    struct NoopTool {
        def: ToolDefinition,
    }

    #[async_trait]
    impl Tool for NoopTool {
        async fn execute(&self, _input: ToolInput) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                success: true,
                content: serde_json::json!({"status": "ok"}),
                warnings: vec![],
                metadata: serde_json::json!({}),
            })
        }

        fn definition(&self) -> &ToolDefinition {
            &self.def
        }
    }

    fn make_tool(name: &str) -> (Arc<dyn Tool>, ToolDefinition) {
        let def = ToolDefinition {
            name: ToolName::from_string(name),
            description: format!("{name} tool"),
            parameters: serde_json::json!({}),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        };
        let tool = Arc::new(NoopTool { def: def.clone() }) as Arc<dyn Tool>;
        (tool, def)
    }

    #[tokio::test]
    async fn test_registry_register_and_get() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("file_read");
        reg.register_tool(tool, def).await.unwrap();
        assert!(reg.get_tool(&ToolName::from_string("file_read")).await.is_some());
        assert_eq!(reg.len().await, 1);
    }

    #[tokio::test]
    async fn test_registry_duplicate_name_rejected() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool1, def1) = make_tool("file_read");
        let (tool2, def2) = make_tool("file_read");
        reg.register_tool(tool1, def1).await.unwrap();
        let err = reg.register_tool(tool2, def2).await.unwrap_err();
        assert!(matches!(err, ToolRegistryError::DuplicateName { .. }));
    }

    #[tokio::test]
    async fn test_registry_unregister() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("file_read");
        reg.register_tool(tool, def).await.unwrap();
        ToolRegistry::unregister(&reg, &ToolName::from_string("file_read"))
            .await
            .unwrap();
        assert!(reg.get_tool(&ToolName::from_string("file_read")).await.is_none());
        assert_eq!(reg.len().await, 0);
    }

    #[tokio::test]
    async fn test_registry_search_by_keyword() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        for name in &["file_read", "file_write", "web_search", "code_exec"] {
            let (tool, def) = make_tool(name);
            reg.register_tool(tool, def).await.unwrap();
        }
        let results = reg.search_tools("file", None).await;
        assert_eq!(results.len(), 2);
        let names: Vec<String> = results.iter().map(|r| r.name.as_str().to_string()).collect();
        assert!(names.contains(&"file_read".to_string()));
        assert!(names.contains(&"file_write".to_string()));
    }

    #[tokio::test]
    async fn test_registry_search_respects_limit() {
        let config = ToolRegistryConfig {
            search_limit: 2,
            ..Default::default()
        };
        let reg = ToolRegistryImpl::new(config);
        for i in 0..10 {
            let (tool, def) = make_tool(&format!("tool_{i}"));
            reg.register_tool(tool, def).await.unwrap();
        }
        let results = reg.search_tools("tool", None).await;
        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn test_registry_trait_tool_index() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("file_read");
        reg.register_tool(tool, def).await.unwrap();

        let index = ToolRegistry::tool_index(&reg).await;
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].name.as_str(), "file_read");
    }
}
