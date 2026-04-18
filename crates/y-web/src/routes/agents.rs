//! Agent management endpoints.
//!
//! Mirrors all agent-related Tauri commands from the GUI.

use std::path::Path;

use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::agent::ContextStrategyHint;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AgentFeatureFlags {
    pub toolcall: bool,
    pub skills: bool,
    pub knowledge: bool,
}

/// Agent summary info returned in the list.
#[derive(Debug, Serialize)]
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

/// Full agent detail.
#[derive(Debug, Serialize)]
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

/// Raw agent source content.
#[derive(Debug, Serialize)]
pub struct AgentSource {
    pub path: String,
    pub content: String,
    pub is_user_file: bool,
}

/// Tool info for agent tool-selection settings.
#[derive(Debug, Serialize)]
pub struct AgentToolInfo {
    pub name: String,
    pub description: String,
    pub category: String,
    pub is_dangerous: bool,
}

/// Built-in prompt section info.
#[derive(Debug, Serialize)]
pub struct PromptSectionInfo {
    pub id: String,
    pub category: String,
}

/// Request body for `PUT /api/v1/agents/:id`.
#[derive(Debug, Deserialize)]
pub struct SaveAgentRequest {
    pub toml_content: String,
}

/// Request body for `POST /api/v1/agents/parse-toml`.
#[derive(Debug, Deserialize)]
pub struct ParseTomlRequest {
    pub toml_content: String,
}

/// Request body for `POST /api/v1/agents/translate`.
#[derive(Debug, Deserialize)]
pub struct TranslateRequest {
    pub text: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/agents` -- list registered agent definitions.
async fn list_agents(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
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

    Ok(Json(agents))
}

/// `GET /api/v1/agents/:id` -- get a single agent definition.
async fn get_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let registry = state.container.agent_registry.lock().await;
    let def = registry
        .get(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Agent not found: {id}")))?;

    Ok(Json(detail_from_definition(
        def,
        registry.is_overridden(&def.id),
    )))
}

/// `GET /api/v1/agents/:id/source` -- get the raw TOML source.
async fn get_agent_source(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let registry = state.container.agent_registry.lock().await;
    let def = registry
        .get(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Agent not found: {id}")))?;

    let file_path = agents_dir(&state.config_dir).join(format!("{}.toml", def.id));
    if file_path.exists() {
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| ApiError::Internal(format!("Failed to read agent file: {e}")))?;
        return Ok(Json(AgentSource {
            path: file_path.display().to_string(),
            content,
            is_user_file: true,
        }));
    }

    let content = toml::to_string_pretty(def)
        .map_err(|e| ApiError::Internal(format!("Failed to serialize agent: {e}")))?;
    Ok(Json(AgentSource {
        path: file_path.display().to_string(),
        content,
        is_user_file: false,
    }))
}

/// `POST /api/v1/agents/parse-toml` -- parse raw agent TOML.
async fn parse_toml(Json(body): Json<ParseTomlRequest>) -> Result<impl IntoResponse, ApiError> {
    let def = y_agent::agent::definition::AgentDefinition::from_toml(&body.toml_content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid agent TOML: {e}")))?;
    Ok(Json(detail_from_definition(&def, false)))
}

/// `PUT /api/v1/agents/:id` -- save (create or update) a user agent definition.
async fn save_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SaveAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut registry = state.container.agent_registry.lock().await;

    let expanded_toml = registry.expand_templates(&body.toml_content);
    let mut def = y_agent::agent::definition::AgentDefinition::from_toml(&expanded_toml)
        .map_err(|e| ApiError::BadRequest(format!("Invalid agent TOML: {e}")))?;

    def.id.clone_from(&id);

    let dir = agents_dir(&state.config_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create agents directory: {e}")))?;

    let file_path = dir.join(format!("{id}.toml"));
    tokio::fs::write(&file_path, &body.toml_content)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write agent file: {e}")))?;

    def.trust_tier = y_agent::TrustTier::UserDefined;
    let _ = registry.register_or_override(def);

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `POST /api/v1/agents/:id/reset` -- reset an overridden built-in agent.
async fn reset_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let mut registry = state.container.agent_registry.lock().await;
    registry
        .reset_builtin(&id)
        .map_err(|e| ApiError::Internal(format!("Failed to reset agent: {e}")))?;

    let file_path = agents_dir(&state.config_dir).join(format!("{id}.toml"));
    if file_path.exists() {
        tokio::fs::remove_file(&file_path)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to remove override file: {e}")))?;
    }

    Ok(Json(serde_json::json!({"message": "reset"})))
}

/// `POST /api/v1/agents/reload` -- reload all user-defined agents.
async fn reload_agents(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let dir = agents_dir(&state.config_dir);
    if !dir.exists() {
        return Ok(Json(serde_json::json!({"message": "no agents directory"})));
    }

    let mut registry = state.container.agent_registry.lock().await;
    registry.load_user_agents(&dir).map_err(|errs| {
        let msgs: Vec<String> = errs.iter().map(|(f, e)| format!("{f}: {e}")).collect();
        ApiError::Internal(format!("Errors loading agents: {}", msgs.join("; ")))
    })?;

    Ok(Json(serde_json::json!({"message": "reloaded"})))
}

/// `GET /api/v1/agents/tools` -- list all registered tool definitions.
async fn list_tools(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
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
    Ok(Json(tools))
}

/// `GET /api/v1/agents/prompt-sections` -- list built-in prompt sections.
async fn list_prompt_sections() -> Result<impl IntoResponse, ApiError> {
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
    Ok(Json(sections))
}

/// `POST /api/v1/agents/translate` -- translate text using the translator agent.
async fn translate_text(
    State(state): State<AppState>,
    Json(body): Json<TranslateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let input = serde_json::json!({ "text": body.text });
    let result = state
        .container
        .agent_delegator
        .delegate("translator", input, ContextStrategyHint::None, None)
        .await
        .map_err(|e| ApiError::Internal(format!("Translation failed: {e}")))?;
    Ok(Json(serde_json::json!({ "text": result.text })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Agent route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/agents", get(list_agents))
        .route("/api/v1/agents/tools", get(list_tools))
        .route("/api/v1/agents/prompt-sections", get(list_prompt_sections))
        .route("/api/v1/agents/parse-toml", post(parse_toml))
        .route("/api/v1/agents/reload", post(reload_agents))
        .route("/api/v1/agents/translate", post(translate_text))
        .route("/api/v1/agents/{id}", get(get_agent).put(save_agent))
        .route("/api/v1/agents/{id}/source", get(get_agent_source))
        .route("/api/v1/agents/{id}/reset", post(reset_agent))
}
