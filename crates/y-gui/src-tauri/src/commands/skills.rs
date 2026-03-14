//! Skill management command handlers — list, get detail, uninstall, enable/disable, open folder.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use y_skills::FilesystemSkillStore;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Skill summary info returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub enabled: bool,
}

/// Full skill detail returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub enabled: bool,
    pub root_content: String,
    pub author: Option<String>,
    pub classification_type: Option<String>,
    pub dir_path: String,
}

// ---------------------------------------------------------------------------
// Disabled-skills persistence
// ---------------------------------------------------------------------------

/// Path to the disabled-skills JSON file.
fn disabled_skills_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("disabled_skills.json")
}

/// Read the set of disabled skill names from disk.
fn read_disabled_skills(config_dir: &std::path::Path) -> std::collections::HashSet<String> {
    let path = disabled_skills_path(config_dir);
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str::<Vec<String>>(&content)
            .unwrap_or_default()
            .into_iter()
            .collect()
    } else {
        std::collections::HashSet::new()
    }
}

/// Write the set of disabled skill names to disk.
fn write_disabled_skills(
    config_dir: &std::path::Path,
    disabled: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let path = disabled_skills_path(config_dir);
    let list: Vec<&String> = disabled.iter().collect();
    let content =
        serde_json::to_string_pretty(&list).map_err(|e| format!("Failed to serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("Failed to write disabled_skills.json: {e}"))
}

/// Resolve the base path of the skill store.
fn skills_store_path(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("skills")
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// List all installed skills.
#[tauri::command]
pub async fn skill_list(state: State<'_, AppState>) -> Result<Vec<SkillInfo>, String> {
    let store_path = skills_store_path(&state.config_dir);
    if !store_path.exists() {
        return Ok(vec![]);
    }

    let store = FilesystemSkillStore::new(&store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;

    let manifests = store
        .load_all()
        .map_err(|e| format!("Failed to load skills: {e}"))?;

    let disabled = read_disabled_skills(&state.config_dir);

    let mut infos: Vec<SkillInfo> = manifests
        .into_iter()
        .map(|m| SkillInfo {
            name: m.name.clone(),
            description: m.description.clone(),
            version: m.version.0.clone(),
            tags: m.tags.clone(),
            enabled: !disabled.contains(&m.name),
        })
        .collect();

    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
}

/// Get full detail for a single skill.
#[tauri::command]
pub async fn skill_get(
    state: State<'_, AppState>,
    name: String,
) -> Result<SkillDetail, String> {
    let store_path = skills_store_path(&state.config_dir);
    let store = FilesystemSkillStore::new(&store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;

    let manifest = store
        .load_skill(&name)
        .map_err(|e| format!("Skill not found: {e}"))?;

    let disabled = read_disabled_skills(&state.config_dir);

    let classification_type = manifest
        .classification
        .as_ref()
        .map(|c| c.skill_type.to_string());

    Ok(SkillDetail {
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        version: manifest.version.0.clone(),
        tags: manifest.tags.clone(),
        enabled: !disabled.contains(&manifest.name),
        root_content: manifest.root_content.clone(),
        author: manifest.author.clone(),
        classification_type,
        dir_path: store_path.join(&manifest.name).to_string_lossy().to_string(),
    })
}

/// Uninstall (delete) a skill.
#[tauri::command]
pub async fn skill_uninstall(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let store_path = skills_store_path(&state.config_dir);
    let store = FilesystemSkillStore::new(&store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;

    store
        .delete_skill(&name)
        .map_err(|e| format!("Failed to uninstall skill: {e}"))?;

    // Also remove from disabled list if present.
    let mut disabled = read_disabled_skills(&state.config_dir);
    if disabled.remove(&name) {
        let _ = write_disabled_skills(&state.config_dir, &disabled);
    }

    Ok(())
}

/// Enable or disable a skill.
#[tauri::command]
pub async fn skill_set_enabled(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let mut disabled = read_disabled_skills(&state.config_dir);
    if enabled {
        disabled.remove(&name);
    } else {
        disabled.insert(name);
    }
    write_disabled_skills(&state.config_dir, &disabled)
}

/// Open a skill's directory in the system file manager.
#[tauri::command]
pub async fn skill_open_folder(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
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
