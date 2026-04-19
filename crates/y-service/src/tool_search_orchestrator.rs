//! Tool search orchestrator — performs actual taxonomy/registry lookups
//! and tool activation when `ToolSearch` is called.
//!
//! The `ToolSearchOrchestrator` is the bridge between the `ToolSearch` meta-tool
//! and the real `ToolRegistryImpl` + `ToolTaxonomy` + `ToolActivationSet`.
//! It intercepts `ToolSearch` calls from the `ChatService` tool loop and
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
    /// Handle a `ToolSearch` call by performing actual lookups.
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

    /// Handle a `ToolSearch` call with optional skill/agent sources.
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
        let mcp_server = arguments.get("mcp_server").and_then(|v| v.as_str());

        // At least one parameter must be present.
        if tool_name.is_none() && category.is_none() && query.is_none() && mcp_server.is_none() {
            return Err(y_core::tool::ToolError::ValidationError {
                message: "at least one of 'tool', 'category', 'query', or 'mcp_server' must be provided".into(),
            });
        }

        // Precedence: tool > mcp_server > category > query.
        if let Some(name) = tool_name {
            Self::handle_get_tool(name, registry, activation_set).await
        } else if let Some(server) = mcp_server {
            Self::handle_search_mcp_server(server, registry, activation_set).await
        } else if let Some(cat) = category {
            Self::handle_browse_category(cat, taxonomy, registry, activation_set).await
        } else if let Some(q) = query {
            Self::handle_search(q, registry, taxonomy, activation_set, sources).await
        } else {
            Err(y_core::tool::ToolError::ValidationError {
                message: "at least one of 'tool', 'category', 'query', or 'mcp_server' must be provided".into(),
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

    /// Search for all tools from a specific MCP server and activate them.
    async fn handle_search_mcp_server(
        server_name: &str,
        registry: &ToolRegistryImpl,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let prefix = format!("mcp_{server_name}_");
        let all_defs = registry.get_all_definitions().await;
        let mcp_defs: Vec<&ToolDefinition> = all_defs
            .iter()
            .filter(|d| d.name.as_str().starts_with(&prefix))
            .collect();

        if mcp_defs.is_empty() {
            return Ok(ToolOutput {
                success: true,
                content: serde_json::json!({
                    "mcp_server": server_name,
                    "tools": [],
                    "count": 0,
                    "message": format!(
                        "No tools found for MCP server '{server_name}'. \
                         Check that the server is configured and connected."
                    ),
                }),
                warnings: vec![],
                metadata: serde_json::json!({"action": "search_mcp_server"}),
            });
        }

        let mut activated_names = Vec::new();
        let mut tool_defs = Vec::new();
        {
            let mut set = activation_set.write().await;
            for def in &mcp_defs {
                set.activate((*def).clone());
                activated_names.push(def.name.as_str().to_string());
                tool_defs.push(Self::summary_to_json(def));
            }
        }

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "mcp_server": server_name,
                "tools": tool_defs,
                "count": tool_defs.len(),
                "activated": activated_names,
            }),
            warnings: vec![],
            metadata: serde_json::json!({"action": "search_mcp_server"}),
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
                        "usage": "Use the 'task' tool to delegate work to this agent: \
                            task({\"agent_name\": \"<id>\", \"prompt\": \"<your_task>\"})",
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
                "note": "Items under 'tools' can be called directly. \
                    To delegate to an agent, use the 'task' tool with the agent's id.",
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
tools = ["FileRead"]

[categories.file.subcategories.write]
label = "File Writing"
description = "Create or modify files"
tools = ["FileWrite"]

[categories.shell]
label = "Shell"
description = "Execute shell commands"
tools = ["ShellExec"]

[categories.meta]
label = "Meta Tools"
description = "Tool management tools"
tools = ["ToolSearch"]
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
            ("FileRead", "Read file contents"),
            ("FileWrite", "Write file contents"),
            ("ShellExec", "Execute shell commands"),
            ("ToolSearch", "Search for tools"),
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

        let args = serde_json::json!({"tool": "FileRead"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["name"], "FileRead");
        assert!(result.content["parameters"].is_object());

        // Verify activation.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("FileRead")));
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
        assert!(tools.iter().any(|t| t == "FileRead"));
        assert!(tools.iter().any(|t| t == "FileWrite"));

        // Verify tool definitions are returned.
        let defs = result.content["tool_definitions"].as_array().unwrap();
        assert!(!defs.is_empty());
        assert!(defs.iter().any(|d| d["name"] == "FileRead"));

        // Verify activated list is returned.
        let activated = result.content["activated"].as_array().unwrap();
        assert!(activated.iter().any(|a| a == "FileRead"));

        // Verify tools are activated in the activation set.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("FileRead")));
        assert!(set.contains(&ToolName::from_string("FileWrite")));
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
        assert!(activated.iter().any(|v| v == "FileRead"));

        // Verify activation set.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("FileRead")));
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
        let args = serde_json::json!({"tool": "ShellExec", "category": "file"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert_eq!(result.content["name"], "ShellExec");
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

    #[tokio::test]
    async fn test_search_mcp_server_returns_matching_tools() {
        let (registry, taxonomy, activation_set) = setup().await;

        // Register MCP tools.
        use y_core::tool::ToolRegistry;
        for (name, desc) in &[
            ("mcp_github_search_repos", "Search GitHub repositories"),
            ("mcp_github_list_issues", "List GitHub issues"),
            ("mcp_filesystem_read_file", "Read file via MCP filesystem"),
        ] {
            let def = sample_def(name, desc);
            registry.register(def).await.unwrap();
        }

        let args = serde_json::json!({"mcp_server": "github"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["mcp_server"], "github");
        assert_eq!(result.content["count"], 2);

        let tools = result.content["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"mcp_github_search_repos"));
        assert!(names.contains(&"mcp_github_list_issues"));
        // filesystem tool should NOT be included.
        assert!(!names.contains(&"mcp_filesystem_read_file"));

        // Verify activation.
        let set = activation_set.read().await;
        assert!(set.contains(&ToolName::from_string("mcp_github_search_repos")));
        assert!(set.contains(&ToolName::from_string("mcp_github_list_issues")));
    }

    #[tokio::test]
    async fn test_search_mcp_server_no_match() {
        let (registry, taxonomy, activation_set) = setup().await;

        let args = serde_json::json!({"mcp_server": "nonexistent"});
        let result = ToolSearchOrchestrator::handle(&args, &registry, &taxonomy, &activation_set)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["count"], 0);
        assert!(result.content["message"].as_str().unwrap().contains("nonexistent"));
    }
}
