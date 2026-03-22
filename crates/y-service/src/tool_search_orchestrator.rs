//! Tool search orchestrator — performs actual taxonomy/registry lookups
//! and tool activation when `tool_search` is called.
//!
//! The `ToolSearchOrchestrator` is the bridge between the `tool_search` meta-tool
//! and the real `ToolRegistryImpl` + `ToolTaxonomy` + `ToolActivationSet`.
//! It intercepts `tool_search` calls from the `ChatService` tool loop and
//! returns real results instead of `{"status": "pending"}`.
//!
//! Since v0.4, keyword search also queries `SkillSearch` and `AgentRegistry`
//! so that skills and agents are discoverable through the same meta-tool.

use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use y_agent::AgentRegistry;
use y_core::tool::{ToolDefinition, ToolOutput};
use y_core::types::ToolName;
use y_skills::SkillSearch;
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

/// Optional extra capability sources for unified search.
///
/// When provided, keyword search (`query`) also returns matching skills
/// and agents alongside tools. Category browsing and direct tool lookup
/// remain tool-only.
pub struct CapabilitySearchSources<'a> {
    /// Skill search index (loaded from the filesystem skill store).
    pub skill_search: Option<&'a RwLock<SkillSearch>>,
    /// Agent registry for agent definition search.
    pub agent_registry: Option<&'a Mutex<AgentRegistry>>,
}

impl CapabilitySearchSources<'_> {
    /// No extra sources (backward-compatible default).
    pub fn none() -> Self {
        Self {
            skill_search: None,
            agent_registry: None,
        }
    }
}

/// Orchestrates tool search actions: category browsing, tool schema retrieval,
/// and keyword search — with automatic activation of discovered tools.
///
/// Keyword search also queries optional skill and agent registries to
/// provide unified capability discovery.
pub struct ToolSearchOrchestrator;

impl ToolSearchOrchestrator {
    /// Handle a `tool_search` call by performing actual lookups.
    ///
    /// Examines the `arguments` JSON and dispatches to the appropriate mode:
    /// - `tool` -> get full definition + activate
    /// - `category` -> browse taxonomy category
    /// - `query` -> keyword search across tools, skills, and agents + activate tool results
    ///
    /// Returns a `ToolOutput` containing real search results.
    pub async fn handle(
        arguments: &serde_json::Value,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        Self::handle_with_sources(
            arguments,
            registry,
            taxonomy,
            activation_set,
            &CapabilitySearchSources::none(),
        )
        .await
    }

