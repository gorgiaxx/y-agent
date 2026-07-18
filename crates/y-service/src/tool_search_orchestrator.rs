//! Tool search orchestrator — performs actual taxonomy/registry lookups
//! and tool activation when `ToolSearch` is called.
//!
//! The `ToolSearchOrchestrator` is the bridge between the `ToolSearch` meta-tool
//! and the real `ToolRegistryImpl` + `ToolTaxonomy` + `ToolActivationSet`.
//! It intercepts `ToolSearch` calls from the `ChatService` tool loop and
//! returns real results instead of `{"status": "pending"}`.
//!
//! Since v0.4, keyword search also queries `SkillSearch` and `AgentRegistry`
//! so that skills, agents, and workflows are discoverable through the same meta-tool.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use y_agent::AgentRegistry;
use y_core::tool::{ToolDefinition, ToolOutput};
use y_core::types::ToolName;
use y_skills::SkillSearch;
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

use crate::capability_search::{CapabilityDocument, CapabilityKind, CapabilitySearchIndex};

/// Optional extra capability sources for unified search.
///
/// Compact reusable workflow descriptor for unified capability search.
#[derive(Debug, Clone)]
pub struct WorkflowSearchItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub parameter_names: Vec<String>,
}

/// When provided, keyword search (`query`) also returns matching skills,
/// agents, and workflows alongside tools. Category browsing and direct tool lookup
/// remain tool-only.
pub struct CapabilitySearchSources<'a> {
    /// Skill search index (loaded from the filesystem skill store).
    pub skill_search: Option<&'a RwLock<SkillSearch>>,
    /// Agent registry for agent definition search.
    pub agent_registry: Option<&'a Mutex<AgentRegistry>>,
    /// Durable reusable workflow descriptors.
    pub workflows: Option<&'a [WorkflowSearchItem]>,
}

