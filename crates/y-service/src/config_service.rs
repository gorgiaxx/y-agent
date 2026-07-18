//! Configuration file service: section-level read/write of per-concern TOML
//! config files.
//!
//! This service centralises the config-file I/O that was previously
//! duplicated across presentation layers (`y-gui`, `y-web`). Presentation
//! layers delegate here instead of re-implementing file I/O and section
//! validation.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

/// Config file basenames (without `.toml` extension) that can be read/written
/// via the section API.
///
/// This is the single source of truth for allowed sections. Presentation
/// layers must not re-declare this list.
pub const CONFIG_SECTIONS: &[&str] = &[
    "providers",
    "storage",
    "session",
    "runtime",
    "hooks",
    "tools",
    "guardrails",
    "browser",
    "knowledge",
    "background_auto_wake",
    "lsp",
    "langfuse",
];

/// Validate that a section name is in the allowed list.
fn validate_section(section: &str) -> Result<(), String> {
    if CONFIG_SECTIONS.contains(&section) {
        Ok(())
    } else {
        Err(format!("Unknown config section: {section}"))
    }
}

/// Configuration file service for section-level TOML read/write.
///
/// All methods take a `config_dir` path so the service is stateless and
/// testable without a running container.
pub struct ConfigService;

impl ConfigService {
    /// Read all config sections from `config_dir` and return them as a JSON
    /// object keyed by section name.
    ///
    /// Missing files are silently skipped.
    pub fn read_all(config_dir: &Path) -> Result<Value, String> {
        let mut merged = serde_json::Map::new();
        for section in CONFIG_SECTIONS {
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

    /// Read a single config section's raw TOML content.
    ///
    /// Returns an empty string if the file does not exist.
    pub fn read_section(config_dir: &Path, section: &str) -> Result<String, String> {
        validate_section(section)?;
        let path = config_dir.join(format!("{section}.toml"));
        if !path.exists() {
            return Ok(String::new());
        }
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {section}.toml: {e}"))
    }

    /// Save a single config section from raw TOML content.
    ///
    /// Validates TOML syntax before writing.
    pub fn save_section(config_dir: &Path, section: &str, content: &str) -> Result<(), String> {
        validate_section(section)?;
        // Validate TOML syntax before writing.
        let _: Value = toml::from_str(content).map_err(|e| format!("Invalid TOML syntax: {e}"))?;
        let path = config_dir.join(format!("{section}.toml"));
        std::fs::create_dir_all(config_dir)
            .map_err(|e| format!("Failed to create config dir: {e}"))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("Failed to write {section}.toml: {e}"))?;
        Ok(())
    }

    /// Write a config section from a JSON value (serialized to TOML).
    ///
    /// Used by the GUI which sends JSON; the web layer uses raw TOML via
    /// [`ConfigService::save_section`].
    pub fn write_section_json(
        config_dir: &Path,
        section: &str,
        content: &Value,
    ) -> Result<(), String> {
        validate_section(section)?;
        let toml_str = toml::to_string_pretty(content)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        let path = config_dir.join(format!("{section}.toml"));
        std::fs::create_dir_all(config_dir)
            .map_err(|e| format!("Failed to create config dir: {e}"))?;
        std::fs::write(&path, toml_str)
            .map_err(|e| format!("Failed to write {section}.toml: {e}"))?;
        Ok(())
    }
}

/// Fetch available models from a provider's model listing endpoint.
///
/// For OpenAI-compatible providers, queries `{base_url}/models`.
/// For Azure, queries `{prefix}/models?api-version={version}` with
/// `api-key` header authentication.
///
/// Returns the full response JSON so the caller can handle it.
pub async fn list_provider_models<S: std::hash::BuildHasher>(
    base_url: &str,
    api_key: &str,
    api_key_env: &str,
    headers: Option<&std::collections::HashMap<String, String, S>>,
    http_protocol: HttpProtocol,
    provider_type: Option<&str>,
    azure_resource_name: Option<&str>,
    azure_api_version: Option<&str>,
    azure_auth_mode: Option<&str>,
) -> Result<Value, String> {
    use crate::system::SystemService;

    let effective_key = if !api_key.is_empty() {
        api_key.to_string()
    } else if !api_key_env.is_empty() {
        std::env::var(api_key_env)
            .map_err(|_| format!("Environment variable '{api_key_env}' is not set"))?
    } else {
        String::new()
    };

    let client = SystemService::provider_http_client_builder(http_protocol)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    let custom_headers = match headers {
        Some(h) => SystemService::provider_custom_header_map(h)?,
        None => reqwest::header::HeaderMap::new(),
    };

    let pt = provider_type.unwrap_or("");
    let res_name = azure_resource_name.unwrap_or("");
    let api_ver = azure_api_version.unwrap_or("");
    let auth_mode = azure_auth_mode.unwrap_or("");

    let url = resolve_models_url(pt, base_url, res_name, api_ver);
    let mut req = SystemService::apply_provider_custom_headers(client.get(&url), &custom_headers);
    if !effective_key.is_empty() {
        req = apply_model_list_auth(req, pt, auth_mode, &effective_key);
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Network error reaching {url}: {e}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let detail: String = serde_json::from_str::<Value>(&body)
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

    let value: Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {e}"))?;
    Ok(value)
}

/// Build the model-listing URL for a provider type.
fn resolve_models_url(
    provider_type: &str,
    base_url: &str,
    azure_resource_name: &str,
    azure_api_version: &str,
) -> String {
    if provider_type != "azure" {
        return format!("{}/models", base_url.trim_end_matches('/'));
    }

    let api_version = if azure_api_version.is_empty() {
        "2024-10-21"
    } else {
        azure_api_version
    };

    let prefix = if !base_url.is_empty() {
        if let Some(idx) = base_url.find("/deployments/") {
            base_url[..idx].to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        }
    } else if !azure_resource_name.is_empty() {
        format!("https://{azure_resource_name}.openai.azure.com/openai")
    } else {
        return format!("{}/models", base_url.trim_end_matches('/'));
    };

    format!("{prefix}/models?api-version={api_version}")
}

/// Apply the correct authentication header for the model-listing request.
fn apply_model_list_auth(
    req: reqwest::RequestBuilder,
    provider_type: &str,
    azure_auth_mode: &str,
    key: &str,
) -> reqwest::RequestBuilder {
    if provider_type == "azure" && azure_auth_mode != "bearer" {
        req.header("api-key", key)
    } else {
        req.header("Authorization", format!("Bearer {key}"))
    }
}

/// Re-export for convenience.
pub use crate::system::HttpProtocol;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_section_accepts_known() {
        assert!(validate_section("providers").is_ok());
        assert!(validate_section("langfuse").is_ok());
    }

    #[test]
    fn validate_section_rejects_unknown() {
        assert!(validate_section("evil").is_err());
    }

    #[test]
    fn resolve_models_url_openai_compat() {
        let url = resolve_models_url("openai", "https://api.openai.com/v1", "", "");
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    #[test]
    fn resolve_models_url_azure_with_resource_name() {
        let url = resolve_models_url("azure", "", "my-resource", "2024-10-21");
        assert_eq!(
            url,
            "https://my-resource.openai.azure.com/openai/models?api-version=2024-10-21"
        );
    }

    #[test]
    fn resolve_models_url_azure_strips_deployments() {
        let url = resolve_models_url(
            "azure",
            "https://my-resource.openai.azure.com/openai/deployments/gpt-4",
            "",
            "2024-10-21",
        );
        assert_eq!(
            url,
            "https://my-resource.openai.azure.com/openai/models?api-version=2024-10-21"
        );
    }

    #[test]
    fn resolve_models_url_azure_default_api_version() {
        let url = resolve_models_url(
            "azure",
            "https://my-resource.openai.azure.com/openai",
            "",
            "",
        );
        assert!(url.contains("api-version=2024-10-21"));
    }

    #[test]
    fn read_section_returns_empty_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let content = ConfigService::read_section(dir.path(), "providers").unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn save_and_read_section_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let toml_content = "db_path = \"/tmp/test.db\"";
        ConfigService::save_section(dir.path(), "storage", toml_content).unwrap();
        let read = ConfigService::read_section(dir.path(), "storage").unwrap();
        assert_eq!(read, toml_content);
    }

    #[test]
    fn save_section_rejects_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let result = ConfigService::save_section(dir.path(), "storage", "not valid {{{}}}");
        assert!(result.is_err());
    }

    #[test]
    fn save_section_rejects_unknown_section() {
        let dir = tempfile::tempdir().unwrap();
        let result = ConfigService::save_section(dir.path(), "evil", "key = \"val\"");
        assert!(result.is_err());
    }

    #[test]
    fn read_all_skips_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("storage.toml"),
            "db_path = \"/tmp/test.db\"",
        )
        .unwrap();
        let all = ConfigService::read_all(dir.path()).unwrap();
        let obj = all.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("storage"));
    }
}
