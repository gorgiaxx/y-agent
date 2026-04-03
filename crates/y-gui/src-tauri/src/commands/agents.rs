//! Agent management command handlers — list, get detail, save, reset, reload.

use std::path::Path;

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Agent summary info returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    pub trust_tier: String,
    pub capabilities: Vec<String>,
    pub is_overridden: bool,
}

/// Full agent detail returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct AgentDetail {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    pub trust_tier: String,
    pub capabilities: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub preferred_models: Vec<String>,
    pub fallback_models: Vec<String>,
    pub provider_tags: Vec<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub timeout_secs: u64,
    pub context_sharing: String,
    pub max_context_tokens: usize,
    pub max_completion_tokens: Option<usize>,
    pub is_overridden: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the user agents directory (`<config_dir>/agents/`).
fn agents_dir(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("agents")
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
            description: def.description.clone(),
            mode: format!("{:?}", def.mode).to_lowercase(),
            trust_tier: format!("{:?}", def.trust_tier),
            capabilities: def.capabilities.clone(),
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

    Ok(AgentDetail {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        mode: format!("{:?}", def.mode).to_lowercase(),
        trust_tier: format!("{:?}", def.trust_tier),
        capabilities: def.capabilities.clone(),
        allowed_tools: def.allowed_tools.clone(),
        denied_tools: def.denied_tools.clone(),
        system_prompt: def.system_prompt.clone(),
        skills: def.skills.clone(),
        preferred_models: def.preferred_models.clone(),
        fallback_models: def.fallback_models.clone(),
        provider_tags: def.provider_tags.clone(),
        temperature: def.temperature,
        top_p: def.top_p,
        max_iterations: def.max_iterations,
        max_tool_calls: def.max_tool_calls,
        timeout_secs: def.timeout_secs,
        context_sharing: format!("{:?}", def.context_sharing).to_lowercase(),
        max_context_tokens: def.max_context_tokens,
        max_completion_tokens: def.max_completion_tokens,
        is_overridden: registry.is_overridden(&def.id),
    })
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
