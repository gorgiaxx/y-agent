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

    for section in &sections {
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
        "providers", "storage", "session", "runtime", "hooks", "tools", "guardrails",
    ];
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    let path = state.config_dir.join(format!("{section}.toml"));
    if !path.exists() {
        return Ok(String::new());
    }

    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {section}.toml: {e}"))
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
        "providers", "storage", "session", "runtime", "hooks", "tools", "guardrails",
    ];
    if !allowed.contains(&section.as_str()) {
        return Err(format!("Unknown config section: {section}"));
    }

    // Validate TOML syntax before writing.
    let _: Value = toml::from_str(&content)
        .map_err(|e| format!("Invalid TOML syntax: {e}"))?;

    let path = state.config_dir.join(format!("{section}.toml"));
    std::fs::create_dir_all(&state.config_dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    std::fs::write(&path, &content)
        .map_err(|e| format!("Failed to write {section}.toml: {e}"))?;

    Ok(())
}

/// Hot-reload the provider pool from updated config files.
///
/// Re-reads `providers.toml` and rebuilds all LLM provider instances.
/// In-flight requests using the old pool are unaffected.
#[tauri::command]
pub async fn config_reload(state: State<'_, AppState>) -> Result<String, String> {
    let path = state.config_dir.join("providers.toml");
    if !path.exists() {
        return Err("providers.toml not found".into());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read providers.toml: {e}"))?;

    let pool_config: y_provider::ProviderPoolConfig = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse providers.toml: {e}"))?;

    state.container.reload_providers(&pool_config).await;

    let count = state.container.provider_pool().await.list_metadata().len();
    Ok(format!("Provider pool reloaded: {count} provider(s) active"))
}

/// Test an LLM provider configuration by sending a minimal probe request.
///
/// Providers using the OpenAI-compatible REST protocol are actively tested by
/// sending a single-token chat completion to `{base_url}/chat/completions`.
/// Supported active-test types: "openai", "openai-compat", "azure", "ollama",
/// "deepseek".
///
/// All other provider types ("anthropic", "gemini", etc.) immediately return
/// Ok to signal that the configuration is accepted.  Active testing for those
/// types is reserved for future development once their SDKs or REST clients
/// are integrated into the Tauri layer.
#[tauri::command]
pub async fn provider_test(
    provider_type: String,
    model: String,
    api_key: String,
    api_key_env: String,
    base_url: Option<String>,
) -> Result<String, String> {
    // Resolve the effective API key: direct value overrides env var.
    let effective_key = if !api_key.is_empty() {
        api_key.clone()
    } else if !api_key_env.is_empty() {
        std::env::var(&api_key_env)
            .map_err(|_| format!("Environment variable '{}' is not set", api_key_env))?
    } else {
        return Err("No API key configured (set 'API Key' or 'API Key Env Var')".into());
    };

    match provider_type.as_str() {
        "openai" | "openai-compat" | "azure" | "ollama" | "deepseek" => {
            // Resolve default base URL per provider type.
            let resolved_base = base_url.as_deref().unwrap_or_else(|| {
                match provider_type.as_str() {
                    "azure" => "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT",
                    "ollama" => "http://localhost:11434/v1",
                    "deepseek" => "https://api.deepseek.com/v1",
                    _ => "https://api.openai.com/v1",
                }
            });

            let url = format!("{}/chat/completions", resolved_base.trim_end_matches('/'));

            // Minimal probe request: ask for a single token to minimise cost.
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }]
            });

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {effective_key}"))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Network error: {e}"))?;

            let status = response.status();

            if status.is_success() {
                return Ok("Connection successful -- provider responded normally".into());
            }

            // Attempt to parse an OpenAI-style error body.
            let body_text = response.text().await.unwrap_or_default();
            let detail: String = serde_json::from_str::<serde_json::Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_owned())
                })
                .unwrap_or_else(|| body_text.chars().take(200).collect());

            if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                return Err(format!("Authentication failed: {detail}"));
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(format!("Rate limited by provider: {detail}"));
            }

            Err(format!("Provider returned HTTP {status}: {detail}"))
        }

        // Anthropic, Gemini, and any other future provider type: skip active
        // testing.  The configuration is accepted as-is; connectivity
        // verification for these providers is reserved for future development.
        _ => Ok(format!(
            "Configuration accepted (active connection test is not yet implemented \
             for provider type '{provider_type}')"
        )),
    }
}
