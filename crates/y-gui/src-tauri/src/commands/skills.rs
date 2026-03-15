//! Skill management command handlers — list, get detail, uninstall, enable/disable,
//! open folder, import, file tree, read/save file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use y_skills::{
    FilesystemSkillStore, FormatDetector, IngestionFormat, ManifestParser, SkillConfig,
    SkillRegistryImpl,
};

use y_core::skill::SkillRegistry;

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

/// Result of a skill import operation.
#[derive(Debug, Serialize, Clone)]
pub struct SkillImportResult {
    pub decision: String,
    pub classification: String,
    pub skill_id: Option<String>,
    pub error: Option<String>,
    pub security_issues: Vec<String>,
}

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

// ---------------------------------------------------------------------------
// Disabled-skills persistence
// ---------------------------------------------------------------------------

/// Path to the disabled-skills JSON file.
fn disabled_skills_path(config_dir: &Path) -> PathBuf {
    config_dir.join("disabled_skills.json")
}

/// Read the set of disabled skill names from disk.
fn read_disabled_skills(config_dir: &Path) -> std::collections::HashSet<String> {
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
    config_dir: &Path,
    disabled: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let path = disabled_skills_path(config_dir);
    let list: Vec<&String> = disabled.iter().collect();
    let content =
        serde_json::to_string_pretty(&list).map_err(|e| format!("Failed to serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("Failed to write disabled_skills.json: {e}"))
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
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
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
pub async fn skill_get(state: State<'_, AppState>, name: String) -> Result<SkillDetail, String> {
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
        dir_path: store_path
            .join(&manifest.name)
            .to_string_lossy()
            .to_string(),
    })
}

/// Uninstall (delete) a skill.
#[tauri::command]
pub async fn skill_uninstall(state: State<'_, AppState>, name: String) -> Result<(), String> {
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
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    sanitize: bool,
) -> Result<SkillImportResult, String> {
    let store_path = skills_store_path(&state.config_dir);
    std::fs::create_dir_all(&store_path)
        .map_err(|e| format!("Failed to create skills directory: {e}"))?;

    let source_path = Path::new(&path);
    if !source_path.exists() {
        return Err(format!("Path not found: {path}"));
    }

    let format = FormatDetector::from_path(source_path);

    // ---------------------------------------------------------------
    // Path A: Direct TOML parsing (only when sanitize=false AND .toml)
    // ---------------------------------------------------------------
    if !sanitize && format == IngestionFormat::Toml {
        let config = SkillConfig::default();
        let content = std::fs::read_to_string(source_path)
            .map_err(|e| format!("Failed to read file: {e}"))?;
        let parser = ManifestParser::new(config);
        let manifest = parser
            .parse(&content)
            .map_err(|e| format!("Failed to parse skill: {e}"))?;

        let store = FilesystemSkillStore::new(&store_path)
            .map_err(|e| format!("Failed to open skill store: {e}"))?;
        let registry = SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?;

        let skill_name = manifest.name.clone();
        let _version = registry
            .register(manifest)
            .await
            .map_err(|e| format!("Failed to register skill: {e}"))?;

        return Ok(SkillImportResult {
            decision: "accepted".to_string(),
            classification: "direct_import".to_string(),
            skill_id: Some(skill_name),
            error: None,
            security_issues: vec![],
        });
    }

    // ---------------------------------------------------------------
    // Path B (optional): Security check — only when sanitize=true
    // ---------------------------------------------------------------
    if sanitize {
        let source_content = std::fs::read_to_string(source_path)
            .map_err(|e| format!("Failed to read file: {e}"))?;

        let format_str = if source_path.is_dir() {
            "directory"
        } else {
            match source_path.extension().and_then(|e| e.to_str()) {
                Some("toml") => "toml",
                Some("md") | Some("markdown") => "markdown",
                Some("yaml") | Some("yml") => "yaml",
                Some("json") => "json",
                _ => "plaintext",
            }
        };

        let security_input = serde_json::json!({
            "source_content": source_content,
            "source_format": format_str,
        });

        use y_core::agent::ContextStrategyHint;
        let security_result = state
            .container
            .agent_delegator
            .delegate(
                "skill-security-check",
                security_input,
                ContextStrategyHint::None,
            )
            .await;

        match security_result {
            Ok(output) => {
                #[derive(serde::Deserialize)]
                struct SecurityOutput {
                    verdict: String,
                    #[serde(default)]
                    issues: Vec<String>,
                    #[serde(default)]
                    risk_level: String,
                    #[serde(default)]
                    summary: String,
                }

                if let Ok(security) = serde_json::from_str::<SecurityOutput>(&output.text) {
                    if security.verdict == "unsecure" {
                        return Ok(SkillImportResult {
                            decision: "rejected".to_string(),
                            classification: String::new(),
                            skill_id: None,
                            error: Some(format!(
                                "Security check failed ({}): {}",
                                security.risk_level, security.summary
                            )),
                            security_issues: security.issues,
                        });
                    }
                }
                // If secure or unparseable, fall through to ingestion.
            }
            Err(e) => {
                tracing::warn!(error = %e, "Security check agent failed — proceeding with ingestion");
            }
        }

        // Notify frontend that the security subagent finished.
        let _ = app.emit(
            "diagnostics:subagent_completed",
            super::chat::SubagentCompletedPayload {
                agent_name: "skill-security-check".to_string(),
            },
        );
    }

    // ---------------------------------------------------------------
    // Path C: Agent-assisted ingestion (both sanitize=true after
    //         security check passes, and sanitize=false for non-TOML)
    // ---------------------------------------------------------------
    let store = FilesystemSkillStore::new(&store_path)
        .map_err(|e| format!("Failed to open skill store: {e}"))?;
    let registry = Arc::new(tokio::sync::RwLock::new(
        SkillRegistryImpl::with_store(store)
            .await
            .map_err(|e| format!("Failed to create registry: {e}"))?,
    ));

    let ingestion_service = state.container.skill_ingestion_service(registry);

    match ingestion_service.import(source_path).await {
        Ok(result) => {
            let decision = match result.decision {
                y_service::ImportDecision::Accepted => "accepted",
                y_service::ImportDecision::PartialAccept => "partial_accept",
                y_service::ImportDecision::Rejected => "rejected",
            };
            Ok(SkillImportResult {
                decision: decision.to_string(),
                classification: result.classification,
                skill_id: result.skill_id,
                error: result.rejection_reason,
                security_issues: result.security_issues,
            })
        }
        Err(e) => Ok(SkillImportResult {
            decision: "rejected".to_string(),
            classification: String::new(),
            skill_id: None,
            error: Some(e.to_string()),
            security_issues: vec![],
        }),
    }
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
    let target = skill_dir.join(&relative_path);

    // Path traversal guard.
    let canonical_dir = skill_dir
        .canonicalize()
        .map_err(|e| format!("Skill dir not found: {e}"))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("File not found: {e}"))?;
    if !canonical_target.starts_with(&canonical_dir) {
        return Err("Access denied: path traversal detected".to_string());
    }

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
    let target = skill_dir.join(&relative_path);

    // Path traversal guard.
    let canonical_dir = skill_dir
        .canonicalize()
        .map_err(|e| format!("Skill dir not found: {e}"))?;

    // For save, the file may not exist yet — check parent instead.
    let parent = target.parent().ok_or("Invalid path")?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|e| format!("Parent dir not found: {e}"))?;
    if !canonical_parent.starts_with(&canonical_dir) {
        return Err("Access denied: path traversal detected".to_string());
    }

    std::fs::write(&target, content).map_err(|e| format!("Failed to write file: {e}"))
}
