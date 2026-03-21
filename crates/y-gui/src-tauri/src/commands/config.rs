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

    for section in &sections {
        let path = config_dir.join(format!("{section}.toml"));
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {section}.toml: {e}"))?;
            let value: Value = toml::from_str(&content)
                .map_err(|e| format!("Failed to parse {section}.toml: {e}"))?;
            merged.insert((*section).to_string(), value);
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
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    let path = state.config_dir.join(format!("{section}.toml"));
    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    let toml_str =
        toml::to_string_pretty(&content).map_err(|e| format!("Failed to serialize config: {e}"))?;

    std::fs::write(&path, toml_str).map_err(|e| format!("Failed to write {section}.toml: {e}"))?;

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
pub async fn config_set_gui(state: State<'_, AppState>, config: GuiConfig) -> Result<(), String> {
    save_gui_config(&state.config_dir, &config)
        .map_err(|e| format!("Failed to save GUI config: {e}"))?;

    let mut current = state.gui_config.write().await;
    *current = config;
    Ok(())
}

/// Get a single config section's raw TOML content.
///
/// Returns the raw file content as a string. If the file does not exist,
/// returns an empty string.
#[tauri::command]
pub async fn config_get_section(
    state: State<'_, AppState>,
    section: String,
) -> Result<String, String> {
    let allowed = [
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
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    let path = state.config_dir.join(format!("{section}.toml"));
    if !path.exists() {
        return Ok(String::new());
    }

    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {section}.toml: {e}"))
}

/// Save a single config section from raw TOML content.
///
/// Validates the TOML syntax before writing.
#[tauri::command]
pub async fn config_save_section(
    state: State<'_, AppState>,
    section: String,
    content: String,
) -> Result<(), String> {
    let allowed = [
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
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    // Validate TOML syntax before writing.
    let _: Value = toml::from_str(&content).map_err(|e| format!("Invalid TOML syntax: {e}"))?;

    let path = state.config_dir.join(format!("{section}.toml"));
    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    std::fs::write(&path, &content).map_err(|e| format!("Failed to write {section}.toml: {e}"))?;

    Ok(())
}

/// Hot-reload configuration from updated config files.
///
/// Re-reads all config TOML files and updates the corresponding runtime
/// managers. In-flight requests using old values are unaffected.
#[tauri::command]
pub async fn config_reload(state: State<'_, AppState>) -> Result<String, String> {
    let mut results: Vec<String> = Vec::new();

    // Helper: try to read a config file; returns None if file doesn't exist.
    let read_toml = |name: &str| -> Result<Option<String>, String> {
        let path = state.config_dir.join(format!("{name}.toml"));
        if !path.exists() {
            return Ok(None);
        }
        std::fs::read_to_string(&path)
            .map(Some)
            .map_err(|e| format!("Failed to read {name}.toml: {e}"))
    };

    // 1. Providers.
    if let Some(content) = read_toml("providers")? {
        let count =
            y_service::SystemService::reload_providers_from_toml(&state.container, &content)
                .await?;
        results.push(format!("{count} provider(s)"));
    }

    // 2. Guardrails.
    if let Some(content) = read_toml("guardrails")? {
        y_service::SystemService::reload_guardrails_from_toml(&state.container, &content)?;
        results.push("guardrails".to_string());
    }

    // 3. Session.
    if let Some(content) = read_toml("session")? {
        y_service::SystemService::reload_session_from_toml(&state.container, &content)?;
        results.push("session".to_string());
    }

    // 4. Runtime.
    if let Some(content) = read_toml("runtime")? {
        y_service::SystemService::reload_runtime_from_toml(&state.container, &content)?;
        results.push("runtime".to_string());
    }

    // 5. Browser.
    if let Some(content) = read_toml("browser")? {
        y_service::SystemService::reload_browser_from_toml(&state.container, &content).await?;
        results.push("browser".to_string());
    }

    // 6. Tools.
    if let Some(content) = read_toml("tools")? {
        y_service::SystemService::reload_tools_from_toml(&state.container, &content)?;
        results.push("tools".to_string());
    }

    // 7. Prompts — always reload from disk (no TOML config, just .txt files).
    y_service::SystemService::reload_prompts(&state.container).await;
    results.push("prompts".to_string());

    if results.is_empty() {
        return Ok("Config reloaded (no config files to update)".into());
    }

    Ok(format!("Config reloaded: {}", results.join(", ")))
}

/// Test an LLM provider configuration by sending a minimal probe request.
#[tauri::command]
pub async fn provider_test(
    provider_type: String,
    model: String,
    api_key: String,
    api_key_env: String,
    base_url: Option<String>,
) -> Result<String, String> {
    y_service::SystemService::test_provider(y_service::ProviderTestRequest {
        provider_type,
        model,
        api_key,
        api_key_env,
        base_url,
    })
    .await
}

// ---------------------------------------------------------------------------
// Prompt file commands (plain-text files in <config_dir>/prompts/)
// ---------------------------------------------------------------------------

/// List all prompt `.txt` files in the prompts directory.
///
/// Returns a sorted list of filenames (e.g. `["core_identity.txt", ...]`).
#[tauri::command]
pub async fn prompt_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let prompts_dir = state.config_dir.join("prompts");
    if !prompts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<String> = std::fs::read_dir(&prompts_dir)
        .map_err(|e| format!("Failed to read prompts directory: {e}"))?
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
    Ok(files)
}

/// Read a single prompt file's content.
#[tauri::command]
pub async fn prompt_get(state: State<'_, AppState>, filename: String) -> Result<String, String> {
    // Validate: no path separators allowed.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err("Invalid filename".into());
    }

    let path = state.config_dir.join("prompts").join(&filename);
    if !path.exists() {
        return Ok(String::new());
    }

    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {filename}: {e}"))
}

/// Save content to a single prompt file.
#[tauri::command]
pub async fn prompt_save(
    state: State<'_, AppState>,
    filename: String,
    content: String,
) -> Result<(), String> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err("Invalid filename".into());
    }

    let prompts_dir = state.config_dir.join("prompts");
    std::fs::create_dir_all(&prompts_dir)
        .map_err(|e| format!("Failed to create prompts dir: {e}"))?;

    std::fs::write(prompts_dir.join(&filename), &content)
        .map_err(|e| format!("Failed to write {filename}: {e}"))
}

/// Get the compiled-in default content for a prompt file.
///
/// Returns the built-in content for the given filename (e.g. `core_identity.txt`).
/// Used by the "Restore" button in the GUI to reset user edits to defaults.
#[tauri::command]
pub async fn prompt_get_default(filename: String) -> Result<String, String> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err("Invalid filename".into());
    }

    for &(name, content) in y_prompt::BUILTIN_PROMPT_FILES {
        if name == filename {
            return Ok(content.to_string());
        }
    }

    Err(format!("No built-in default for: {filename}"))
}
