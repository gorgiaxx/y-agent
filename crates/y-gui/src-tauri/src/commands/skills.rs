//! Skill management command handlers -- list, get, uninstall, enable/disable,
//! open folder, import, file tree, read/save file.
//!
//! CRUD operations (`skill_list`, `skill_get`, `skill_uninstall`,
//! `skill_set_enabled`) delegate to [`y_service::SkillService`].
//! Presentation-only commands (open folder, file tree, read/save) remain here.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};

use y_service::SkillService;

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

/// A file/directory entry within a skill directory.
#[derive(Debug, Serialize, Clone)]
pub struct SkillFileEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<SkillFileEntry>>,
}

/// Resolve the base path of the skill store.
fn skills_store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("skills")
}

// ---------------------------------------------------------------------------
// Helper: build file tree recursively
// ---------------------------------------------------------------------------

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

    // Sort: directories first, then files, alphabetically.
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    entries
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
    let dir = skills_store_path(&state.config_dir).join(&name);
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

/// Get the file tree of a skill directory.
#[tauri::command]
pub async fn skill_get_files(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<SkillFileEntry>, String> {
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    if !skill_dir.exists() {
        return Err(format!(
            "Skill directory not found: {}",
            skill_dir.display()
        ));
    }

    Ok(build_file_tree(&skill_dir, &skill_dir))
}

/// Read a file within a skill directory.
#[tauri::command]
pub async fn skill_read_file(
    state: State<'_, AppState>,
    name: String,
    relative_path: String,
) -> Result<String, String> {
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    let canonical_target =
        y_service::resolve_skill_read_path(&skill_dir, Path::new(&relative_path))?;

    std::fs::read_to_string(&canonical_target).map_err(|e| format!("Failed to read file: {e}"))
}

/// Save edits to a file within a skill directory.
#[tauri::command]
pub async fn skill_save_file(
    state: State<'_, AppState>,
    name: String,
    relative_path: String,
    content: String,
) -> Result<(), String> {
    let skill_dir = skills_store_path(&state.config_dir).join(&name);
    let target = y_service::resolve_skill_write_path(&skill_dir, Path::new(&relative_path))?;

    std::fs::write(&target, content).map_err(|e| format!("Failed to write file: {e}"))
}
