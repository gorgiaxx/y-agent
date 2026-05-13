//! Skill management endpoints.
//!
//! Mirrors skill-related Tauri commands from the GUI (except `skill_open_folder`
//! which is desktop-only).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::agent::ContextStrategyHint;
use y_core::skill::SkillRegistry;
use y_service::SkillService;
use y_skills::{
    FilesystemSkillStore, FormatDetector, IngestionFormat, ManifestParser, SkillConfig,
    SkillRegistryImpl,
};

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

#[derive(Debug, Deserialize)]
pub struct ImportSkillRequest {
    pub path: String,
    #[serde(default = "default_import_sanitize")]
    pub sanitize: bool,
}

#[derive(Debug, Serialize)]
pub struct SkillImportResult {
    pub decision: String,
    pub classification: String,
    pub skill_id: Option<String>,
    pub error: Option<String>,
    pub security_issues: Vec<String>,
    pub permissions_needed: Option<PermissionsNeeded>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PermissionsNeeded {
    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub files_write: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SecurityOutput {
    verdict: String,
    #[serde(default)]
    issues: Vec<String>,
    #[serde(default)]
    risk_level: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    permissions_needed: Option<PermissionsNeeded>,
}

fn default_import_sanitize() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn skills_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

fn validate_skill_name(name: &str) -> Result<(), ApiError> {
    let is_plain_name = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));

    if is_plain_name {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("Invalid skill name: {name}")))
    }
}

fn skill_dir_path(skills_dir: &Path, name: &str) -> Result<PathBuf, ApiError> {
    validate_skill_name(name)?;
    Ok(skills_dir.join(name))
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
    validate_skill_name(&name)?;
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let detail = svc.get(&name).await.map_err(ApiError::NotFound)?;
    Ok(Json(detail))
}

/// `DELETE /api/v1/skills/:name`
async fn uninstall_skill(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_skill_name(&name)?;
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
    validate_skill_name(&name)?;
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
    let skill_dir = skill_dir_path(&skills_store_path(&state.config_dir), &name)?;
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
    let skill_dir = skill_dir_path(&skills_store_path(&state.config_dir), &name)?;
    let canonical_target =
        y_service::resolve_skill_read_path(&skill_dir, Path::new(&relative_path))
            .map_err(ApiError::BadRequest)?;

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
    let skill_dir = skill_dir_path(&skills_store_path(&state.config_dir), &name)?;
    let target = y_service::resolve_skill_write_path(&skill_dir, Path::new(&relative_path))
        .map_err(ApiError::BadRequest)?;

    tokio::fs::write(&target, &body.content)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to write file: {e}")))?;

    Ok(Json(serde_json::json!({"message": "saved"})))
}

