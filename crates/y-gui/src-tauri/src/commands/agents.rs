//! Agent management command handlers — list, get detail, save, reset, reload, translate.

#[cfg(test)]
use std::path::Path;

use tauri::State;
use y_service::{
    AgentDetail, AgentInfo, AgentManagementService, AgentSource, AgentToolInfo, PromptSectionInfo,
};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the user agents directory (`<config_dir>/agents/`).
#[cfg(test)]
fn agents_dir(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("agents")
}

#[cfg(test)]
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
    Ok(AgentManagementService::list_agents(&state.container).await)
}

/// Get full detail for a single agent.
#[tauri::command]
pub async fn agent_get(state: State<'_, AppState>, id: String) -> Result<AgentDetail, String> {
    AgentManagementService::get_agent(&state.container, &id).await
}

/// Get the raw TOML source for a single agent definition.
#[tauri::command]
pub async fn agent_source_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<AgentSource, String> {
    AgentManagementService::get_agent_source_info(&state.container, &id).await
}

/// Parse raw agent TOML and return the normalized detail shape used by the GUI.
#[tauri::command]
pub async fn agent_toml_parse(toml_content: String) -> Result<AgentDetail, String> {
    AgentManagementService::parse_agent_toml(&toml_content)
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
    state.container.save_agent(&id, &toml_content).await
}

/// Reset an overridden built-in agent to its original definition.
///
/// Deletes the user override file and restores the original in-memory definition.
#[tauri::command]
pub async fn agent_reset(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.container.reset_agent(&id).await
}

/// Reload all user-defined agents from the agents directory.
///
/// Re-scans `<config_dir>/agents/` and updates the in-memory registry.
#[tauri::command]
pub async fn agent_reload(state: State<'_, AppState>) -> Result<(), String> {
    let (_loaded, errored) = state.container.reload_agents().await;
    if errored == 0 {
        Ok(())
    } else {
        Err(format!("Errors loading agents: {errored}"))
    }
}

/// Translate text using the built-in translator agent.
///
/// Delegates the input text to the `translator` agent and returns the
/// translated output. The target language is determined by the
/// `{{TRANSLATE_TARGET_LANGUAGE}}` template variable set in GUI settings.
#[tauri::command]
pub async fn translate_text(state: State<'_, AppState>, text: String) -> Result<String, String> {
    AgentManagementService::translate_text(&state.container, text).await
}

/// List all registered tool definitions for agent tool configuration.
#[tauri::command]
pub async fn agent_tool_list(state: State<'_, AppState>) -> Result<Vec<AgentToolInfo>, String> {
    Ok(AgentManagementService::list_tools(&state.container).await)
}

/// List built-in prompt sections that can be selected for an agent preset.
#[tauri::command]
pub async fn agent_prompt_section_list(
    state: State<'_, AppState>,
) -> Result<Vec<PromptSectionInfo>, String> {
    Ok(AgentManagementService::list_prompt_sections(
        &state.config_dir,
    ))
}

#[cfg(test)]
mod tests {
    use super::{load_agent_source, AgentDetail};
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
            workspace_isolation: y_core::agent::WorkspaceIsolationPreference::default(),
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
            fallback_provider_tags: vec![],
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
            mcp_mode: None,
            mcp_servers: vec![],
            prune_tool_history: false,
            auto_update: true,
            response_format: None,
        }
    }

    #[test]
    fn detail_from_definition_maps_user_facing_fields() {
        let detail = AgentDetail::from_definition(&sample_definition(), true);
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
