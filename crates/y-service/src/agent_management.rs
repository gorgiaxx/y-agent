//! Agent CRUD operations extracted from [`ServiceContainer`].
//!
//! Follows the same unit-struct + static-method pattern as
//! [`crate::chat::ChatService`]: all methods take `&ServiceContainer`
//! (or sub-components thereof) as their first argument so the service
//! carries no state of its own.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;
use tracing::info;
use y_agent::agent::definition::AgentDefinition;
use y_agent::AgentRegistry;
use y_core::agent::ContextStrategyHint;
use y_core::runtime::RuntimeBackend;

use crate::container::ServiceContainer;

/// Feature switches resolved for presentation clients.
#[derive(Debug, Clone, Serialize)]
pub struct AgentFeatureFlags {
    pub toolcall: bool,
    pub skills: bool,
    pub knowledge: bool,
}

/// Agent summary shared by all presentation layers.
#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub description: String,
    pub mode: String,
    pub trust_tier: String,
    pub capabilities: Vec<String>,
    pub working_directory: Option<String>,
    pub provider_id: Option<String>,
    pub features: AgentFeatureFlags,
    pub user_callable: bool,
    pub is_overridden: bool,
}

/// Full agent configuration shared by all presentation layers.
#[derive(Debug, Clone, Serialize)]
pub struct AgentDetail {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub description: String,
    pub mode: String,
    pub trust_tier: String,
    pub capabilities: Vec<String>,
    pub working_directory: Option<String>,
    pub allowed_tools: Vec<String>,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub features: AgentFeatureFlags,
    pub knowledge_collections: Vec<String>,
    pub prompt_section_ids: Vec<String>,
    pub provider_id: Option<String>,
    pub preferred_models: Vec<String>,
    pub fallback_models: Vec<String>,
    pub provider_tags: Vec<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub plan_mode: Option<String>,
    pub thinking_effort: Option<String>,
    pub permission_mode: Option<String>,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub timeout_secs: u64,
    pub context_sharing: String,
    pub max_context_tokens: usize,
    pub max_completion_tokens: Option<usize>,
    pub user_callable: bool,
    pub is_overridden: bool,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Vec<String>,
}

impl AgentDetail {
    /// Convert a domain definition into the stable presentation contract.
    pub fn from_definition(definition: &AgentDefinition, is_overridden: bool) -> Self {
        Self {
            id: definition.id.clone(),
            name: definition.name.clone(),
            icon: definition.icon.clone(),
            description: definition.description.clone(),
            mode: format!("{:?}", definition.mode).to_lowercase(),
            trust_tier: format!("{:?}", definition.trust_tier),
            capabilities: definition.capabilities.clone(),
            working_directory: definition.working_directory.clone(),
            allowed_tools: definition.allowed_tools.clone(),
            system_prompt: definition.system_prompt.clone(),
            skills: definition.skills.clone(),
            features: AgentFeatureFlags::from_definition(definition),
            knowledge_collections: definition.knowledge_collections.clone(),
            prompt_section_ids: definition.prompt_section_ids.clone(),
            provider_id: definition.provider_id.clone(),
            preferred_models: definition.preferred_models.clone(),
            fallback_models: definition.fallback_models.clone(),
            provider_tags: definition.provider_tags.clone(),
            temperature: definition.temperature,
            top_p: definition.top_p,
            plan_mode: definition.plan_mode.clone(),
            thinking_effort: definition.thinking_effort.clone(),
            permission_mode: definition.permission_mode.map(|mode| mode.to_string()),
            max_iterations: definition.max_iterations,
            max_tool_calls: definition.max_tool_calls,
            timeout_secs: definition.timeout_secs,
            context_sharing: format!("{:?}", definition.context_sharing).to_lowercase(),
            max_context_tokens: definition.max_context_tokens,
            max_completion_tokens: definition.max_completion_tokens,
            user_callable: definition.user_callable,
            is_overridden,
            mcp_mode: definition.mcp_mode.clone(),
            mcp_servers: definition.mcp_servers.clone(),
        }
    }
}

