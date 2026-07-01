//! `ToolRegistryImpl`: manages tool registration, lookup, and search.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use async_trait::async_trait;
use tokio::sync::RwLock;

use y_core::tool::{Tool, ToolCategory, ToolDefinition, ToolError, ToolIndexEntry, ToolRegistry};
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
/// Type alias for a tool availability check function.
pub type ToolCheckFn = Arc<dyn Fn() -> bool + Send + Sync>;

/// Cached result of a tool availability check.
#[derive(Clone)]
struct CachedCheck {
    result: bool,
    /// Whether the `check_fn` panicked. Panic results use a shorter TTL
    /// (`CHECK_GRACE`) to retry sooner.
    panicked: bool,
    checked_at: std::time::Instant,
}
/// TTL for cached `check_fn` results (30 seconds).
const CHECK_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// Grace period after a `check_fn` panic (60 seconds).
const CHECK_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

/// Main implementation of the [`ToolRegistry`] trait.
///
/// Stores tool instances and their definitions, maintains a compact index,
/// and supports category/keyword search for lazy loading.
///
/// Tools may declare a `check_fn` (via [`ToolRegistryImpl::set_check_fn`])
/// that gates whether the tool is exposed to the LLM. When `check_fn`
/// returns `false`, the tool's schema is omitted from `get_all_definitions()`,
/// saving context tokens. Results are cached for 30s with a 60s grace period
/// on failure.
pub struct ToolRegistryImpl {
    inner: RwLock<RegistryInner>,
    config: StdRwLock<ToolRegistryConfig>,
    check_cache: StdRwLock<HashMap<ToolName, CachedCheck>>,
}

struct RegistryInner {
    /// Tool instances keyed by name.
    tools: HashMap<ToolName, Arc<dyn Tool>>,
    /// Tool definitions keyed by name.
    definitions: HashMap<ToolName, ToolDefinition>,
    /// Compact index for LLM context injection.
    index: ToolIndex,
    /// Optional availability check functions keyed by tool name.
    /// When present, the tool is only exposed to the LLM if the check
    /// returns `true`. See [`ToolRegistryImpl::set_check_fn`].
    check_fns: HashMap<ToolName, ToolCheckFn>,
}
impl ToolRegistryImpl {
    pub fn new(config: ToolRegistryConfig) -> Self {
        Self {
            inner: RwLock::new(RegistryInner {
                tools: HashMap::new(),
                definitions: HashMap::new(),
                index: ToolIndex::new(),
                check_fns: HashMap::new(),
            }),
            config: StdRwLock::new(config),
            check_cache: StdRwLock::new(HashMap::new()),
        }
    }

    /// Hot-reload the tool registry configuration.
    pub fn reload_config(&self, new_config: ToolRegistryConfig) {
        *self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new_config;
        tracing::info!("Tool registry config hot-reloaded");
    }