    /// Handle a `tool_search` call with optional skill/agent sources.
    ///
    /// This is the full-featured entry point. Keyword search queries all
    /// three registries (tools, skills, agents) when sources are provided.
    pub async fn handle_with_sources(
        arguments: &serde_json::Value,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
        sources: &CapabilitySearchSources<'_>,
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
            Self::handle_browse_category(cat, taxonomy, registry, activation_set).await
        } else if let Some(q) = query {
            Self::handle_search(q, registry, taxonomy, activation_set, sources).await
        } else {
            // Unreachable: the is_none() guard above ensures at least one param is present.
            Err(y_core::tool::ToolError::ValidationError {
                message: "at least one of 'tool', 'category', or 'query' must be provided".into(),
            })
        }
    }

    /// Get a specific tool's full definition and activate it.
    async fn handle_get_tool(
        name: &str,
        registry: &ToolRegistryImpl,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let tool_name = ToolName::from_string(name);

        let definition = registry.get_definition(&tool_name).await.ok_or_else(|| {
            y_core::tool::ToolError::NotFound {
                name: name.to_string(),
            }
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

    /// Browse a taxonomy category, returning subcategories, tool lists,
    /// and full tool definitions (with auto-activation).
    async fn handle_browse_category(
        category: &str,
        taxonomy: &ToolTaxonomy,
        registry: &ToolRegistryImpl,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let detail = taxonomy.category_detail(category).ok_or_else(|| {
            y_core::tool::ToolError::NotFound {
                name: format!("category: {category}"),
            }
        })?;

        let tools = taxonomy.tools_in_category(category);

        // Look up full definitions for tools in the category and activate them.
        let mut tool_defs = Vec::new();
        let mut activated_names = Vec::new();
        {
            let mut set = activation_set.write().await;
            for tool_name_str in &tools {
                let tn = ToolName::from_string(tool_name_str);
                if let Some(def) = registry.get_definition(&tn).await {
                    set.activate(def.clone());
                    activated_names.push(tool_name_str.clone());
                    tool_defs.push(Self::summary_to_json(&def));
                }
            }
        }

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "category": category,
                "detail": detail,
                "tools": tools,
                "tool_definitions": tool_defs,
                "activated": activated_names,
            }),
            warnings: vec![],
            metadata: serde_json::json!({"action": "browse_category"}),
        })
    }

    /// Keyword search across tools, skills, and agents, with tool activation.
    async fn handle_search(
        query: &str,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
        sources: &CapabilitySearchSources<'_>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        // --- 1. Search tools (registry + taxonomy) ---
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

        let tools_json: Vec<serde_json::Value> =
            result_defs.iter().map(Self::summary_to_json).collect();

        // --- 2. Search skills ---
        let skills_json = if let Some(skill_search) = sources.skill_search {
            let ss = skill_search.read().await;
            let skill_results = ss.search(query, 10);
            skill_results
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "type": "skill",
                        "name": s.name,
                        "description": s.description,
                        "tags": s.tags,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        // --- 3. Search agents ---
        let agents_json = if let Some(agent_registry) = sources.agent_registry {
            let ar = agent_registry.lock().await;
            let agent_results = ar.search(query);
            agent_results
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "type": "agent",
                        "id": a.id,
                        "name": a.name,
                        "description": a.description,
                        "mode": format!("{:?}", a.mode),
                        "capabilities": a.capabilities,
                        "usage": "Agents are internal sub-agents managed by \
                            the system -- do NOT call them as tools.",
                    })
                })
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        // --- 4. Build unified response ---
        let total_count = tools_json.len() + skills_json.len() + agents_json.len();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "query": query,
                "note": "Only items under 'tools' can be called as tool_call. \
                    Skills and agents are NOT callable tools.",
                "tools": {
                    "results": tools_json,
                    "count": tools_json.len(),
                    "activated": activated_names,
                },
                "skills": skills_json,
                "agents": agents_json,
                "total_count": total_count,
            }),
            warnings: vec![],
            metadata: serde_json::json!({"action": "search"}),
        })
    }

    /// Compact summary for search/browse results (no parameters, no help).
    fn summary_to_json(def: &ToolDefinition) -> serde_json::Value {
        serde_json::json!({
            "name": def.name.as_str(),
            "description": def.description,
            "category": format!("{:?}", def.category),
        })
    }

    /// Full tool representation for direct lookup (includes parameters and help).
    fn definition_to_json(def: &ToolDefinition) -> serde_json::Value {
        let mut json = serde_json::json!({
            "name": def.name.as_str(),
            "description": def.description,
            "parameters": def.parameters,
            "category": format!("{:?}", def.category),
        });
        if let Some(ref help) = def.help {
            json["help"] = serde_json::json!(help);
        }
        json
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
tools = ["file_read"]

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
            help: None,
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
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
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
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["category"], "file");
        let tools = result.content["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t == "file_read"));
        assert!(tools.iter().any(|t| t == "file_write"));

        // Verify tool definitions are returned.
        let defs = result.content["tool_definitions"].as_array().unwrap();
        assert!(!defs.is_empty());
        assert!(defs.iter().any(|d| d["name"] == "file_read"));

        // Verify activated list is returned.
        let activated = result.content["activated"].as_array().unwrap();
        assert!(activated.iter().any(|a| a == "file_read"));

        // Verify tools are activated in the activation set.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("file_read")));
        assert!(set.contains(&ToolName::from_string("file_write")));
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
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert!(result.success);
        let tools = &result.content["tools"];
        let activated = tools["activated"].as_array().unwrap();
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

        // Both `tool` and `category` provided -- `tool` should take precedence.
        let args = serde_json::json!({"tool": "shell_exec", "category": "file"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert_eq!(result.content["name"], "shell_exec");
    }

    #[tokio::test]
    async fn test_keyword_search_includes_skills_and_agents() {
        let (registry, taxonomy, activation_set) = setup().await;

        // Set up a SkillSearch with a test skill.
        let skill_search = RwLock::new(SkillSearch::new());
        {
            let mut ss = skill_search.write().await;
            let now = y_core::types::now();
            ss.index(y_core::skill::SkillManifest {
                id: y_core::types::SkillId::from_string("skill-1"),
                name: "code-review".to_string(),
                description: "Reviews code for file consistency".to_string(),
                version: y_core::skill::SkillVersion("v1".to_string()),
                tags: vec!["file".to_string(), "review".to_string()],
                trigger_patterns: vec![],
                knowledge_bases: vec![],
                root_content: String::new(),
                sub_documents: vec![],
                token_estimate: 10,
                created_at: now,
                updated_at: now,
                classification: None,
                constraints: None,
                security: None,
                references: None,
                author: None,
                source_format: None,
                source_hash: None,
                state: None,
                root_path: None,
            });
        }

        // Set up an AgentRegistry with a matching agent.
        let agent_registry = Mutex::new(y_agent::AgentRegistry::new());

        let sources = CapabilitySearchSources {
            skill_search: Some(&skill_search),
            agent_registry: Some(&agent_registry),
        };

        let args = serde_json::json!({"query": "file"});
        let result = ToolSearchOrchestrator::handle_with_sources(
            &args,
            &registry,
            &taxonomy,
            &activation_set,
            &sources,
        )
        .await
        .unwrap();

        assert!(result.success);

        // Tools should be present.
        let tools = &result.content["tools"];
        assert!(tools["count"].as_u64().unwrap() > 0);

        // Skills matching "file" should appear.
        let skills = result.content["skills"].as_array().unwrap();
        assert!(!skills.is_empty());
        assert!(skills.iter().any(|s| s["name"] == "code-review"));

        // total_count should include tools + skills + agents.
        let total = result.content["total_count"].as_u64().unwrap();
        assert!(total >= tools["count"].as_u64().unwrap() + skills.len() as u64);
    }

    #[tokio::test]
    async fn test_keyword_search_no_sources_returns_empty_skills_agents() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"query": "file"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        // Without sources, skills and agents arrays should be empty.
        let skills = result.content["skills"].as_array().unwrap();
        assert!(skills.is_empty());
        let agents = result.content["agents"].as_array().unwrap();
        assert!(agents.is_empty());
    }
}