impl AgentFeatureFlags {
    fn from_definition(definition: &AgentDefinition) -> Self {
        Self {
            toolcall: definition.toolcall_enabled_resolved(),
            skills: definition.skills_enabled_resolved(),
            knowledge: definition.knowledge_enabled_resolved(),
        }
    }
}

/// Tool metadata used by agent configuration clients.
#[derive(Debug, Clone, Serialize)]
pub struct AgentToolInfo {
    pub name: String,
    pub description: String,
    pub category: String,
    pub is_dangerous: bool,
}

/// Built-in prompt section exposed to agent configuration clients.
#[derive(Debug, Clone, Serialize)]
pub struct PromptSectionInfo {
    pub id: String,
    pub category: String,
    pub priority: i32,
    pub content: String,
    pub condition: Option<String>,
}

/// Raw agent source returned to presentation clients.
#[derive(Debug, Clone, Serialize)]
pub struct AgentSource {
    pub path: String,
    pub content: String,
    pub is_user_file: bool,
}

/// Stateless service encapsulating agent definition management (CRUD,
/// reload, callable-text refresh).
pub struct AgentManagementService;

impl AgentManagementService {
    /// List registered agents using the shared presentation contract.
    pub async fn list_agents(container: &ServiceContainer) -> Vec<AgentInfo> {
        let registry = container.agent_registry.lock().await;
        let mut agents = registry
            .list()
            .iter()
            .map(|definition| AgentInfo {
                id: definition.id.clone(),
                name: definition.name.clone(),
                icon: definition.icon.clone(),
                description: definition.description.clone(),
                mode: format!("{:?}", definition.mode).to_lowercase(),
                trust_tier: format!("{:?}", definition.trust_tier),
                capabilities: definition.capabilities.clone(),
                working_directory: definition.working_directory.clone(),
                provider_id: definition.provider_id.clone(),
                features: AgentFeatureFlags::from_definition(definition),
                user_callable: definition.user_callable,
                is_overridden: registry.is_overridden(&definition.id),
            })
            .collect::<Vec<_>>();

        agents.sort_by(|left, right| {
            agent_tier_order(&left.trust_tier)
                .cmp(&agent_tier_order(&right.trust_tier))
                .then(left.name.cmp(&right.name))
        });
        agents
    }

    /// Get a registered agent using the shared presentation contract.
    pub async fn get_agent(container: &ServiceContainer, id: &str) -> Result<AgentDetail, String> {
        let registry = container.agent_registry.lock().await;
        let definition = registry
            .get(id)
            .ok_or_else(|| format!("Agent not found: {id}"))?;
        Ok(AgentDetail::from_definition(
            definition,
            registry.is_overridden(&definition.id),
        ))
    }

    /// Parse raw agent TOML into the shared presentation contract.
    pub fn parse_agent_toml(toml_content: &str) -> Result<AgentDetail, String> {
        let definition = AgentDefinition::from_toml(toml_content)
            .map_err(|error| format!("Invalid agent TOML: {error}"))?;
        Ok(AgentDetail::from_definition(&definition, false))
    }

    /// List registered tools for agent configuration clients.
    pub async fn list_tools(container: &ServiceContainer) -> Vec<AgentToolInfo> {
        let mut tools = container
            .tool_registry
            .get_all_definitions()
            .await
            .into_iter()
            .map(|definition| AgentToolInfo {
                name: definition.name.0,
                description: definition.description,
                category: format!("{:?}", definition.category).to_lowercase(),
                is_dangerous: definition.is_dangerous,
            })
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools
    }

