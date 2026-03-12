//! Configuration command handlers — read/write settings.

use serde_json::Value;
use tauri::State;

use crate::state::{save_gui_config, AppState, GuiConfig};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Get the full service configuration as JSON.
///
/// Returns a JSON object with all config sections (providers, storage, session,
/// runtime, hooks, tools, guardrails) as they were loaded at startup.
#[tauri::command]
pub async fn config_get(state: State<'_, AppState>) -> Result<Value, String> {
    // Read the config files from the config directory and return as JSON.
    let config_dir = &state.config_dir;
    let mut merged = serde_json::Map::new();

    let sections = [
        "providers", "storage", "session", "runtime", "hooks", "tools", "guardrails",
    ];

    for section in §ions {
        let path = config_dir.join(format!("{section}.toml"));
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {section}.toml: {e}"))?;
            let value: Value = toml::from_str(&content)
                .map_err(|e| format!("Failed to parse {section}.toml: {e}"))?;
            merged.insert(section.to_string(), value);
        }
    }

    Ok(Value::Object(merged))
}

/// Set a specific configuration section.
///
/// Writes the given JSON content to `~/.config/y-agent/{section}.toml`.
#[tauri::command]
pub async fn config_set_section(
    state: State<'_, AppState>,
    section: String,
    content: Value,
) -> Result<(), String> {
    let allowed = [
        "providers", "storage", "session", "runtime", "hooks", "tools", "guardrails",
    ];
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    let path = state.config_dir.join(format!("{section}.toml"));
    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    let toml_str = toml::to_string_pretty(&content)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;

    std::fs::write(&path, toml_str)
        .map_err(|e| format!("Failed to write {section}.toml: {e}"))?;

    Ok(())
}

/// Get the GUI-specific configuration.
#[tauri::command]
pub async fn config_get_gui(state: State<'_, AppState>) -> Result<GuiConfig, String> {
    let config = state.gui_config.read().await;
    Ok(config.clone())
}

/// Set the GUI-specific configuration and persist to disk.
#[tauri::command]
pub async fn config_set_gui(
    state: State<'_, AppState>,
    config: GuiConfig,
) -> Result<(), String> {
    save_gui_config(&state.config_dir, &config)
        .map_err(|e| format!("Failed to save GUI config: {e}"))?;

    let mut current = state.gui_config.write().await;
    *current = config;
    Ok(())
}
