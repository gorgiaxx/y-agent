//! System status service.

use y_core::provider::ProviderPool;
use y_core::runtime::RuntimeAdapter;
use y_provider::ProviderPoolConfig;

use crate::container::ServiceContainer;

/// Status report for the system.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusReport {
    /// Application version.
    pub version: String,
    /// Number of registered LLM providers.
    pub providers_registered: usize,
    /// Number of registered tools.
    pub tools_registered: usize,
    /// Runtime backend identifier.
    pub runtime_backend: String,
    /// Storage connection status.
    pub storage_status: String,
}

/// Health report combining diagnostics and system status.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthReport {
    /// System status.
    pub status: StatusReport,
    /// Diagnostics health.
    pub diagnostics: crate::diagnostics::HealthCheckResult,
}

/// Summary of a configured provider.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    /// Unique provider identifier.
    pub id: String,
    /// Model name configured for this provider.
    pub model: String,
    /// Provider backend type (e.g. "openai", "anthropic").
    pub provider_type: String,
}

/// Request to test a provider configuration.
#[derive(Debug, Clone)]
pub struct ProviderTestRequest {
    /// Provider type identifier.
    pub provider_type: String,
    /// Model to test.
    pub model: String,
    /// Direct API key value.
    pub api_key: String,
    /// Environment variable name holding the API key.
    pub api_key_env: String,
    /// Optional base URL override.
    pub base_url: Option<String>,
}

/// System-level service for status and health reporting.
pub struct SystemService;

impl SystemService {
    /// Gather system status report.
    pub async fn status(container: &ServiceContainer, version: &str) -> StatusReport {
        let provider_count = container
            .provider_pool()
            .await
            .provider_statuses()
            .await
            .len();
        let tool_count = container.tool_registry.len().await;
        let runtime_backend = format!("{:?}", container.runtime_manager.backend());

        StatusReport {
            version: version.to_string(),
            providers_registered: provider_count,
            tools_registered: tool_count,
            runtime_backend,
            storage_status: "connected".to_string(),
        }
    }

    /// Full health report (system + diagnostics).
    pub async fn health(container: &ServiceContainer, version: &str) -> HealthReport {
        let status = Self::status(container, version).await;
        let diagnostics = crate::DiagnosticsService::health_check(container).await;
        HealthReport {
            status,
            diagnostics,
        }
    }

    /// List all configured providers with their metadata.
    pub async fn list_providers(container: &ServiceContainer) -> Vec<ProviderInfo> {
        let pool = container.provider_pool().await;
        pool.list_metadata()
            .iter()
            .map(|m| ProviderInfo {
                id: m.id.to_string(),
                model: m.model.clone(),
                provider_type: format!("{:?}", m.provider_type),
            })
            .collect()
    }

    /// Hot-reload the provider pool from a TOML config string.
    ///
    /// Parses the TOML into `ProviderPoolConfig` and delegates to
    /// `container.reload_providers()`. Returns the count of active providers.
    pub async fn reload_providers_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<usize, String> {
        let pool_config: ProviderPoolConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse providers config: {e}"))?;
        container.reload_providers(&pool_config).await;
        let count = container.provider_pool().await.list_metadata().len();
        Ok(count)
    }