    /// List built-in prompt sections for agent configuration clients.
    pub fn list_prompt_sections(config_dir: &Path) -> Vec<PromptSectionInfo> {
        let prompts_dir = config_dir.join("prompts");
        let store = y_prompt::builtin_section_store_with_overrides(
            prompts_dir.is_dir().then_some(prompts_dir.as_path()),
            &RuntimeBackend::Native,
        );
        let mut sections = store
            .section_ids()
            .into_iter()
            .filter_map(|id| {
                store.get(id).map(|section| PromptSectionInfo {
                    id: id.to_string(),
                    category: format!("{:?}", section.category).to_lowercase(),
                    priority: section.priority,
                    content: store.load_content(id).unwrap_or_default(),
                    condition: section
                        .condition
                        .as_ref()
                        .map(|condition| format!("{condition:?}")),
                })
            })
            .collect::<Vec<_>>();
        sections.sort_by(|left, right| left.id.cmp(&right.id));
        sections
    }

    /// Translate text through the configured translator agent.
    pub async fn translate_text(
        container: &ServiceContainer,
        text: String,
    ) -> Result<String, String> {
        let input = serde_json::json!({ "text": text });
        let result = container
            .agent_delegator
            .delegate("translator", input, ContextStrategyHint::None, None)
            .await
            .map_err(|error| format!("Translation failed: {error}"))?;
        Ok(result.text)
    }

    /// Hot-reload agent definitions from the agents directory.
    ///
    /// Re-scans all `*.toml` files in the user agents directory and
    /// registers (or overrides) definitions. Built-in agents are preserved.
    /// Newly created agent files take effect immediately without restart.
    ///
    /// Returns `(loaded, errored)` counts.
    pub async fn reload_agents(container: &ServiceContainer) -> (usize, usize) {
        let mut registry = container.agent_registry.lock().await;
        let (loaded, errored) = registry.reload_user_agents_from_dir();
        info!(loaded, errored, "Agent definitions hot-reloaded");
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        (loaded, errored)
    }

    /// Register a single agent from raw TOML content at runtime.
    ///
    /// Useful when `agent-architect` creates a new agent definition and
    /// wants it to take effect immediately without a full directory scan.
    ///
    /// Returns the registered agent's ID on success.
    pub async fn register_agent_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<String, String> {
        let mut registry = container.agent_registry.lock().await;
        let id = registry.register_agent_from_toml(toml_content)?;
        info!(agent_id = %id, "Agent definition registered at runtime");
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(id)
    }

    /// Refresh the callable agents text injected into the orchestration prompt.
    ///
    /// Reads all definitions from the registry where `user_callable == true`
    /// and writes a markdown-formatted summary into the shared handle.
    pub(crate) async fn refresh_callable_agents_text(
        registry: &AgentRegistry,
        handle: &Arc<RwLock<String>>,
    ) {
        let callable: Vec<_> = registry
            .list()
            .into_iter()
            .filter(|d| d.user_callable)
            .collect();

        let text = if callable.is_empty() {
            String::from("### User-Callable Agents\n\n(none currently registered)")
        } else {
            let mut buf = String::from("### User-Callable Agents\n\n");
            for agent in &callable {
                use std::fmt::Write;
                let _ = writeln!(
                    buf,
                    "- **{}**: {} (mode: {:?}, capabilities: [{}])",
                    agent.id,
                    agent.description,
                    agent.mode,
                    agent.capabilities.join(", "),
                );
            }
            buf
        };

        let mut guard = handle.write().await;
        *guard = text;
    }

    /// Populate the callable agents text at startup.
    ///
    /// Called once after construction so the first prompt assembly has the list.
    pub async fn init_callable_agents_text(container: &ServiceContainer) {
        let registry = container.agent_registry.lock().await;
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
    }