    /// Get a read-only snapshot of the current configuration.
    pub fn config(&self) -> ToolRegistryConfig {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
        inner
            .definitions
            .insert(definition.name.clone(), definition);
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

    /// Remove a tool from the registry by name.
    pub async fn unregister_tool(&self, name: &ToolName) -> bool {
        let mut inner = self.inner.write().await;
        let had = inner.definitions.remove(name).is_some();
        inner.tools.remove(name);
        inner.index.remove(name);
        inner.check_fns.remove(name);
        if let Ok(mut cache) = self.check_cache.write() {
            cache.remove(name);
        }
        had
    }

    /// Register an availability check function for a tool.
    ///
    /// When `check_fn` returns `false`, the tool's schema is omitted from
    /// [`get_all_definitions`](Self::get_all_definitions), preventing it from
    /// being sent to the LLM. This saves context tokens for tools whose
    /// runtime requirements are not met (e.g., `KnowledgeSearch` when no
    /// vector backend is configured).
    ///
    /// Results are cached for 30 seconds. If the check panics, the tool is
    /// treated as available (fail-open) for a 60-second grace period.
    pub async fn set_check_fn(&self, name: &ToolName, check_fn: ToolCheckFn) {
        let mut inner = self.inner.write().await;
        inner.check_fns.insert(name.clone(), check_fn);
        // Invalidate cache for this tool so the next get_all_definitions
        // call re-evaluates.
        if let Ok(mut cache) = self.check_cache.write() {
            cache.remove(name);
        }
        tracing::debug!(tool = %name, "check_fn registered for tool");
    }

    /// Check if a tool is currently available according to its `check_fn`.
    ///
    /// Returns `true` if:
    /// - The tool has no `check_fn` (always available).
    /// - The cached result is still fresh (within TTL).
    /// - The `check_fn` returns `true`.
    ///
    /// Returns `false` only when the `check_fn` returns `false` and the
    /// result is fresh. On panic, returns `true` (fail-open).
    fn check_tool_available(&self, name: &ToolName, check_fn: &ToolCheckFn) -> bool {
        // Check cache first. Use the appropriate TTL: normal results use
        // CHECK_TTL (30s); panic-fail-open results use CHECK_GRACE (60s).
        if let Ok(cache) = self.check_cache.read() {
            if let Some(cached) = cache.get(name) {
                let ttl = if cached.panicked {
                    CHECK_GRACE
                } else {
                    CHECK_TTL
                };
                if cached.checked_at.elapsed() < ttl {
                    return cached.result;
                }
            }
        }

        // Evaluate check_fn, catching panics (fail-open).
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check_fn()));
        let panicked = panic_result.is_err();
        // On panic, treat as available (fail-open) so a flaky check doesn't
        // hide a tool that might actually work.
        let result = panic_result.unwrap_or(true);

        // Cache the result.
        if let Ok(mut cache) = self.check_cache.write() {
            cache.insert(
                name.clone(),
                CachedCheck {
                    result,
                    panicked,
                    checked_at: std::time::Instant::now(),
                },
            );
        }

