//! Tool search orchestrator — performs actual taxonomy/registry lookups
//! and tool activation when `tool_search` is called.
//!
//! The `ToolSearchOrchestrator` is the bridge between the `tool_search` meta-tool
//! and the real `ToolRegistryImpl` + `ToolTaxonomy` + `ToolActivationSet`.
//! It intercepts `tool_search` calls from the `ChatService` tool loop and
//! returns real results instead of `{"status": "pending"}`.

use std::sync::Arc;

use tokio::sync::RwLock;

use y_core::tool::{ToolDefinition, ToolOutput};
use y_core::types::ToolName;
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

/// Orchestrates tool search actions: category browsing, tool schema retrieval,
/// and keyword search — with automatic activation of discovered tools.
pub struct ToolSearchOrchestrator;

impl ToolSearchOrchestrator {
    /// Handle a `tool_search` call by performing actual lookups.
    ///
    /// Examines the `arguments` JSON and dispatches to the appropriate mode:
    /// - `tool` → get full definition + activate
    /// - `category` → browse taxonomy category
    /// - `query` → keyword search + activate top results
    ///
    /// Returns a `ToolOutput` containing real search results.
    pub async fn handle(
        arguments: &serde_json::Value,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let tool_name = arguments.get("tool").and_then(|v| v.as_str());
        let category = arguments.get("category").and_then(|v| v.as_str());
        let query = arguments.get("query").and_then(|v| v.as_str());

        // At least one parameter must be present.
        if tool_name.is_none() && category.is_none() && query.is_none() {
            return Err(y_core::tool::ToolError::ValidationError {
                message: "at least one of 'tool', 'category', or 'query' must be provided".into(),
            });
        }

        // Precedence: tool > category > query.
        if let Some(name) = tool_name {
            Self::handle_get_tool(name, registry, activation_set).await
        } else if let Some(cat) = category {
            Self::handle_browse_category(cat, taxonomy).await
        } else {
            Self::handle_search(query.unwrap(), registry, taxonomy, activation_set).await
        }
    }

    /// Get a specific tool's full definition and activate it.
    async fn handle_get_tool(
        name: &str,
        registry: &ToolRegistryImpl,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let tool_name = ToolName::from_string(name);

        let definition = registry
            .get_definition(&tool_name)
            .await
            .ok_or_else(|| y_core::tool::ToolError::NotFound {
                name: name.to_string(),
            })?;

        // Activate the tool.
        {
            let mut set = activation_set.write().await;
            set.activate(definition.clone());
        }

        Ok(ToolOutput {
            success: true,
            content: Self::definition_to_json(&definition),
            warnings: vec![],
            metadata: serde_json::json!({"action": "get_tool", "activated": true}),
        })
    }