    /// Hot-reload the guardrail config from a TOML config string.
    ///
    /// Parses the TOML into `GuardrailConfig` and delegates to
    /// `container.reload_guardrails()`.
    pub fn reload_guardrails_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_guardrails::GuardrailConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse guardrails config: {e}"))?;
        container.reload_guardrails(config);
        Ok(())
    }

    /// Hot-reload the session config from a TOML config string.
    ///
    /// `session.toml` now contains both session fields and a `[pruning]` section.
    /// The session portion is hot-reloaded; the pruning portion is parsed but
    /// not hot-reloaded (`PruningEngine` does not support runtime reconfiguration).
    pub fn reload_session_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        /// Combined struct matching the `session.toml` layout.
        #[derive(serde::Deserialize)]
        struct SessionFileConfig {
            #[serde(flatten)]
            session: y_session::SessionConfig,
            #[serde(default)]
            #[allow(dead_code)]
            pruning: y_context::PruningConfig,
        }

        let combined: SessionFileConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse session config: {e}"))?;
        container.reload_session(combined.session);
        // NOTE: pruning config is not hot-reloaded; restart required for changes.
        Ok(())
    }

    /// Hot-reload the runtime config from a TOML config string.
    pub fn reload_runtime_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_runtime::RuntimeConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse runtime config: {e}"))?;
        container.reload_runtime(config);
        Ok(())
    }

    /// Hot-reload the browser config from a TOML config string.
    pub async fn reload_browser_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_browser::BrowserConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse browser config: {e}"))?;
        container.reload_browser(config).await;
        Ok(())
    }

    /// Hot-reload the tools config from a TOML config string.
    pub fn reload_tools_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_tools::ToolRegistryConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse tools config: {e}"))?;
        container.reload_tools(config);
        Ok(())
    }

    /// Hot-reload prompt section files from disk.
    ///
    /// Re-reads all prompt `.txt` files from the prompts directory and
    /// rebuilds the in-memory section store. Changes take effect on the
    /// next LLM turn.
    pub async fn reload_prompts(container: &ServiceContainer) {
        container.reload_prompts().await;
    }

    /// Hot-reload the knowledge config from a TOML config string.
    ///
    /// Parses the TOML into `KnowledgeConfig` and delegates to
    /// `container.reload_knowledge()`.
    pub async fn reload_knowledge_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_knowledge::config::KnowledgeConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse knowledge config: {e}"))?;
        container.reload_knowledge(config).await;
        Ok(())
    }

    /// Test an LLM provider by sending a minimal probe request.
    ///
    /// Providers using OpenAI-compatible REST are actively tested via a
    /// single-token chat completion. Other types return Ok immediately.
    pub async fn test_provider(request: ProviderTestRequest) -> Result<String, String> {
        let effective_key = if !request.api_key.is_empty() {
            request.api_key.clone()
        } else if !request.api_key_env.is_empty() {
            std::env::var(&request.api_key_env)
                .map_err(|_| format!("Environment variable '{}' is not set", request.api_key_env))?
        } else {
            return Err("No API key configured (set 'API Key' or 'API Key Env Var')".into());
        };

        match request.provider_type.as_str() {
            "openai" | "openai-compat" | "azure" | "ollama" | "deepseek" => {
                let resolved_base = request
                    .base_url
                    .as_deref()
                    .unwrap_or(match request.provider_type.as_str() {
                    "azure" => {
                        "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT"
                    }
                    "ollama" => "http://localhost:11434/v1",
                    "deepseek" => "https://api.deepseek.com/v1",
                    _ => "https://api.openai.com/v1",
                });

                let url = format!("{}/chat/completions", resolved_base.trim_end_matches('/'));

                let body = serde_json::json!({
                    "model": request.model,
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

                let body_text = response.text().await.unwrap_or_default();
                let detail: String = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v.pointer("/error/message")
                            .and_then(|m| m.as_str())
                            .map(std::borrow::ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| {
                        if body_text.is_empty() {
                            format!("(no response body, HTTP {status})")
                        } else {
                            body_text.chars().take(200).collect()
                        }
                    });

                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(format!("Authentication failed (HTTP {status}): {detail}"));
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!("Rate limited by provider: {detail}"));
                }

                Err(format!("Provider returned HTTP {status}: {detail}"))
            }
            "anthropic" => {
                let resolved_base = request
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.anthropic.com/v1");

                let url = format!("{}/messages", resolved_base.trim_end_matches('/'));

                let body = serde_json::json!({
                    "model": request.model,
                    "max_tokens": 1,
                    "messages": [{ "role": "user", "content": "ping" }]
                });

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                    .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

                let response = client
                    .post(&url)
                    .header("x-api-key", &effective_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error: {e}"))?;

                let status = response.status();

                if status.is_success() {
                    return Ok("Connection successful -- provider responded normally".into());
                }

                let body_text = response.text().await.unwrap_or_default();
                // Anthropic error shape: {"type":"error","error":{"type":"...","message":"..."}}
                let detail: String = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v.pointer("/error/message")
                            .and_then(|m| m.as_str())
                            .map(std::borrow::ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| {
                        if body_text.is_empty() {
                            format!("(no response body, HTTP {status})")
                        } else {
                            body_text.chars().take(200).collect()
                        }
                    });

                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(format!("Authentication failed (HTTP {status}): {detail}"));
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!("Rate limited by provider: {detail}"));
                }

                Err(format!("Provider returned HTTP {status}: {detail}"))
            }
            "gemini" => {
                let resolved_base = request
                    .base_url
                    .as_deref()
                    .unwrap_or("https://generativelanguage.googleapis.com/v1beta");

                let url = format!(
                    "{}/models/{}:generateContent?key={}",
                    resolved_base.trim_end_matches('/'),
                    request.model,
                    effective_key
                );

                let body = serde_json::json!({
                    "contents": [{"parts": [{"text": "ping"}]}],
                    "generationConfig": {"maxOutputTokens": 1}
                });

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(15))
                    .build()
                    .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

                let response = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error: {e}"))?;

                let status = response.status();

                if status.is_success() {
                    return Ok("Connection successful -- provider responded normally".into());
                }

                let body_text = response.text().await.unwrap_or_default();
                let detail: String = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| {
                        v.pointer("/error/message")
                            .and_then(|m| m.as_str())
                            .map(std::borrow::ToOwned::to_owned)
                    })
                    .unwrap_or_else(|| {
                        if body_text.is_empty() {
                            format!("(no response body, HTTP {status})")
                        } else {
                            body_text.chars().take(200).collect()
                        }
                    });

                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(format!("Authentication failed (HTTP {status}): {detail}"));
                }
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!("Rate limited by provider: {detail}"));
                }

                Err(format!("Provider returned HTTP {status}: {detail}"))
            }
            _ => Ok(format!(
                "Configuration accepted (active connection test is not yet implemented \
                 for provider type '{}')",
                request.provider_type
            )),
        }
    }
}
