//! Agent listing endpoint.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::state::AppState;

/// Agent info returned in the list.
#[derive(Debug, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    pub trust_tier: String,
    pub capabilities: Vec<String>,
    pub is_overridden: bool,
}

/// Full agent detail.
#[derive(Debug, Serialize)]
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
    pub is_overridden: bool,
}

/// `GET /api/v1/agents` — list registered agent definitions.
async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentInfo>> {
    let registry = state.container.agent_registry.lock().await;
    let agents: Vec<AgentInfo> = registry
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
    Json(agents)
}

/// `GET /api/v1/agents/:id` — get a single agent definition.
async fn get_agent(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<AgentDetail>, (axum::http::StatusCode, String)> {
    let registry = state.container.agent_registry.lock().await;
    let def = registry
        .get(&id)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Agent not found: {id}")))?;

    Ok(Json(AgentDetail {
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
        is_overridden: registry.is_overridden(&def.id),
    }))
}

/// Agent route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/agents", get(list_agents))
        .route("/api/v1/agents/{id}", get(get_agent))
}