impl CapabilitySearchSources<'_> {
    /// No extra sources (backward-compatible default).
    pub fn none() -> Self {
        Self {
            skill_search: None,
            agent_registry: None,
            workflows: None,
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
    /// - `query` -> keyword search across tools, skills, agents, and workflows
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
    /// all capability registries when sources are provided.
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
                message:
                    "at least one of 'tool', 'category', 'query', or 'mcp_server' must be provided"
                        .into(),
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
                message:
                    "at least one of 'tool', 'category', 'query', or 'mcp_server' must be provided"
                        .into(),
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

    /// Keyword search across tools, skills, agents, and workflows, with tool activation.
    async fn handle_search(
        query: &str,
        registry: &ToolRegistryImpl,
        taxonomy: &ToolTaxonomy,
        activation_set: &Arc<RwLock<ToolActivationSet>>,
        sources: &CapabilitySearchSources<'_>,
    ) -> Result<ToolOutput, y_core::tool::ToolError> {
        let tool_definitions = registry.get_all_definitions().await;
        let taxonomy_hits: HashSet<String> = taxonomy.search(query).into_iter().collect();
        let skill_documents = if let Some(skill_search) = sources.skill_search {
            skill_search.read().await.documents()
        } else {
            Vec::new()
        };
        let agent_definitions = if let Some(agent_registry) = sources.agent_registry {
            agent_registry
                .lock()
                .await
                .list()
                .into_iter()
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let workflows = sources.workflows.unwrap_or_default();

        let mut documents = Vec::with_capacity(
            tool_definitions.len()
                + skill_documents.len()
                + agent_definitions.len()
                + workflows.len(),
        );
        documents.extend(tool_definitions.iter().map(|definition| {
            let mut keywords = schema_parameter_names(&definition.parameters);
            keywords.push(format!("{:?}", definition.category));
            keywords.push(format!("{:?}", definition.tool_type));
            if let Some(help) = &definition.help {
                keywords.push(help.clone());
            }
            if let Ok(capabilities) = serde_json::to_string(&definition.capabilities) {
                keywords.push(capabilities);
            }
            if taxonomy_hits.contains(definition.name.as_str()) {
                keywords.push(query.to_string());
            }
            CapabilityDocument {
                kind: CapabilityKind::Tool,
                id: definition.name.as_str().to_string(),
                name: definition.name.as_str().to_string(),
                description: definition.description.clone(),
                aliases: tool_aliases(definition.name.as_str()),
                keywords,
            }
        }));
        documents.extend(skill_documents.iter().map(|skill| {
            CapabilityDocument {
                kind: CapabilityKind::Skill,
                id: skill.id.to_string(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                aliases: Vec::new(),
                keywords: skill
                    .tags
                    .iter()
                    .chain(skill.trigger_patterns.iter())
                    .cloned()
                    .collect(),
            }
        }));
        documents.extend(agent_definitions.iter().map(|agent| {
            CapabilityDocument {
                kind: CapabilityKind::Agent,
                id: agent.id.clone(),
                name: agent.name.clone(),
                description: agent.description.clone(),
                aliases: Vec::new(),
                keywords: agent
                    .capabilities
                    .iter()
                    .cloned()
                    .chain([
                        format!("{:?}", agent.mode),
                        format!("{:?}", agent.trust_tier),
                    ])
                    .collect(),
            }
        }));
        documents.extend(workflows.iter().map(|workflow| {
            CapabilityDocument {
                kind: CapabilityKind::Workflow,
                id: workflow.id.clone(),
                name: workflow.name.clone(),
                description: workflow.description.clone().unwrap_or_default(),
                aliases: Vec::new(),
                keywords: workflow
                    .tags
                    .iter()
                    .chain(workflow.parameter_names.iter())
                    .cloned()
                    .collect(),
            }
        }));

        let search_limit = registry.config().search_limit;
        let ranked = CapabilitySearchIndex::build(documents).search(query, 40);
        let tool_by_id: HashMap<_, _> = tool_definitions
            .iter()
            .map(|definition| (definition.name.as_str(), definition))
            .collect();
        let skill_by_id: HashMap<_, _> = skill_documents
            .iter()
            .map(|skill| (skill.id.to_string(), skill))
            .collect();
        let agent_by_id: HashMap<_, _> = agent_definitions
            .iter()
            .map(|agent| (agent.id.as_str(), agent))
            .collect();
        let workflow_by_id: HashMap<_, _> = workflows
            .iter()
            .map(|workflow| (workflow.id.as_str(), workflow))
            .collect();

        let mut tools_json = Vec::new();
        let mut skills_json = Vec::new();
        let mut agents_json = Vec::new();
        let mut workflows_json = Vec::new();
        let mut unified_json = Vec::new();
        let mut activated_names = Vec::new();
        for hit in ranked {
            let score = hit.score;
            let match_reason = hit.reason.as_str();
            match hit.kind {
                CapabilityKind::Tool if tools_json.len() < search_limit => {
                    let Some(definition) = tool_by_id.get(hit.id.as_str()).copied() else {
                        continue;
                    };
                    activation_set.write().await.activate(definition.clone());
                    activated_names.push(hit.id.clone());
                    let mut result = Self::summary_to_json(definition);
                    result["type"] = serde_json::json!("tool");
                    result["id"] = serde_json::json!(hit.id);
                    result["score"] = serde_json::json!(score);
                    result["match_reason"] = serde_json::json!(match_reason);
                    unified_json.push(result.clone());
                    tools_json.push(result);
                }
                CapabilityKind::Skill if skills_json.len() < 10 => {
                    let Some(skill) = skill_by_id.get(&hit.id) else {
                        continue;
                    };
                    let result = serde_json::json!({
                        "type": "skill",
                        "id": skill.id,
                        "name": skill.name,
                        "description": skill.description,
                        "tags": skill.tags,
                        "score": score,
                        "match_reason": match_reason,
                    });
                    unified_json.push(result.clone());
                    skills_json.push(result);
                }
                CapabilityKind::Agent if agents_json.len() < 10 => {
                    let Some(agent) = agent_by_id.get(hit.id.as_str()).copied() else {
                        continue;
                    };
                    let result = serde_json::json!({
                        "type": "agent",
                        "id": agent.id,
                        "name": agent.name,
                        "description": agent.description,
                        "mode": format!("{:?}", agent.mode),
                        "capabilities": agent.capabilities,
                        "score": score,
                        "match_reason": match_reason,
                        "usage": "Use the 'task' tool to delegate work to this agent: \
                            task({\"agent_name\": \"<id>\", \"prompt\": \"<your_task>\"})",
                    });
                    unified_json.push(result.clone());
                    agents_json.push(result);
                }
                CapabilityKind::Workflow if workflows_json.len() < 10 => {
                    let Some(workflow) = workflow_by_id.get(hit.id.as_str()).copied() else {
                        continue;
                    };
                    let result = serde_json::json!({
                        "type": "workflow",
                        "id": workflow.id,
                        "name": workflow.name,
                        "description": workflow.description,
                        "tags": workflow.tags,
                        "parameter_names": workflow.parameter_names,
                        "score": score,
                        "match_reason": match_reason,
                        "usage": "Use WorkflowRun with this workflow's id or name.",
                    });
                    unified_json.push(result.clone());
                    workflows_json.push(result);
                }
                CapabilityKind::Tool
                | CapabilityKind::Skill
                | CapabilityKind::Agent
                | CapabilityKind::Workflow => {}
            }
        }
        if !workflows_json.is_empty() && !activated_names.iter().any(|name| name == "WorkflowRun") {
            let workflow_run = ToolName::from_string("WorkflowRun");
            if let Some(definition) = registry.get_definition(&workflow_run).await {
                activation_set.write().await.activate(definition);
                activated_names.push("WorkflowRun".to_string());
            }
        }

        // --- 5. Build unified response ---
        let total_count =
            tools_json.len() + skills_json.len() + agents_json.len() + workflows_json.len();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "query": query,
                "note": "Items under 'tools' can be called directly. \
                    To delegate to an agent, use the 'task' tool with the agent's id.",
                "results": unified_json,
                "tools": {
                    "results": tools_json,
                    "count": tools_json.len(),
                    "activated": activated_names,
                },
                "skills": skills_json,
                "agents": agents_json,
                "workflows": workflows_json,
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

fn schema_parameter_names(schema: &serde_json::Value) -> Vec<String> {
    fn visit(value: &serde_json::Value, names: &mut Vec<String>) {
        if let Some(properties) = value
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, nested) in properties {
                names.push(name.clone());
                visit(nested, names);
            }
        }
        if let Some(items) = value.get("items") {
            visit(items, names);
        }
    }

    let mut names = Vec::new();
    visit(schema, &mut names);
    names.sort();
    names.dedup();
    names
}

pub(crate) fn schema_parameter_names_from_text(schema: Option<&str>) -> Vec<String> {
    schema
        .and_then(|value| serde_json::from_str(value).ok())
        .map_or_else(Vec::new, |value| schema_parameter_names(&value))
}

fn tool_aliases(name: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    if let Some(remainder) = name.strip_prefix("mcp_") {
        if let Some((_, bare_name)) = remainder.split_once('_') {
            aliases.push(bare_name.to_string());
        }
    }
    for delimiter in ["::", ".", "/"] {
        if let Some((_, bare_name)) = name.rsplit_once(delimiter) {
            aliases.push(bare_name.to_string());
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
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
            ("WorkflowRun", "Execute a reusable workflow"),
        ] {
            let def = sample_def(name, desc);
            use y_core::tool::ToolRegistry;
            registry.register(def).await.unwrap();
        }

        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let activation_set = Arc::new(RwLock::new(ToolActivationSet::new(20)));

        (registry, taxonomy, activation_set)
    }

    #[test]
    fn workflow_schema_parameter_names_include_nested_inputs() {
        let names = schema_parameter_names_from_text(Some(
            r#"{
                "type": "object",
                "properties": {
                    "repository": {"type": "string"},
                    "release": {
                        "type": "object",
                        "properties": {"version": {"type": "string"}}
                    }
                }
            }"#,
        ));

        assert_eq!(names, vec!["release", "repository", "version"]);
    }

    #[test]
    fn mcp_tool_alias_uses_the_bare_tool_name() {
        assert_eq!(
            tool_aliases("mcp_github_search_repos"),
            vec!["search_repos"]
        );
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
    async fn keyword_search_returns_unified_bm25_ranking() {
        let (registry, taxonomy, activation_set) = setup().await;
        use y_core::tool::ToolRegistry;
        for index in 0..20 {
            registry
                .register(sample_def(
                    &format!("GenericTool{index}"),
                    "General formatting and text conversion helper",
                ))
                .await
                .expect("register generic tool");
        }
        registry
            .register(sample_def(
                "RepositoryIssueLookup",
                "Search repository issue tracker entries by label and state",
            ))
            .await
            .expect("register relevant tool");

        let result = ToolSearchOrchestrator::handle(
            &serde_json::json!({"query": "find repository issues by label"}),
            &registry,
            &taxonomy,
            &activation_set,
        )
        .await
        .expect("search");

        let ranked = result.content["results"]
            .as_array()
            .expect("ranked results");
        assert_eq!(ranked[0]["type"], "tool");
        assert_eq!(ranked[0]["id"], "RepositoryIssueLookup");
        assert!(ranked[0]["score"].as_u64().expect("score") > 0);
        assert_eq!(ranked[0]["match_reason"], "bm25");
    }

    #[tokio::test]
    async fn exact_workflow_id_outranks_cross_type_lexical_matches() {
        let (registry, taxonomy, activation_set) = setup().await;
        use y_core::tool::ToolRegistry;
        registry
            .register(sample_def(
                "ReleaseNotes",
                "Prepare output for the release pipeline",
            ))
            .await
            .expect("register tool");
        let workflows = vec![WorkflowSearchItem {
            id: "release-pipeline".to_string(),
            name: "Release Pipeline".to_string(),
            description: Some("Build, verify, and publish a release".to_string()),
            tags: vec!["build".to_string(), "publish".to_string()],
            parameter_names: vec!["version".to_string()],
        }];
        let sources = CapabilitySearchSources {
            skill_search: None,
            agent_registry: None,
            workflows: Some(&workflows),
        };

        let result = ToolSearchOrchestrator::handle_with_sources(
            &serde_json::json!({"query": "release-pipeline"}),
            &registry,
            &taxonomy,
            &activation_set,
            &sources,
        )
        .await
        .expect("search");

        let ranked = result.content["results"]
            .as_array()
            .expect("ranked results");
        assert_eq!(ranked[0]["type"], "workflow");
        assert_eq!(ranked[0]["id"], "release-pipeline");
        assert_eq!(ranked[0]["score"], 10_000);
        assert_eq!(ranked[0]["match_reason"], "exact_id");
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
        let workflows = vec![WorkflowSearchItem {
            id: "workflow-1".to_string(),
            name: "file-review-pipeline".to_string(),
            description: Some("Review files and summarize findings".to_string()),
            tags: vec!["file".to_string(), "review".to_string()],
            parameter_names: vec!["path".to_string()],
        }];

        let sources = CapabilitySearchSources {
            skill_search: Some(&skill_search),
            agent_registry: Some(&agent_registry),
            workflows: Some(&workflows),
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

        let workflow_results = result.content["workflows"].as_array().unwrap();
        assert_eq!(workflow_results.len(), 1);
        assert_eq!(workflow_results[0]["name"], "file-review-pipeline");
        assert!(tools["activated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "WorkflowRun"));

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

        // Without sources, non-tool capability arrays should be empty.
        let skills = result.content["skills"].as_array().unwrap();
        assert!(skills.is_empty());
        let agents = result.content["agents"].as_array().unwrap();
        assert!(agents.is_empty());
        let workflows = result.content["workflows"].as_array().unwrap();
        assert!(workflows.is_empty());
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
        assert!(result.content["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent"));
    }
}
