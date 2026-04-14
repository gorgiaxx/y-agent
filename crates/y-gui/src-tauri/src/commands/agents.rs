//! Agent management command handlers — list, get detail, save, reset, reload, translate.

use std::path::Path;

use serde::Serialize;
use tauri::State;
use y_core::agent::ContextStrategyHint;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Agent summary info returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct AgentFeatureFlags {
    pub toolcall: bool,
    pub skills: bool,
    pub knowledge: bool,
}

#[derive(Debug, Serialize, Clone)]
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

/// Full agent detail returned to the frontend.
#[derive(Debug, Serialize, Clone)]
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
}

/// Tool info returned for agent tool-selection settings.
#[derive(Debug, Serialize, Clone)]
pub struct AgentToolInfo {
    pub name: String,
    pub description: String,
    pub category: String,
    pub is_dangerous: bool,
}

/// Built-in prompt section info for agent prompt-selection settings.
#[derive(Debug, Serialize, Clone)]
pub struct PromptSectionInfo {
    pub id: String,
    pub category: String,
}

/// Raw agent source content used by the frontend raw editor.
#[derive(Debug, Serialize, Clone)]
pub struct AgentSource {
    pub path: String,
    pub content: String,
    pub is_user_file: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the user agents directory (`<config_dir>/agents/`).
fn agents_dir(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("agents")
}

fn detail_from_definition(
    def: &y_agent::agent::definition::AgentDefinition,
    is_overridden: bool,
) -> AgentDetail {
    AgentDetail {
        id: def.id.clone(),
        name: def.name.clone(),
        icon: def.icon.clone(),
        description: def.description.clone(),
        mode: format!("{:?}", def.mode).to_lowercase(),
        trust_tier: format!("{:?}", def.trust_tier),
        capabilities: def.capabilities.clone(),
        working_directory: def.working_directory.clone(),
        allowed_tools: def.allowed_tools.clone(),
        system_prompt: def.system_prompt.clone(),
        skills: def.skills.clone(),
        features: AgentFeatureFlags {
            toolcall: def.toolcall_enabled_resolved(),
            skills: def.skills_enabled_resolved(),
            knowledge: def.knowledge_enabled_resolved(),
        },
        knowledge_collections: def.knowledge_collections.clone(),
        prompt_section_ids: def.prompt_section_ids.clone(),
        provider_id: def.provider_id.clone(),
        preferred_models: def.preferred_models.clone(),
        fallback_models: def.fallback_models.clone(),
        provider_tags: def.provider_tags.clone(),
        temperature: def.temperature,
        top_p: def.top_p,
        plan_mode: def.plan_mode.clone(),
        thinking_effort: def.thinking_effort.clone(),
        permission_mode: def.permission_mode.map(|mode| mode.to_string()),
        max_iterations: def.max_iterations,
        max_tool_calls: def.max_tool_calls,
        timeout_secs: def.timeout_secs,
        context_sharing: format!("{:?}", def.context_sharing).to_lowercase(),
        max_context_tokens: def.max_context_tokens,
        max_completion_tokens: def.max_completion_tokens,
        user_callable: def.user_callable,
        is_overridden,
    }
}

fn load_agent_source(
    config_dir: &Path,
    def: &y_agent::agent::definition::AgentDefinition,
) -> Result<AgentSource, String> {
    let file_path = agents_dir(config_dir).join(format!("{}.toml", def.id));
    if file_path.exists() {
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read agent file: {e}"))?;
        return Ok(AgentSource {
            path: file_path.display().to_string(),
            content,
            is_user_file: true,
        });
    }

    let content = toml::to_string_pretty(def)
        .map_err(|e| format!("Failed to serialize agent definition: {e}"))?;
    Ok(AgentSource {
        path: file_path.display().to_string(),
        content,
        is_user_file: false,
    })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all registered agent definitions.
#[tauri::command]
pub async fn agent_list(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    let registry = state.container.agent_registry.lock().await;

    let mut agents: Vec<AgentInfo> = registry
        .list()
        .iter()
        .map(|def| AgentInfo {
            id: def.id.clone(),
            name: def.name.clone(),
            icon: def.icon.clone(),
            description: def.description.clone(),
            mode: format!("{:?}", def.mode).to_lowercase(),
            trust_tier: format!("{:?}", def.trust_tier),
            capabilities: def.capabilities.clone(),
            working_directory: def.working_directory.clone(),
            provider_id: def.provider_id.clone(),
            features: AgentFeatureFlags {
                toolcall: def.toolcall_enabled_resolved(),
                skills: def.skills_enabled_resolved(),
                knowledge: def.knowledge_enabled_resolved(),
            },
            user_callable: def.user_callable,
            is_overridden: registry.is_overridden(&def.id),
        })
        .collect();

    // Sort: built-in first, then user-defined, then dynamic; alphabetically within each tier.
    agents.sort_by(|a, b| {
        let tier_order = |t: &str| match t {
            "BuiltIn" => 0,
            "UserDefined" => 1,
            "Dynamic" => 2,
            _ => 3,
        };
        tier_order(&a.trust_tier)
            .cmp(&tier_order(&b.trust_tier))
            .then(a.name.cmp(&b.name))
    });

    Ok(agents)
}

/// Get full detail for a single agent.
#[tauri::command]
pub async fn agent_get(state: State<'_, AppState>, id: String) -> Result<AgentDetail, String> {
    let registry = state.container.agent_registry.lock().await;

    let def = registry
        .get(&id)
        .ok_or_else(|| format!("Agent not found: {id}"))?;

    Ok(detail_from_definition(def, registry.is_overridden(&def.id)))
}

/// Get the raw TOML source for a single agent definition.
#[tauri::command]
pub async fn agent_source_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentSource, String> {
    let registry = state.container.agent_registry.lock().await;
    let def = registry
        .get(&id)
        .ok_or_else(|| format!("Agent not found: {id}"))?;
    load_agent_source(&state.config_dir, def)
}

/// Parse raw agent TOML and return the normalized detail shape used by the GUI.
#[tauri::command]
pub async fn agent_toml_parse(toml_content: String) -> Result<AgentDetail, String> {
    let def = y_agent::agent::definition::AgentDefinition::from_toml(&toml_content)
        .map_err(|e| format!("Invalid agent TOML: {e}"))?;
    Ok(detail_from_definition(&def, false))
}

/// Save (create or update) a user agent definition.
///
/// Writes TOML to `<config_dir>/agents/<id>.toml` and updates the in-memory registry.
#[tauri::command]
pub async fn agent_save(
    state: State<'_, AppState>,
    id: String,
    toml_content: String,
) -> Result<(), String> {
    let mut registry = state.container.agent_registry.lock().await;

    // Parse the TOML to validate it's a proper AgentDefinition.
    let expanded_toml = registry.expand_templates(&toml_content);
    let mut def = y_agent::agent::definition::AgentDefinition::from_toml(&expanded_toml)
        .map_err(|e| format!("Invalid agent TOML: {e}"))?;

    // Ensure the ID matches.
    def.id.clone_from(&id);

    // Write to disk.
    let dir = agents_dir(&state.config_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create agents directory: {e}"))?;

    let file_path = dir.join(format!("{id}.toml"));
    std::fs::write(&file_path, &toml_content)
        .map_err(|e| format!("Failed to write agent file: {e}"))?;

    // Update in-memory registry (forces UserDefined tier).
    def.trust_tier = y_agent::TrustTier::UserDefined;
    let _ = registry.register_or_override(def);

    Ok(())
}

/// Reset an overridden built-in agent to its original definition.
///
/// Deletes the user override file and restores the original in-memory definition.
#[tauri::command]
pub async fn agent_reset(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut registry = state.container.agent_registry.lock().await;

    // Reset in-memory.
    registry
        .reset_builtin(&id)
        .map_err(|e| format!("Failed to reset agent: {e}"))?;

    // Remove the override file if it exists.
    let file_path = agents_dir(&state.config_dir).join(format!("{id}.toml"));
    if file_path.exists() {
        std::fs::remove_file(&file_path)
            .map_err(|e| format!("Failed to remove override file: {e}"))?;
    }

    Ok(())
}

/// Reload all user-defined agents from the agents directory.
///
/// Re-scans `<config_dir>/agents/` and updates the in-memory registry.
#[tauri::command]
pub async fn agent_reload(state: State<'_, AppState>) -> Result<(), String> {
    let dir = agents_dir(&state.config_dir);
    if !dir.exists() {
        return Ok(());
    }

    let mut registry = state.container.agent_registry.lock().await;
    registry.load_user_agents(&dir).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|(f, e)| format!("{f}: {e}")).collect();
        format!("Errors loading agents: {}", msgs.join("; "))
    })?;