    /// Browse a taxonomy category, returning subcategories and tool lists.
    async fn handle_browse_category(
        category: &str,
        taxonomy: &ToolTaxonomy,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let detail = taxonomy
            .category_detail(category)
            .ok_or_else(|| y_core::tool::ToolError::NotFound {
                name: format!("category: {category}"),
            })?;

        let tools = taxonomy.tools_in_category(category);

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "category": category,
                "detail": detail,
                "tools": tools,
            }),
            warnings: vec![],
            metadata: serde_json::json!({"action": "browse_category"}),
        })
    }

    /// Keyword search across registry and taxonomy, with activation.
    async fn handle_search(
        query: &str,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        // Search both registry and taxonomy.
        let registry_results = registry.search_tools(query, None).await;
        let taxonomy_hits = taxonomy.search(query);

        // Merge: registry results first, then any taxonomy-only names.
        let mut result_defs: Vec<ToolDefinition> = registry_results;
        for tool_name_str in &taxonomy_hits {
            let tn = ToolName::from_string(tool_name_str);
            if !result_defs.iter().any(|d| d.name == tn) {
                if let Some(def) = registry.get_definition(&tn).await {
                    result_defs.push(def);
                }
            }
        }

        // Activate all found tools.
        let activated_names: Vec<String> = result_defs
            .iter()
            .map(|d| d.name.as_str().to_string())
            .collect();

        {
            let mut set = activation_set.write().await;
            for def in &result_defs {
                set.activate(def.clone());
            }
        }

        let results_json: Vec<serde_json::Value> = result_defs
            .iter()
            .map(Self::definition_to_json)
            .collect();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "query": query,
                "results": results_json,
                "count": results_json.len(),
                "activated": activated_names,
            }),
            warnings: vec![],
            metadata: serde_json::json!({"action": "search"}),
        })
    }

    /// Convert a `ToolDefinition` to its JSON representation for LLM consumption.
    fn definition_to_json(def: &ToolDefinition) -> serde_json::Value {
        serde_json::json!({
            "name": def.name.as_str(),
            "description": def.description,
            "parameters": def.parameters,
            "category": format!("{:?}", def.category),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::runtime::RuntimeCapability;
    use y_core::tool::{ToolCategory, ToolType};

    const TEST_TOML: &str = r#"
[categories.file]
label = "File Management"
description = "Read, write, and manage files"

[categories.file.subcategories.read]
label = "File Reading"
description = "Read file contents"
tools = ["file_read", "file_list"]

[categories.file.subcategories.write]
label = "File Writing"
description = "Create or modify files"
tools = ["file_write"]

[categories.shell]
label = "Shell"
description = "Execute shell commands"
tools = ["shell_exec"]

[categories.meta]
label = "Meta Tools"
description = "Tool management tools"
tools = ["tool_search"]
"#;

    fn sample_def(name: &str, desc: &str) -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string(name),
            description: desc.into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    async fn setup() -> (
        ToolRegistryImpl,
        ToolTaxonomy,
        Arc<RwLock<ToolActivationSet>>,
    ) {
        let registry = ToolRegistryImpl::new(y_tools::ToolRegistryConfig::default());

        // Register sample tools via the trait.
        for (name, desc) in &[
            ("file_read", "Read file contents"),
            ("file_list", "List directory contents"),
            ("file_write", "Write file contents"),
            ("shell_exec", "Execute shell commands"),
            ("tool_search", "Search for tools"),
        ] {
            let def = sample_def(name, desc);
            use y_core::tool::ToolRegistry;
            registry.register(def).await.unwrap();
        }

        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let activation_set = Arc::new(RwLock::new(ToolActivationSet::new(20)));

        (registry, taxonomy, activation_set)
    }

    #[tokio::test]
    async fn test_get_tool_returns_definition_and_activates() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"tool": "file_read"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
                .await
                .unwrap();

        assert!(result.success);
        assert_eq!(result.content["name"], "file_read");
        assert!(result.content["parameters"].is_object());

        // Verify activation.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("file_read")));
    }

    #[tokio::test]
    async fn test_get_tool_not_found() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"tool": "nonexistent_tool"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_browse_category_returns_detail() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"category": "file"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
                .await
                .unwrap();

        assert!(result.success);
        assert_eq!(result.content["category"], "file");
        let tools = result.content["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t == "file_read"));
        assert!(tools.iter().any(|t| t == "file_write"));
    }

    #[tokio::test]
    async fn test_browse_category_not_found() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"category": "nonexistent"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_keyword_search_activates_matches() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"query": "file"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
                .await
                .unwrap();

        assert!(result.success);
        let activated = result.content["activated"].as_array().unwrap();
        assert!(!activated.is_empty());
        assert!(activated.iter().any(|v| v == "file_read"));

        // Verify activation set.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("file_read")));
    }

    #[tokio::test]
    async fn test_no_params_returns_error() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_precedence_tool_over_category() {
        let (registry, taxonomy, activation_set) = setup().await;

        // Both `tool` and `category` provided — `tool` should take precedence.
        let args = serde_json::json!({"tool": "shell_exec", "category": "file"});
        let result =
            ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
                .await
                .unwrap();

        assert_eq!(result.content["name"], "shell_exec");
    }
}
