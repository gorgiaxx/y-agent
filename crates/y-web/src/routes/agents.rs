//! Agent management endpoints.
//!
//! Mirrors all agent-related Tauri commands from the GUI.

use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use y_service::AgentManagementService;

use crate::error::ApiError;
use crate::state::AppState;

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
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/agents` -- list registered agent definitions.
async fn list_agents(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(
        AgentManagementService::list_agents(&state.container).await,
    ))
}

/// `GET /api/v1/agents/:id` -- get a single agent definition.
async fn get_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let detail = AgentManagementService::get_agent(&state.container, &id)
        .await
        .map_err(ApiError::NotFound)?;
    Ok(Json(detail))
}

/// `GET /api/v1/agents/:id/source` -- get the raw TOML source.
async fn get_agent_source(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let source = AgentManagementService::get_agent_source_info(&state.container, &id)
        .await
        .map_err(ApiError::NotFound)?;
    Ok(Json(source))
}

/// `POST /api/v1/agents/parse-toml` -- parse raw agent TOML.
async fn parse_toml(Json(body): Json<ParseTomlRequest>) -> Result<impl IntoResponse, ApiError> {
    let detail = AgentManagementService::parse_agent_toml(&body.toml_content)
        .map_err(ApiError::BadRequest)?;
    Ok(Json(detail))
}

/// `PUT /api/v1/agents/:id` -- save (create or update) a user agent definition.
async fn save_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SaveAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .container
        .save_agent(&id, &body.toml_content)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `POST /api/v1/agents/:id/reset` -- reset an overridden built-in agent.
async fn reset_agent(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .container
        .reset_agent(&id)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(serde_json::json!({"message": "reset"})))
}

/// `POST /api/v1/agents/reload` -- reload all user-defined agents.
async fn reload_agents(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let (loaded, errored) = state.container.reload_agents().await;
    Ok(Json(
        serde_json::json!({"message": "reloaded", "loaded": loaded, "errored": errored}),
    ))
}

/// `GET /api/v1/agents/tools` -- list all registered tool definitions.
async fn list_tools(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(
        AgentManagementService::list_tools(&state.container).await,
    ))
}

/// `GET /api/v1/agents/prompt-sections` -- list built-in prompt sections.
async fn list_prompt_sections(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(AgentManagementService::list_prompt_sections(
        &state.config_dir,
    )))
}

/// `POST /api/v1/agents/translate` -- translate text using the translator agent.
async fn translate_text(
    State(state): State<AppState>,
    Json(body): Json<TranslateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let text = AgentManagementService::translate_text(&state.container, body.text)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({ "text": text })))
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