    /// Save an agent definition from raw TOML content to the agents directory.
    ///
    /// Parses the TOML, writes the file to disk, and registers the definition
    /// in the agent registry with `UserDefined` trust tier.
    pub async fn save_agent(
        container: &ServiceContainer,
        id: &str,
        toml_content: &str,
    ) -> Result<(), String> {
        let (mut def, dir) = {
            let registry = container.agent_registry.lock().await;
            let expanded_toml = registry.expand_templates(toml_content);
            let mut def = y_agent::agent::definition::AgentDefinition::from_toml(&expanded_toml)
                .map_err(|e| format!("Invalid agent TOML: {e}"))?;
            def.id = id.to_string();
            let dir = registry
                .agents_dir()
                .ok_or_else(|| "no agents directory configured".to_string())?
                .to_path_buf();
            (def, dir)
        };

        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("failed to create agents directory: {e}"))?;

        let file_path = dir.join(format!("{id}.toml"));
        tokio::fs::write(&file_path, toml_content)
            .await
            .map_err(|e| format!("failed to write agent file: {e}"))?;

        def.trust_tier = y_agent::TrustTier::UserDefined;
        let mut registry = container.agent_registry.lock().await;
        let _ = registry.register_or_override(def);

        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(())
    }

    /// Reset an overridden built-in agent to its original definition.
    ///
    /// Removes the user override file from disk and restores the built-in
    /// definition in the registry.
    pub async fn reset_agent(container: &ServiceContainer, id: &str) -> Result<(), String> {
        let file_path = {
            let mut registry = container.agent_registry.lock().await;
            registry
                .reset_builtin(id)
                .map_err(|e| format!("failed to reset agent: {e}"))?;
            registry
                .agents_dir()
                .map(|dir| dir.join(format!("{id}.toml")))
        };

        if let Some(file_path) = file_path {
            if file_path.exists() {
                tokio::fs::remove_file(&file_path)
                    .await
                    .map_err(|e| format!("failed to remove override file: {e}"))?;
            }
        }

        let registry = container.agent_registry.lock().await;
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(())
    }

    /// Read the raw TOML source for an agent definition.
    ///
    /// Returns `(path, content, is_user_file)`. If a user override file exists
    /// on disk, returns its content; otherwise serializes the in-memory definition.
    pub async fn get_agent_source(
        container: &ServiceContainer,
        id: &str,
    ) -> Result<(String, String, bool), String> {
        let (def, file_path) = {
            let registry = container.agent_registry.lock().await;
            let def = registry
                .get(id)
                .ok_or_else(|| format!("agent not found: {id}"))?
                .clone();
            let file_path = registry
                .agents_dir()
                .map(|d| d.join(format!("{}.toml", def.id)))
                .unwrap_or_default();
            (def, file_path)
        };

        if file_path.exists() {
            let content = tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| format!("failed to read agent file: {e}"))?;
            return Ok((file_path.display().to_string(), content, true));
        }

        let content =
            toml::to_string_pretty(&def).map_err(|e| format!("failed to serialize agent: {e}"))?;
        Ok((file_path.display().to_string(), content, false))
    }

    /// Read raw agent source using the shared presentation contract.
    pub async fn get_agent_source_info(
        container: &ServiceContainer,
        id: &str,
    ) -> Result<AgentSource, String> {
        let (path, content, is_user_file) = Self::get_agent_source(container, id).await?;
        Ok(AgentSource {
            path,
            content,
            is_user_file,
        })
    }
}

fn agent_tier_order(trust_tier: &str) -> u8 {
    match trust_tier {
        "BuiltIn" => 0,
        "UserDefined" => 1,
        "Dynamic" => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_agent_detail_includes_mcp_contract_fields() {
        let detail = AgentManagementService::parse_agent_toml(
            r#"
id = "reviewer"
name = "Reviewer"
description = "Reviews code"
mode = "general"
trust_tier = "user_defined"
system_prompt = "Review carefully."
mcp_mode = "manual"
mcp_servers = ["github"]
"#,
        )
        .expect("parse agent detail");

        assert_eq!(detail.id, "reviewer");
        assert_eq!(detail.mcp_mode.as_deref(), Some("manual"));
        assert_eq!(detail.mcp_servers, ["github"]);
    }
}