        result
    }

    /// Search for tools by keywords in name/description and optional category.
    ///
    /// The query is split on whitespace, commas, or semicolons into individual
    /// keywords. A tool matches if its name or description contains at least
    /// one keyword (OR semantics).
    pub async fn search_tools(
        &self,
        query: &str,
        category: Option<&ToolCategory>,
    ) -> Vec<ToolDefinition> {
        let inner = self.inner.read().await;
        let keywords: Vec<String> = query
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let limit = self
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .search_limit;

        if keywords.is_empty() {
            return Vec::new();
        }

        let text_matches = |text: &str| -> bool {
            let lower = text.to_lowercase();
            keywords.iter().any(|kw| lower.contains(kw.as_str()))
        };

        inner
            .index
            .entries()
            .iter()
            .filter(|entry| {
                let name_match = text_matches(entry.name.as_str());
                let desc_match = text_matches(&entry.description);
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

    /// Get all registered tool definitions, filtered by `check_fn`.
    ///
    /// Tools with a `check_fn` that returns `false` are excluded — their
    /// schema is not sent to the LLM, saving context tokens. Results are
    /// cached for 30s (see `check_tool_available`).
    pub async fn get_all_definitions(&self) -> Vec<ToolDefinition> {
        let inner = self.inner.read().await;
        inner
            .definitions
            .values()
            .filter(|def| {
                // No check_fn = always available.
                let Some(check_fn) = inner.check_fns.get(&def.name) else {
                    return true;
                };
                self.check_tool_available(&def.name, check_fn)
            })
            .cloned()
            .collect()
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
        inner
            .definitions
            .insert(definition.name.clone(), definition);
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
            help: None,
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
        let (tool, def) = make_tool("FileRead");
        reg.register_tool(tool, def).await.unwrap();
        assert!(reg
            .get_tool(&ToolName::from_string("FileRead"))
            .await
            .is_some());
        assert_eq!(reg.len().await, 1);
    }

    #[tokio::test]
    async fn test_registry_duplicate_name_rejected() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool1, def1) = make_tool("FileRead");
        let (tool2, def2) = make_tool("FileRead");
        reg.register_tool(tool1, def1).await.unwrap();
        let err = reg.register_tool(tool2, def2).await.unwrap_err();
        assert!(matches!(err, ToolRegistryError::DuplicateName { .. }));
    }

    #[tokio::test]
    async fn test_registry_unregister() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("FileRead");
        reg.register_tool(tool, def).await.unwrap();
        ToolRegistry::unregister(&reg, &ToolName::from_string("FileRead"))
            .await
            .unwrap();
        assert!(reg
            .get_tool(&ToolName::from_string("FileRead"))
            .await
            .is_none());
        assert_eq!(reg.len().await, 0);
    }

    #[tokio::test]
    async fn test_registry_search_by_keyword() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        for name in &["FileRead", "FileWrite", "WebSearch", "code_exec"] {
            let (tool, def) = make_tool(name);
            reg.register_tool(tool, def).await.unwrap();
        }
        let results = reg.search_tools("file", None).await;
        assert_eq!(results.len(), 2);
        let names: Vec<String> = results
            .iter()
            .map(|r| r.name.as_str().to_string())
            .collect();
        assert!(names.contains(&"FileRead".to_string()));
        assert!(names.contains(&"FileWrite".to_string()));
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
        let (tool, def) = make_tool("FileRead");
        reg.register_tool(tool, def).await.unwrap();

        let index = ToolRegistry::tool_index(&reg).await;
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].name.as_str(), "FileRead");
    }

    // --- check_fn tests ---

    #[tokio::test]
    async fn test_check_fn_filters_tool_from_definitions() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool1, def1) = make_tool("AlwaysAvailable");
        let (tool2, def2) = make_tool("GatedTool");
        reg.register_tool(tool1, def1).await.unwrap();
        reg.register_tool(tool2, def2).await.unwrap();

        // Set check_fn that returns false.
        reg.set_check_fn(&ToolName::from_string("GatedTool"), Arc::new(|| false))
            .await;

        let defs = reg.get_all_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name.as_str(), "AlwaysAvailable");
    }

    #[tokio::test]
    async fn test_check_fn_pass_includes_tool() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("GatedTool");
        reg.register_tool(tool, def).await.unwrap();

        reg.set_check_fn(&ToolName::from_string("GatedTool"), Arc::new(|| true))
            .await;

        let defs = reg.get_all_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name.as_str(), "GatedTool");
    }

    #[tokio::test]
    async fn test_no_check_fn_means_always_available() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("PlainTool");
        reg.register_tool(tool, def).await.unwrap();

        let defs = reg.get_all_definitions().await;
        assert_eq!(defs.len(), 1);
    }

    #[tokio::test]
    async fn test_check_fn_caches_result() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("CountedTool");
        reg.register_tool(tool, def).await.unwrap();

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);
        reg.set_check_fn(
            &ToolName::from_string("CountedTool"),
            Arc::new(move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                true
            }),
        )
        .await;

        // First call evaluates check_fn.
        let _ = reg.get_all_definitions().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second call uses cache (no new evaluation).
        let _ = reg.get_all_definitions().await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_check_fn_panic_fails_open() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("PanicTool");
        reg.register_tool(tool, def).await.unwrap();

        reg.set_check_fn(
            &ToolName::from_string("PanicTool"),
            Arc::new(|| panic!("`check_fn` panicked")),
        )
        .await;

        // On panic, tool should still be available (fail-open).
        let defs = reg.get_all_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name.as_str(), "PanicTool");
    }

    #[tokio::test]
    async fn test_unregister_removes_check_fn() {
        let reg = ToolRegistryImpl::new(ToolRegistryConfig::default());
        let (tool, def) = make_tool("GatedTool");
        reg.register_tool(tool, def).await.unwrap();
        reg.set_check_fn(&ToolName::from_string("GatedTool"), Arc::new(|| false))
            .await;

        // Tool is filtered out.
        assert_eq!(reg.get_all_definitions().await.len(), 0);

        // Unregister and re-register without check_fn.
        reg.unregister_tool(&ToolName::from_string("GatedTool"))
            .await;
        let (tool2, def2) = make_tool("GatedTool");
        reg.register_tool(tool2, def2).await.unwrap();

        // Now it should be available (no check_fn).
        assert_eq!(reg.get_all_definitions().await.len(), 1);
    }
}