async fn import_toml_skill(
    store_path: &Path,
    source_path: &Path,
) -> Result<SkillImportResult, ApiError> {
    let config = SkillConfig::default();
    let content = tokio::fs::read_to_string(source_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read file: {e}")))?;
    let parser = ManifestParser::new(config);
    let manifest = parser
        .parse(&content)
        .map_err(|e| ApiError::BadRequest(format!("Failed to parse skill: {e}")))?;

    validate_skill_name(&manifest.name)?;
    let skill_name = manifest.name.clone();

    let store = FilesystemSkillStore::new(store_path)
        .map_err(|e| ApiError::Internal(format!("Failed to open skill store: {e}")))?;
    let registry = SkillRegistryImpl::with_store(store)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create registry: {e}")))?;

    registry
        .register(manifest)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to register skill: {e}")))?;

    Ok(SkillImportResult {
        decision: "accepted".to_string(),
        classification: "direct_import".to_string(),
        skill_id: Some(skill_name),
        error: None,
        security_issues: vec![],
        permissions_needed: None,
    })
}

async fn security_rejection(
    state: &AppState,
    source_path: &Path,
) -> Result<Option<SkillImportResult>, ApiError> {
    if source_path.is_dir() {
        return Ok(None);
    }

    let source_content = tokio::fs::read_to_string(source_path)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to read file: {e}")))?;
    let format_str = match source_path.extension().and_then(|e| e.to_str()) {
        Some("toml") => "toml",
        Some("md" | "markdown") => "markdown",
        Some("yaml" | "yml") => "yaml",
        Some("json") => "json",
        _ => "plaintext",
    };

    let security_input = serde_json::json!({
        "source_content": source_content,
        "source_format": format_str,
    });

    let security_result = state
        .container
        .agent_delegator
        .delegate(
            "skill-security-check",
            security_input,
            ContextStrategyHint::None,
            None,
        )
        .await;

    let output = match security_result {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!(error = %e, "Skill security check failed; proceeding with ingestion");
            return Ok(None);
        }
    };

    let Ok(security) = serde_json::from_str::<SecurityOutput>(&output.text) else {
        tracing::warn!("Skill security check returned invalid JSON; proceeding with ingestion");
        return Ok(None);
    };

    match security.verdict.as_str() {
        "insecure" => Ok(Some(SkillImportResult {
            decision: "rejected".to_string(),
            classification: String::new(),
            skill_id: None,
            error: Some(format!(
                "Security check failed ({}): {}",
                security.risk_level, security.summary
            )),
            security_issues: security.issues,
            permissions_needed: security.permissions_needed,
        })),
        "caution" => {
            tracing::warn!(
                risk_level = %security.risk_level,
                summary = %security.summary,
                issues = ?security.issues,
                "Skill security check returned caution; proceeding with ingestion"
            );
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn import_result_from_service(result: y_service::ImportResult) -> SkillImportResult {
    let decision = match result.decision {
        y_service::ImportDecision::Accepted => "accepted",
        y_service::ImportDecision::Optimized => "optimized",
        y_service::ImportDecision::PartialAccept => "partial_accept",
        y_service::ImportDecision::Rejected => "rejected",
    };

    SkillImportResult {
        decision: decision.to_string(),
        classification: result.classification,
        skill_id: result.skill_id,
        error: result.rejection_reason,
        security_issues: result.security_issues,
        permissions_needed: None,
    }
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
    tokio::fs::create_dir_all(&skills_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create skills dir: {e}")))?;

    let source_path = PathBuf::from(&body.path);
    if !source_path.exists() {
        return Err(ApiError::NotFound(format!("Path not found: {}", body.path)));
    }

    let format = FormatDetector::from_path(&source_path);
    if !body.sanitize && format == IngestionFormat::Toml {
        return import_toml_skill(&skills_dir, &source_path).await.map(Json);
    }

    if body.sanitize {
        if let Some(rejection) = security_rejection(&state, &source_path).await? {
            return Ok(Json(rejection));
        }
    }

    let store = FilesystemSkillStore::new(&skills_dir)
        .map_err(|e| ApiError::Internal(format!("Failed to open skill store: {e}")))?;
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create registry: {e}")))?,
    ));
    let ingestion_service = state.container.skill_ingestion_service(registry);

    let result = ingestion_service.import(&source_path).await.map_or_else(
        |e| SkillImportResult {
            decision: "rejected".to_string(),
            classification: String::new(),
            skill_id: None,
            error: Some(e.to_string()),
            security_issues: vec![],
            permissions_needed: None,
        },
        import_result_from_service,
    );

    Ok(Json(result))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_skill_dir_path_accepts_plain_skill_name() {
        let base = PathBuf::from("/tmp/y-agent/skills");
        let path = skill_dir_path(&base, "writer").unwrap();

        assert_eq!(path, base.join("writer"));
    }

    #[test]
    fn test_skill_dir_path_rejects_parent_directory_name() {
        let base = PathBuf::from("/tmp/y-agent/skills");
        let error = skill_dir_path(&base, "..").unwrap_err();

        assert!(error.to_string().contains("Invalid skill name"));
    }

    #[tokio::test]
    async fn test_import_toml_skill_registers_with_filesystem_store() {
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("writer.toml");
        tokio::fs::write(
            &source,
            r#"
name = "writer"
description = "Writing helper"
version = "1.0.0"
root_content = "Use concise, concrete prose."
"#,
        )
        .await
        .unwrap();

        let result = import_toml_skill(dir.path(), &source).await.unwrap();

        assert_eq!(result.decision, "accepted");
        assert_eq!(result.classification, "direct_import");
        assert_eq!(result.skill_id.as_deref(), Some("writer"));
        assert!(dir.path().join("writer").join("skill.toml").exists());
        assert!(dir.path().join("writer").join("root.md").exists());
    }
}