    Ok(())
}

/// Translate text using the built-in translator agent.
///
/// Delegates the input text to the `translator` agent and returns the
/// translated output. The target language is determined by the
/// `{{TRANSLATE_TARGET_LANGUAGE}}` template variable set in GUI settings.
#[tauri::command]
pub async fn translate_text(state: State<'_, AppState>, text: String) -> Result<String, String> {
    let input = serde_json::json!({ "text": text });
    let result = state
        .container
        .agent_delegator
        .delegate("translator", input, ContextStrategyHint::None, None)
        .await
        .map_err(|e| format!("Translation failed: {e}"))?;
    Ok(result.text)
}

/// List all registered tool definitions for agent tool configuration.
#[tauri::command]
pub async fn agent_tool_list(state: State<'_, AppState>) -> Result<Vec<AgentToolInfo>, String> {
    let mut tools: Vec<AgentToolInfo> = state
        .container
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
        .collect();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(tools)
}

/// List built-in prompt sections that can be selected for an agent preset.
#[tauri::command]
pub async fn agent_prompt_section_list() -> Result<Vec<PromptSectionInfo>, String> {
    let store = y_prompt::builtin_section_store();
    let mut sections: Vec<PromptSectionInfo> = store
        .section_ids()
        .into_iter()
        .filter_map(|id| {
            store.get(id).map(|section| PromptSectionInfo {
                id: id.to_string(),
                category: format!("{:?}", section.category).to_lowercase(),
            })
        })
        .collect();
    sections.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::{detail_from_definition, load_agent_source};
    use tempfile::tempdir;
    use y_agent::agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
    use y_agent::TrustTier;
    use y_core::permission_types::PermissionMode;

    fn sample_definition() -> AgentDefinition {
        AgentDefinition {
            id: "reviewer".to_string(),
            name: "Reviewer".to_string(),
            description: "Reviews code".to_string(),
            mode: AgentMode::General,
            trust_tier: TrustTier::UserDefined,
            capabilities: vec!["chat".to_string()],
            icon: Some("R".to_string()),
            working_directory: Some("/tmp/workspace".to_string()),
            toolcall_enabled: Some(true),
            skills_enabled: Some(true),
            knowledge_enabled: Some(false),
            allowed_tools: vec!["read_file".to_string()],
            system_prompt: "Be strict.".to_string(),
            skills: vec!["code-review".to_string()],
            knowledge_collections: vec![],
            prompt_section_ids: vec!["safety".to_string()],
            provider_id: Some("openai".to_string()),
            preferred_models: vec!["gpt-5".to_string()],
            fallback_models: vec!["gpt-4.1".to_string()],
            provider_tags: vec!["code".to_string()],
            temperature: Some(0.2),
            top_p: Some(0.9),
            plan_mode: Some("plan".to_string()),
            thinking_effort: Some("high".to_string()),
            permission_mode: Some(PermissionMode::AcceptEdits),
            max_iterations: 12,
            max_tool_calls: 24,
            timeout_secs: 90,
            context_sharing: ContextStrategy::Summary,
            max_context_tokens: 2048,
            max_completion_tokens: Some(512),
            user_callable: true,
            prune_tool_history: false,
            auto_update: true,
        }
    }

    #[test]
    fn detail_from_definition_maps_user_facing_fields() {
        let detail = detail_from_definition(&sample_definition(), true);
        assert_eq!(detail.id, "reviewer");
        assert_eq!(detail.provider_id.as_deref(), Some("openai"));
        assert_eq!(detail.plan_mode.as_deref(), Some("plan"));
        assert_eq!(detail.permission_mode.as_deref(), Some("accept_edits"));
        assert!(detail.features.toolcall);
        assert!(detail.is_overridden);
    }

    #[test]
    fn load_agent_source_prefers_existing_user_file() {
        let dir = tempdir().expect("tempdir");
        let agent_dir = dir.path().join("agents");
        std::fs::create_dir_all(&agent_dir).expect("create agents dir");
        let file_path = agent_dir.join("reviewer.toml");
        std::fs::write(&file_path, "id = \"reviewer\"\nname = \"Reviewer\"\n")
            .expect("write agent file");

        let source = load_agent_source(dir.path(), &sample_definition()).expect("load source");
        assert!(source.is_user_file);
        assert_eq!(source.path, file_path.display().to_string());
        assert!(source.content.contains("name = \"Reviewer\""));
    }

    #[test]
    fn load_agent_source_serializes_definition_when_no_user_file_exists() {
        let dir = tempdir().expect("tempdir");
        let source = load_agent_source(dir.path(), &sample_definition()).expect("load source");
        assert!(!source.is_user_file);
        assert!(source.path.ends_with("agents/reviewer.toml"));
        assert!(source.content.contains("system_prompt = \"Be strict.\""));
    }
}
