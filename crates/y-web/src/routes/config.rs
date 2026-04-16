//! Configuration management endpoints.
//!
//! Mirrors all config-related Tauri commands: section CRUD, config reload,
//! provider testing, model listing, MCP config, and prompt file management.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ALLOWED_SECTIONS: &[&str] = &[
    "providers",
    "storage",
    "session",
    "runtime",
    "hooks",
    "tools",
    "guardrails",
    "browser",
    "knowledge",
];

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Request body for saving a config section (raw TOML).
#[derive(Debug, Deserialize)]
pub struct SaveSectionRequest {
    pub content: String,
}

/// Request body for provider testing.
#[derive(Debug, Deserialize)]
pub struct ProviderTestRequest {
    pub provider_type: String,
    pub model: String,
    pub api_key: String,
    pub api_key_env: String,
    pub base_url: Option<String>,
}

/// Request body for listing models.
#[derive(Debug, Deserialize)]
pub struct ListModelsRequest {
    pub base_url: String,
    pub api_key: String,
    pub api_key_env: String,
}

/// Request body for saving a prompt file.
#[derive(Debug, Deserialize)]
pub struct SavePromptRequest {
    pub content: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/config` -- get the full configuration as JSON.
async fn config_get(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let config_dir = &state.config_dir;
    let mut merged = serde_json::Map::new();

    for section in ALLOWED_SECTIONS {
        let path = config_dir.join(format!("{section}.toml"));
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| ApiError::Internal(format!("Failed to read {section}.toml: {e}")))?;
            let value: Value = toml::from_str(&content)
                .map_err(|e| ApiError::Internal(format!("Failed to parse {section}.toml: {e}")))?;
            merged.insert((*section).to_string(), value);
        }
    }

    Ok(Json(Value::Object(merged)))
}

/// `GET /api/v1/config/:section` -- get a section as raw TOML.
async fn config_get_section(
    State(state): State<AppState>,
    Path(section): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if !ALLOWED_SECTIONS.contains(&section.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "Unknown config section: {section}"
        )));
    }

    let path = state.config_dir.join(format!("{section}.toml"));
    if !path.exists() {
        return Ok(Json(serde_json::json!({ "content": "" })));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| ApiError::Internal(format!("Failed to read {section}.toml: {e}")))?;

    Ok(Json(serde_json::json!({ "content": content })))
}

/// `PUT /api/v1/config/:section` -- save a section from raw TOML.
async fn config_save_section(
    State(state): State<AppState>,
    Path(section): Path<String>,
    Json(body): Json<SaveSectionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !ALLOWED_SECTIONS.contains(&section.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "Unknown config section: {section}"
        )));
    }

    // Validate TOML syntax.
    let _: Value = toml::from_str(&body.content)
        .map_err(|e| ApiError::BadRequest(format!("Invalid TOML syntax: {e}")))?;

    let path = state.config_dir.join(format!("{section}.toml"));
    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to create config dir: {e}")))?;

    std::fs::write(&path, &body.content)
        .map_err(|e| ApiError::Internal(format!("Failed to write {section}.toml: {e}")))?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `POST /api/v1/config/reload` -- hot-reload configuration.
async fn config_reload(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let mut results: Vec<String> = Vec::new();

    let read_toml = |name: &str| -> Result<Option<String>, ApiError> {
        let path = state.config_dir.join(format!("{name}.toml"));
        if !path.exists() {
            return Ok(None);
        }
        std::fs::read_to_string(&path)
            .map(Some)
            .map_err(|e| ApiError::Internal(format!("Failed to read {name}.toml: {e}")))
    };

    if let Some(content) = read_toml("providers")? {
        let count =
            y_service::SystemService::reload_providers_from_toml(&state.container, &content)
                .await
                .map_err(ApiError::Internal)?;
        results.push(format!("{count} provider(s)"));
    }

    if let Some(content) = read_toml("guardrails")? {
        y_service::SystemService::reload_guardrails_from_toml(&state.container, &content)
            .map_err(ApiError::Internal)?;
        results.push("guardrails".to_string());
    }

    if let Some(content) = read_toml("session")? {
        y_service::SystemService::reload_session_from_toml(&state.container, &content)
            .map_err(ApiError::Internal)?;
        results.push("session".to_string());
    }

    if let Some(content) = read_toml("runtime")? {
        y_service::SystemService::reload_runtime_from_toml(&state.container, &content)
            .map_err(ApiError::Internal)?;
        results.push("runtime".to_string());
    }

    if let Some(content) = read_toml("browser")? {
        y_service::SystemService::reload_browser_from_toml(&state.container, &content)
            .await
            .map_err(ApiError::Internal)?;
        results.push("browser".to_string());
    }

    if let Some(content) = read_toml("tools")? {
        y_service::SystemService::reload_tools_from_toml(&state.container, &content)
            .map_err(ApiError::Internal)?;
        results.push("tools".to_string());
    }

    if let Some(content) = read_toml("knowledge")? {
        y_service::SystemService::reload_knowledge_from_toml(&state.container, &content)
            .await
            .map_err(ApiError::Internal)?;
        results.push("knowledge".to_string());
    }

    if let Some(content) = read_toml("hooks")? {
        y_service::SystemService::reload_hooks_from_toml(&state.container, &content)
            .map_err(ApiError::Internal)?;
        results.push("hooks".to_string());
    }

    y_service::SystemService::reload_prompts(&state.container).await;
    results.push("prompts".to_string());

    let (loaded, errored) = y_service::SystemService::reload_agents(&state.container).await;
    if errored > 0 {
        results.push(format!("{loaded} agent(s), {errored} error(s)"));
    } else {
        results.push(format!("{loaded} agent(s)"));
    }

    let summary = if results.is_empty() {
        "Config reloaded (no config files to update)".to_string()
    } else {
        format!("Config reloaded: {}", results.join(", "))
    };

    Ok(Json(serde_json::json!({"message": summary})))
}

/// `POST /api/v1/providers/test` -- test a provider configuration.
async fn provider_test(
    Json(body): Json<ProviderTestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let result = y_service::SystemService::test_provider(y_service::ProviderTestRequest {
        provider_type: body.provider_type,
        model: body.model,
        api_key: body.api_key,
        api_key_env: body.api_key_env,
        base_url: body.base_url,
    })
    .await
    .map_err(ApiError::Internal)?;

    Ok(Json(serde_json::json!({"result": result})))
}

/// `POST /api/v1/providers/list-models` -- fetch available models from an endpoint.
async fn provider_list_models(
    Json(body): Json<ListModelsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let effective_key = if !body.api_key.is_empty() {
        body.api_key
    } else if !body.api_key_env.is_empty() {
        std::env::var(&body.api_key_env).map_err(|_| {
            ApiError::BadRequest(format!(
                "Environment variable '{}' is not set",
                body.api_key_env
            ))
        })?
    } else {
        String::new()
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ApiError::Internal(format!("Failed to build HTTP client: {e}")))?;

    let url = format!("{}/models", body.base_url.trim_end_matches('/'));
    let mut req = client.get(&url);
    if !effective_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {effective_key}"));
    }

    let response = req
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("Network error reaching {url}: {e}")))?;

    let status = response.status();
    let response_body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(ApiError::Internal(format!(
            "HTTP {status}: {response_body}"
        )));
    }

    let value: Value = serde_json::from_str(&response_body)
        .map_err(|e| ApiError::Internal(format!("Failed to parse response: {e}")))?;
    Ok(Json(value))
}

/// `GET /api/v1/config/mcp` -- get MCP server configuration.
async fn mcp_config_get(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let path = state.config_dir.join("mcp.json");
    if !path.exists() {
        return Ok(Json(serde_json::json!({"mcpServers": {}})));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| ApiError::Internal(format!("Failed to read mcp.json: {e}")))?;
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| ApiError::Internal(format!("Failed to parse mcp.json: {e}")))?;
    Ok(Json(value))
}

/// `PUT /api/v1/config/mcp` -- save MCP server configuration.
async fn mcp_config_save(
    State(state): State<AppState>,
    Json(content): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let json_str = serde_json::to_string_pretty(&content)
        .map_err(|e| ApiError::Internal(format!("Failed to serialize MCP config: {e}")))?;

    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to create config dir: {e}")))?;

    let path = state.config_dir.join("mcp.json");
    std::fs::write(&path, &json_str)
        .map_err(|e| ApiError::Internal(format!("Failed to write mcp.json: {e}")))?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `GET /api/v1/config/prompts` -- list all prompt files.
async fn prompt_list(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let prompts_dir = state.config_dir.join("prompts");
    if !prompts_dir.exists() {
        return Ok(Json(Vec::<String>::new()));
    }

    let mut files: Vec<String> = std::fs::read_dir(&prompts_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to read prompts directory: {e}")))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
                && entry.file_type().ok()?.is_file()
            {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    Ok(Json(files))
}

/// `GET /api/v1/config/prompts/:filename` -- read a prompt file.
async fn prompt_get(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(ApiError::BadRequest("Invalid filename".into()));
    }

    let path = state.config_dir.join("prompts").join(&filename);
    if !path.exists() {
        return Ok(Json(serde_json::json!({ "content": "" })));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| ApiError::Internal(format!("Failed to read {filename}: {e}")))?;

    Ok(Json(serde_json::json!({ "content": content })))
}

/// `PUT /api/v1/config/prompts/:filename` -- save a prompt file.
async fn prompt_save(
    State(state): State<AppState>,
    Path(filename): Path<String>,
    Json(body): Json<SavePromptRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(ApiError::BadRequest("Invalid filename".into()));
    }

    let prompts_dir = state.config_dir.join("prompts");
    std::fs::create_dir_all(&prompts_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to create prompts dir: {e}")))?;

    std::fs::write(prompts_dir.join(&filename), &body.content)
        .map_err(|e| ApiError::Internal(format!("Failed to write {filename}: {e}")))?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `GET /api/v1/config/prompts/:filename/default` -- get built-in default.
async fn prompt_get_default(Path(filename): Path<String>) -> Result<impl IntoResponse, ApiError> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(ApiError::BadRequest("Invalid filename".into()));
    }

    for &(name, content) in y_prompt::BUILTIN_PROMPT_FILES {
        if name == filename {
            return Ok(Json(serde_json::json!({ "content": content })));
        }
    }

    Err(ApiError::NotFound(format!(
        "No built-in default for: {filename}"
    )))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Config route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/config", get(config_get))
        .route("/api/v1/config/reload", post(config_reload))
        .route(
            "/api/v1/config/mcp",
            get(mcp_config_get).put(mcp_config_save),
        )
        .route("/api/v1/config/prompts", get(prompt_list))
        .route(
            "/api/v1/config/prompts/{filename}",
            get(prompt_get).put(prompt_save),
        )
        .route(
            "/api/v1/config/prompts/{filename}/default",
            get(prompt_get_default),
        )
        .route(
            "/api/v1/config/{section}",
            get(config_get_section).put(config_save_section),
        )
        .route("/api/v1/providers/test", post(provider_test))
        .route("/api/v1/providers/list-models", post(provider_list_models))
}
