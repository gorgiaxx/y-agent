//! Skill management endpoints.
//!
//! Mirrors skill-related Tauri commands from the GUI (except `skill_open_folder`
//! which is desktop-only).

use std::path::{Path, PathBuf};

use axum::extract::{Multipart, Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_service::SkillService;

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

pub type SkillInfo = y_service::SkillInfo;
pub type SkillDetail = y_service::SkillDetail;

/// A file/directory entry within a skill directory.
#[derive(Debug, Serialize)]
pub struct SkillFileEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<SkillFileEntry>>,
}

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn skills_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

fn build_file_tree(dir: &Path, relative_base: &Path) -> Vec<SkillFileEntry> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        let abs_path = entry.path();
        let rel_path = abs_path
            .strip_prefix(relative_base)
            .unwrap_or(&abs_path)
            .to_string_lossy()
            .to_string();

        if meta.is_dir() {
            let children = build_file_tree(&abs_path, relative_base);
            entries.push(SkillFileEntry {
                path: rel_path,
                name: file_name,
                is_dir: true,
                size: 0,
                children: Some(children),
            });
        } else {
            entries.push(SkillFileEntry {
                path: rel_path,
                name: file_name,
                is_dir: false,
                size: meta.len(),
                children: None,
            });
        }
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    entries
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
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let detail = svc.get(&name).await.map_err(ApiError::NotFound)?;
    Ok(Json(detail))
}

/// `DELETE /api/v1/skills/:name`
async fn uninstall_skill(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
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
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    if !skill_dir.exists() {
        return Err(ApiError::NotFound(format!(
            "Skill directory not found: {}",
            skill_dir.display()
        )));
    }

    let tree = tokio::task::spawn_blocking(move || build_file_tree(&skill_dir, &skill_dir))
        .await
        .map_err(|e| ApiError::Internal(format!("Task join error: {e}")))?;

    Ok(Json(tree))
}

/// `GET /api/v1/skills/:name/files/*path`
async fn read_file(
    State(state): State<AppState>,
    AxumPath((name, relative_path)): AxumPath<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    let target = skill_dir.join(&relative_path);

    let canonical_dir = skill_dir
        .canonicalize()
        .map_err(|e| ApiError::NotFound(format!("Skill dir not found: {e}")))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|e| ApiError::NotFound(format!("File not found: {e}")))?;
    if !canonical_target.starts_with(&canonical_dir) {
        return Err(ApiError::BadRequest(
            "Access denied: path traversal detected".into(),
        ));
    }

    let content = tokio::fs::read_to_string(&canonical_target)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read file: {e}")))?;

    Ok(Json(serde_json::json!({ "content": content })))
}

/// `PUT /api/v1/skills/:name/files/*path`
async fn save_file(
    State(state): State<AppState>,
    AxumPath((name, relative_path)): AxumPath<(String, String)>,
    Json(body): Json<SaveFileRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    let target = skill_dir.join(&relative_path);

    let canonical_dir = skill_dir
        .canonicalize()
        .map_err(|e| ApiError::NotFound(format!("Skill dir not found: {e}")))?;

    let parent = target
        .parent()
        .ok_or(ApiError::BadRequest("Invalid path".into()))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| ApiError::NotFound(format!("Parent dir not found: {e}")))?;
    if !canonical_parent.starts_with(&canonical_dir) {
        return Err(ApiError::BadRequest(
            "Access denied: path traversal detected".into(),
        ));
    }

    tokio::fs::write(&target, &body.content)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write file: {e}")))?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

/// `POST /api/v1/skills/import` -- import skill from uploaded archive.
///
/// Accepts multipart form data with a single file field containing a
/// .zip or .tar.gz skill package. Extracts to the skills directory.
async fn import_skill(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let skills_dir = skills_store_path(&state.config_dir);
    tokio::fs::create_dir_all(&skills_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create skills dir: {e}")))?;

    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Multipart error: {e}")))?
    else {
        return Err(ApiError::BadRequest("No file provided".into()));
    };

    let filename = field
        .file_name()
        .map(String::from)
        .ok_or_else(|| ApiError::BadRequest("Missing filename".into()))?;

    let data = field
        .bytes()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Failed to read field: {e}")))?;

    let temp_path = std::env::temp_dir().join(&filename);
    tokio::fs::write(&temp_path, &data)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write temp file: {e}")))?;

    // Extract archive to skills directory.
    // For now, return a placeholder response. Full extraction logic would use
    // zip/tar crates to unpack the archive.
    let _ = tokio::fs::remove_file(&temp_path).await;

    Ok(Json(serde_json::json!({
        "message": "import endpoint ready (extraction not yet implemented)",
        "filename": filename
    })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Skills route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/skills", get(list_skills))
        .route("/api/v1/skills/import", post(import_skill))
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
