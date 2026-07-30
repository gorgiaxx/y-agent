//! Skill management command handlers -- list, get, uninstall, enable/disable,
//! open folder, import, file tree, read/save file.
//!
//! CRUD operations (`skill_list`, `skill_get`, `skill_uninstall`,
//! `skill_set_enabled`) delegate to [`y_service::SkillService`].
//! Presentation-only commands (open folder, file tree, read/save) remain here.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};

use y_service::{SkillFileEntry, SkillService, SkillValidationResult};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Skill summary info returned to the frontend.
pub type SkillInfo = y_service::SkillInfo;

/// Full skill detail returned to the frontend.
pub type SkillDetail = y_service::SkillDetail;

/// Result of a skill import operation.
pub type SkillImportResult = y_service::SkillImportOutcome;

/// Result of a skill creation operation.
pub type SkillCreateResult = y_service::SkillCreateOutcome;

/// Resolve the base path of the skill store.
fn skills_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all installed skills.
#[tauri::command]
pub async fn skill_list(state: State<'_, AppState>) -> Result<Vec<SkillInfo>, String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.list().await
}

/// Get full detail for a single skill.
#[tauri::command]
pub async fn skill_get(state: State<'_, AppState>, name: String) -> Result<SkillDetail, String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.get(&name).await
}

/// Uninstall (delete) a skill.
#[tauri::command]
pub async fn skill_uninstall(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.uninstall(&name).await
}

/// Enable or disable a skill.
#[tauri::command]
pub async fn skill_set_enabled(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.set_enabled(&name, enabled).await
}

/// Open a skill's directory in the system file manager.
#[tauri::command]
pub async fn skill_open_folder(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    let dir = svc.skill_directory(&name)?;
    if !dir.exists() {
        return Err(format!("Skill directory not found: {}", dir.display()));
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }

    Ok(())
}

/// Import a skill from a file path. When `sanitize` is true, runs the
/// `skill-security-check` agent before ingestion. Non-TOML formats always
/// use agent-assisted ingestion but the security screening is only performed
/// when the user explicitly enables it.
#[tauri::command]
pub async fn skill_import(
    _app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    sanitize: bool,
) -> Result<SkillImportResult, String> {
    let store_path = skills_store_path(&state.config_dir);
    state
        .container
        .import_skill_from_path(&store_path, Path::new(&path), sanitize)
        .await
}

/// Create a new skill from a natural-language description. Delegates to the
/// `skill-creator` agent, which generates the skill content and metadata,
/// then registers the result in the skill store.
#[tauri::command]
pub async fn skill_create(
    _app: AppHandle,
    state: State<'_, AppState>,
    request: String,
    domain_hints: Option<Vec<String>>,
    language: Option<String>,
) -> Result<SkillCreateResult, String> {
    let store_path = skills_store_path(&state.config_dir);
    state
        .container
        .create_skill(
            &store_path,
            &request,
            domain_hints.as_deref(),
            language.as_deref(),
        )
        .await
}

/// Get the file tree of a skill directory.
#[tauri::command]
pub async fn skill_get_files(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<SkillFileEntry>, String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.file_tree(&name).await
}

/// Read a file within a skill directory.
#[tauri::command]
pub async fn skill_read_file(
    state: State<'_, AppState>,
    name: String,
    relative_path: String,
) -> Result<String, String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.read_file(&name, Path::new(&relative_path)).await
}

/// Save edits to a file within a skill directory.
#[tauri::command]
pub async fn skill_save_file(
    state: State<'_, AppState>,
    name: String,
    relative_path: String,
    content: String,
) -> Result<(), String> {
    let svc = SkillService::new(&skills_store_path(&state.config_dir));
    svc.write_file(&name, Path::new(&relative_path), &content)
        .await
}

/// Validate all installed skills.
///
/// Runs the skill validator on every skill in the store and returns
/// per-skill results with any validation errors.
#[tauri::command]
pub async fn skill_validate(
    state: State<'_, AppState>,
) -> Result<Vec<SkillValidationResult>, String> {
    SkillService::new(&skills_store_path(&state.config_dir)).validate_all()
}
