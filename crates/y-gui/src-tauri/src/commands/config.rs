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

    // Sync the translation target language template variable in the agent registry.
    let mut registry = state.container.agent_registry.lock().await;
    registry.add_template_var(
        "{{TRANSLATE_TARGET_LANGUAGE}}".to_string(),
        config.translate_target_language.clone(),
    );
    drop(registry);

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
// MCP server config commands (backed by <config_dir>/tools.toml [[mcp_servers]])
// ---------------------------------------------------------------------------

/// Get the MCP server configuration as JSON.
///
/// Loads `[[mcp_servers]]` entries from `tools.toml` and returns them in the
/// frontend-native shape: `{"mcpServers": {name: {transport, ...}}}`.
#[tauri::command]
pub async fn mcp_config_get(state: State<'_, AppState>) -> Result<Value, String> {
    let path = state.config_dir.join("tools.toml");
    let servers = y_tools::mcp_toml::load_mcp_servers(&path)
        .map_err(|e| format!("Failed to read mcp_servers from tools.toml: {e}"))?;

    let mut map = serde_json::Map::new();
    for s in servers {
        map.insert(s.name.clone(), server_to_json(&s));
    }
    Ok(serde_json::json!({ "mcpServers": Value::Object(map) }))
}

/// Save the MCP server configuration from JSON.
///
/// Parses the frontend `{"mcpServers": {...}}` shape and replaces the
/// `[[mcp_servers]]` section of `tools.toml`, preserving unrelated entries.
#[tauri::command]
pub async fn mcp_config_save(state: State<'_, AppState>, content: Value) -> Result<(), String> {
    let path = state.config_dir.join("tools.toml");
    let servers = json_to_servers(&content)?;
    y_tools::mcp_toml::replace_mcp_servers(&path, &servers)
        .map_err(|e| format!("Failed to write mcp_servers to tools.toml: {e}"))?;
    Ok(())
}

fn server_to_json(s: &y_tools::McpServerConfig) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("transport".into(), Value::String(s.transport.clone()));
    obj.insert("disabled".into(), Value::Bool(!s.enabled));
    obj.insert(
        "startup_timeout_secs".into(),
        Value::Number(s.startup_timeout_secs.into()),
    );
    obj.insert(
        "tool_timeout_secs".into(),
        Value::Number(s.tool_timeout_secs.into()),
    );
    if let Some(c) = &s.command {
        obj.insert("command".into(), Value::String(c.clone()));
    }
    if !s.args.is_empty() {
        obj.insert(
            "args".into(),
            Value::Array(s.args.iter().cloned().map(Value::String).collect()),
        );
    }
    if let Some(u) = &s.url {
        obj.insert("url".into(), Value::String(u.clone()));
    }
    if !s.env.is_empty() {
        let env = s
            .env
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        obj.insert("env".into(), Value::Object(env));
    }
    if !s.headers.is_empty() {
        let h = s
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        obj.insert("headers".into(), Value::Object(h));
    }
    if let Some(cwd) = &s.cwd {
        obj.insert("cwd".into(), Value::String(cwd.clone()));
    }
    if let Some(t) = &s.bearer_token {
        obj.insert("bearer_token".into(), Value::String(t.clone()));
    }
    if let Some(list) = &s.enabled_tools {
        obj.insert(
            "alwaysAllow".into(),
            Value::Array(list.iter().cloned().map(Value::String).collect()),
        );
    }
    Value::Object(obj)
}

fn json_to_servers(content: &Value) -> Result<Vec<y_tools::McpServerConfig>, String> {
    let servers_val = content
        .get("mcpServers")
        .ok_or("payload is missing 'mcpServers' key")?;
    let servers_obj = servers_val
        .as_object()
        .ok_or("'mcpServers' must be an object")?;

    let mut out = Vec::with_capacity(servers_obj.len());
    for (name, cfg) in servers_obj {
        let cfg_obj = cfg
            .as_object()
            .ok_or_else(|| format!("'{name}' must be an object"))?;

        let transport = cfg_obj
            .get("transport")
            .and_then(Value::as_str)
            .map_or_else(
                || {
                    if cfg_obj.contains_key("url") {
                        "http".to_string()
                    } else {
                        "stdio".to_string()
                    }
                },
                str::to_string,
            );

        let disabled = cfg_obj
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let args = cfg_obj
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let env = string_map(cfg_obj.get("env"));
        let headers = string_map(cfg_obj.get("headers"));

        let enabled_tools = cfg_obj
            .get("alwaysAllow")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty());

        out.push(y_tools::McpServerConfig {
            name: name.clone(),
            transport,
            command: cfg_obj
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            args,
            url: cfg_obj
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string),
            env,
            enabled: !disabled,
            headers,
            startup_timeout_secs: cfg_obj
                .get("startup_timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(30),
            tool_timeout_secs: cfg_obj
                .get("tool_timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(120),
            cwd: cfg_obj
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_string),
            bearer_token: cfg_obj
                .get("bearer_token")
                .and_then(Value::as_str)
                .map(str::to_string),
            enabled_tools,
            disabled_tools: None,
            auto_reconnect: cfg_obj
                .get("auto_reconnect")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            max_reconnect_attempts: cfg_obj
                .get("max_reconnect_attempts")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(5),
        });
    }
    Ok(out)
}

fn string_map(v: Option<&Value>) -> std::collections::HashMap<String, String> {
    v.and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
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
