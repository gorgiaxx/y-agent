//! Skill management endpoints.
//!
//! Mirrors skill-related Tauri commands from the GUI (except `skill_open_folder`
//! which is desktop-only).

use std::path::{Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use y_service::SkillService;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

pub type SkillInfo = y_service::SkillInfo;
pub type SkillDetail = y_service::SkillDetail;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SetEnabledRequest {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct SaveFileRequest {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportSkillRequest {
    pub path: String,
    #[serde(default = "default_import_sanitize")]
    pub sanitize: bool,
}

fn default_import_sanitize() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct CreateSkillRequest {
    pub request: String,
    #[serde(default)]
    pub domain_hints: Option<Vec<String>>,
    #[serde(default)]
    pub language: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn skills_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/skills`
async fn list_skills(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let skills = svc.list().await.map_err(ApiError::Internal)?;
    Ok(Json(skills))
}

/// `GET /api/v1/skills/:name`
async fn get_skill(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let detail = svc.get(&name).await.map_err(ApiError::NotFound)?;
    Ok(Json(detail))
}

/// `DELETE /api/v1/skills/:name`
async fn uninstall_skill(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.uninstall(&name).await.map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({"message": "uninstalled"})))
}

/// `PUT /api/v1/skills/:name/enabled`
async fn set_enabled(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<SetEnabledRequest>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.set_enabled(&name, body.enabled)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({"message": "updated"})))
}

/// `GET /api/v1/skills/:name/files`
async fn get_files(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let tree = svc.file_tree(&name).await.map_err(ApiError::NotFound)?;
    Ok(Json(tree))
}

/// `GET /api/v1/skills/:name/files/*path`
async fn read_file(
    State(state): State<AppState>,
    AxumPath((name, relative_path)): AxumPath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    let content = SkillService::new(&skills_store_path(&state.config_dir))
        .read_file(&name, Path::new(&relative_path))
        .await
        .map_err(ApiError::BadRequest)?;

    Ok(Json(serde_json::json!({ "content": content })))
}

/// `PUT /api/v1/skills/:name/files/*path`
async fn save_file(
    State(state): State<AppState>,
    AxumPath((name, relative_path)): AxumPath<(String, String)>,
    Json(body): Json<SaveFileRequest>,
) -> Result<impl IntoResponse, ApiError> {
    SkillService::validate_name(&name).map_err(ApiError::BadRequest)?;
    SkillService::new(&skills_store_path(&state.config_dir))
        .write_file(&name, Path::new(&relative_path), &body.content)
        .await
        .map_err(ApiError::BadRequest)?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `POST /api/v1/skills/import` -- import a skill from a local source path.
///
/// Mirrors the Tauri `skill_import` command. Trusted TOML skills can be
/// imported directly with `sanitize=false`; all other imports go through the
/// agent-assisted ingestion service, optionally after security screening.
async fn import_skill(
    State(state): State<AppState>,
    Json(body): Json<ImportSkillRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let skills_dir = skills_store_path(&state.config_dir);
    let source_path = PathBuf::from(&body.path);
    let result = state
        .container
        .import_skill_from_path(&skills_dir, &source_path, body.sanitize)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(result))
}

/// `POST /api/v1/skills/create` -- create a skill from a description.
async fn create_skill(
    State(state): State<AppState>,
    Json(body): Json<CreateSkillRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let skills_dir = skills_store_path(&state.config_dir);
    let result = state
        .container
        .create_skill(
            &skills_dir,
            &body.request,
            body.domain_hints.as_deref(),
            body.language.as_deref(),
        )
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(result))
}

/// `GET /api/v1/skills/validate` -- validate all installed skills.
async fn validate_skills(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let results = SkillService::new(&skills_store_path(&state.config_dir))
        .validate_all()
        .map_err(ApiError::Internal)?;
    Ok(Json(results))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Skills route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/skills", get(list_skills))
        .route("/api/v1/skills/import", post(import_skill))
        .route("/api/v1/skills/create", post(create_skill))
        .route("/api/v1/skills/validate", get(validate_skills))
        .route(
            "/api/v1/skills/{name}",
            get(get_skill).delete(uninstall_skill),
        )
        .route("/api/v1/skills/{name}/enabled", put(set_enabled))
        .route("/api/v1/skills/{name}/files", get(get_files))
        .route(
            "/api/v1/skills/{name}/files/{*path}",
            get(read_file).put(save_file),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_dir_path_accepts_plain_skill_name() {
        let base = PathBuf::from("/tmp/y-agent/skills");
        let path = SkillService::new(&base).skill_directory("writer").unwrap();

        assert_eq!(path, base.join("writer"));
    }

    #[test]
    fn test_skill_dir_path_rejects_parent_directory_name() {
        let base = PathBuf::from("/tmp/y-agent/skills");
        let error = SkillService::new(&base).skill_directory("..").unwrap_err();

        assert!(error.contains("Invalid skill name"));
    }
}
