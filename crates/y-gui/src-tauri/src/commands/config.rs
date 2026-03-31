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

    // 7. Knowledge.
    if let Some(content) = read_toml("knowledge")? {
        y_service::SystemService::reload_knowledge_from_toml(&state.container, &content).await?;
        results.push("knowledge".to_string());
    }

    // 8. Hooks.
    if let Some(content) = read_toml("hooks")? {
        y_service::SystemService::reload_hooks_from_toml(&state.container, &content)?;
        results.push("hooks".to_string());
    }

    // 9. Prompts -- always reload from disk (no TOML config, just .txt files).
    y_service::SystemService::reload_prompts(&state.container).await;
    results.push("prompts".to_string());

    // 10. Agents -- always reload from disk (TOML files in agents/ directory).
    let (loaded, errored) = y_service::SystemService::reload_agents(&state.container).await;
    if errored > 0 {
        results.push(format!("{loaded} agent(s), {errored} error(s)"));
    } else {
        results.push(format!("{loaded} agent(s)"));
    }

    // NOTE: Storage (connection pool, WAL mode) is not hot-reloadable;
    // changes to storage.toml require an application restart.
    // NOTE: MCP server configuration (mcp.json) is saved to disk but not
    // hot-reloaded into the runtime; MCP servers are discovered on demand.

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

/// Fetch available models from an OpenAI-compatible `/v1/models` endpoint.
///
/// Returns a JSON array of `{id, display_name}` objects.
#[tauri::command]
pub async fn provider_list_models(
    base_url: String,
    api_key: String,
    api_key_env: String,
) -> Result<serde_json::Value, String> {
    let effective_key = if !api_key.is_empty() {
        api_key
    } else if !api_key_env.is_empty() {
        std::env::var(&api_key_env)
            .map_err(|_| format!("Environment variable '{api_key_env}' is not set"))?
    } else {
        String::new()
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut req = client.get(&url);
    if !effective_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {effective_key}"));
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Network error reaching {url}: {e}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let detail: String = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(std::borrow::ToOwned::to_owned)
            })
            .unwrap_or_else(|| {
                if body.is_empty() {
                    format!("(no response body, HTTP {status})")
                } else {
                    body.chars().take(200).collect()
                }
            });
        return Err(format!("HTTP {status}: {detail}"));
    }

    // Parse and return the full response JSON so the frontend can handle it.
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {e}"))?;
    Ok(value)
}

// ---------------------------------------------------------------------------
// MCP server config commands (JSON-based, stored in <config_dir>/mcp.json)
// ---------------------------------------------------------------------------

/// Get the MCP server configuration as JSON.
///
/// Returns the parsed contents of `mcp.json`. If the file does not exist,
/// returns `{"mcpServers": {}}`.
#[tauri::command]
pub async fn mcp_config_get(state: State<'_, AppState>) -> Result<Value, String> {
    let path = state.config_dir.join("mcp.json");
    if !path.exists() {
        return Ok(serde_json::json!({"mcpServers": {}}));
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read mcp.json: {e}"))?;
    let value: Value =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse mcp.json: {e}"))?;
    Ok(value)
}

/// Save the MCP server configuration from JSON.
///
/// Validates the JSON and writes it to `mcp.json` with pretty formatting.
#[tauri::command]
pub async fn mcp_config_save(state: State<'_, AppState>, content: Value) -> Result<(), String> {
    let json_str = serde_json::to_string_pretty(&content)
        .map_err(|e| format!("Failed to serialize MCP config: {e}"))?;

    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    let path = state.config_dir.join("mcp.json");
    std::fs::write(&path, &json_str).map_err(|e| format!("Failed to write mcp.json: {e}"))?;

    Ok(())
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
