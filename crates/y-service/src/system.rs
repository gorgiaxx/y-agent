//! System status service.

use std::collections::HashMap;

use y_core::provider::ProviderCapability;
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
    /// Provider capabilities used for request shaping.
    pub capabilities: Vec<ProviderCapability>,
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
    /// Additional HTTP headers to send with active probe requests.
    pub headers: HashMap<String, String>,
    /// Routing tags (legacy hint for capability inference).
    pub tags: Vec<String>,
    /// Explicit provider capabilities.
    pub capabilities: Vec<ProviderCapability>,
    /// Probe mode (`auto`, `text_chat`, `image_generation`).
    pub probe_mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderProbeMode {
    TextChat,
    ImageGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeSuccessKind {
    TextChat,
    ImageGeneration,
}

/// System-level service for status and health reporting.
pub struct SystemService;

impl SystemService {
    /// Build a validated header map for provider-facing HTTP requests.
    pub fn provider_custom_header_map<S: std::hash::BuildHasher>(
        headers: &HashMap<String, String, S>,
    ) -> Result<reqwest::header::HeaderMap, String> {
        y_provider::http_headers::custom_header_map(headers)
            .map_err(|message| format!("Invalid custom header: {message}"))
    }

    /// Apply provider-facing custom headers to a request builder.
    pub fn apply_provider_custom_headers(
        request_builder: reqwest::RequestBuilder,
        headers: &reqwest::header::HeaderMap,
    ) -> reqwest::RequestBuilder {
        y_provider::http_headers::apply_custom_headers(request_builder, headers)
    }

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
                capabilities: m.capabilities.clone(),
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

    /// Hot-reload the hook system config from a TOML config string.
    ///
    /// Parses the TOML into `HookConfig` and delegates to
    /// `container.reload_hooks()`.
    pub fn reload_hooks_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<(), String> {
        let config: y_hooks::HookConfig = toml::from_str(toml_content)
            .map_err(|e| format!("Failed to parse hooks config: {e}"))?;
        container.reload_hooks(&config);
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

    /// Hot-reload agent definitions from the agents directory.
    ///
    /// Re-scans all `*.toml` files from the user agents directory and
    /// updates the in-memory registry. Returns `(loaded, errored)` counts.
    pub async fn reload_agents(container: &ServiceContainer) -> (usize, usize) {
        container.reload_agents().await
    }

    /// Register a single agent from raw TOML content at runtime.
    ///
    /// Useful when `agent-architect` creates a new definition that should
    /// take effect immediately. Returns the registered agent's ID.
    pub async fn register_agent_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<String, String> {
        container.register_agent_from_toml(toml_content).await
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
            String::new()
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        let custom_headers = Self::provider_custom_header_map(&request.headers)?;

        let probe_mode = Self::resolve_probe_mode(&request);

        match (request.provider_type.as_str(), probe_mode) {
            (
                "openai" | "openai-compat" | "azure" | "ollama" | "deepseek",
                ProviderProbeMode::TextChat,
            ) => {
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
                let mut req = client.post(&url);
                req = Self::apply_provider_custom_headers(req, &custom_headers)
                    .header("Content-Type", "application/json");
                if !effective_key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {effective_key}"));
                }
                let response = req
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error reaching {url}: {e}"))?;
                Self::interpret_response(response, &["model"], ProbeSuccessKind::TextChat).await
            }
            ("openai" | "openai-compat" | "deepseek", ProviderProbeMode::ImageGeneration) => {
                let resolved_base =
                    request
                        .base_url
                        .as_deref()
                        .unwrap_or(match request.provider_type.as_str() {
                            "deepseek" => "https://api.deepseek.com/v1",
                            _ => "https://api.openai.com/v1",
                        });
                let url = format!("{}/images/generations", resolved_base.trim_end_matches('/'));
                let body = serde_json::json!({
                    "model": request.model,
                    "prompt": "ping",
                    "response_format": "b64_json"
                });
                let mut req = client.post(&url);
                req = Self::apply_provider_custom_headers(req, &custom_headers)
                    .header("Content-Type", "application/json");
                if !effective_key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {effective_key}"));
                }
                let response = req
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error reaching {url}: {e}"))?;
                Self::interpret_response(response, &["model"], ProbeSuccessKind::ImageGeneration)
                    .await
            }
            ("azure", ProviderProbeMode::ImageGeneration) => {
                let resolved_base = request.base_url.as_deref().unwrap_or(
                    "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT",
                );
                let url = format!(
                    "{}/images/generations?api-version=2024-10-21",
                    resolved_base.trim_end_matches('/'),
                );
                let body = serde_json::json!({
                    "model": request.model,
                    "prompt": "ping",
                    "response_format": "b64_json"
                });
                let mut req = client.post(&url);
                req = Self::apply_provider_custom_headers(req, &custom_headers)
                    .header("Content-Type", "application/json");
                if !effective_key.is_empty() {
                    req = req.header("api-key", effective_key.clone());
                }
                let response = req
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error reaching {url}: {e}"))?;
                Self::interpret_response(response, &["model"], ProbeSuccessKind::ImageGeneration)
                    .await
            }
            ("anthropic", ProviderProbeMode::TextChat) => {
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
                let mut req = client.post(&url);
                req = Self::apply_provider_custom_headers(req, &custom_headers)
                    .header("anthropic-version", "2023-06-01")
                    .header("Content-Type", "application/json");
                if !effective_key.is_empty() {
                    req = req.header("Authorization", format!("Bearer {effective_key}"));
                }
                let response = req
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error reaching {url}: {e}"))?;
                Self::interpret_response(response, &["model"], ProbeSuccessKind::TextChat).await
            }
            ("gemini", ProviderProbeMode::TextChat) => {
                let resolved_base = request
                    .base_url
                    .as_deref()
                    .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
                let url = if effective_key.is_empty() {
                    format!(
                        "{}/models/{}:generateContent",
                        resolved_base.trim_end_matches('/'),
                        request.model,
                    )
                } else {
                    format!(
                        "{}/models/{}:generateContent?key={}",
                        resolved_base.trim_end_matches('/'),
                        request.model,
                        effective_key
                    )
                };
                let body = serde_json::json!({
                    "contents": [{"parts": [{"text": "ping"}]}],
                    "generationConfig": {"maxOutputTokens": 1}
                });
                let req = client.post(&url);
                let response = Self::apply_provider_custom_headers(req, &custom_headers)
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("Network error reaching {url}: {e}"))?;
                Self::interpret_response(
                    response,
                    &["modelVersion", "model"],
                    ProbeSuccessKind::TextChat,
                )
                .await
            }
            (_, ProviderProbeMode::ImageGeneration) => Err(format!(
                "Active image-generation test is not yet implemented for provider type '{}'",
                request.provider_type
            )),
            _ => Ok(format!(
                "Configuration accepted (active connection test is not yet implemented \
                 for provider type '{}')",
                request.provider_type
            )),
        }
    }

    fn resolve_probe_mode(request: &ProviderTestRequest) -> ProviderProbeMode {
        match request.probe_mode.as_str() {
            "text_chat" => ProviderProbeMode::TextChat,
            "image_generation" => ProviderProbeMode::ImageGeneration,
            _ => {
                if request
                    .capabilities
                    .contains(&ProviderCapability::ImageGeneration)
                    || request.tags.iter().any(|tag| {
                        let normalized = tag.to_ascii_lowercase();
                        normalized == "image" || normalized == "image_generation"
                    })
                {
                    ProviderProbeMode::ImageGeneration
                } else {
                    ProviderProbeMode::TextChat
                }
            }
        }
    }

    /// Interpret the HTTP response from a provider test probe.
    ///
    /// On success, tries to extract the model name from the response body using
    /// the given `model_keys` (checked in order). On failure, parses the error
    /// body for a human-readable detail message.
    async fn interpret_response(
        response: reqwest::Response,
        model_keys: &[&str],
        success_kind: ProbeSuccessKind,
    ) -> Result<String, String> {
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();

        if status.is_success() {
            let model_name = serde_json::from_str::<serde_json::Value>(&body_text)
                .ok()
                .and_then(|v| {
                    model_keys
                        .iter()
                        .find_map(|k| v.get(*k).and_then(|m| m.as_str()).map(String::from))
                });
            return match model_name {
                Some(m) => Ok(match success_kind {
                    ProbeSuccessKind::TextChat => {
                        format!("Connection successful -- model '{m}' responded normally")
                    }
                    ProbeSuccessKind::ImageGeneration => format!(
                        "Connection successful -- image generation model '{m}' responded normally"
                    ),
                }),
                None => Ok(match success_kind {
                    ProbeSuccessKind::TextChat => {
                        "Connection successful -- provider responded normally".into()
                    }
                    ProbeSuccessKind::ImageGeneration => {
                        "Connection successful -- image generation endpoint responded normally"
                            .into()
                    }
                }),
            };
        }

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

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(format!("Authentication failed (HTTP {status}): {detail}"));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(format!("Rate limited by provider: {detail}"));
        }

        Err(format!("Provider returned HTTP {status}: {detail}"))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use y_core::provider::ProviderCapability;

    use super::*;

    struct SingleResponseServer {
        base_url: String,
        request_line_rx: tokio::sync::oneshot::Receiver<String>,
        header_text_rx: tokio::sync::oneshot::Receiver<String>,
        body_rx: tokio::sync::oneshot::Receiver<String>,
    }

    async fn spawn_single_response_server(
        response_body: &'static str,
    ) -> Option<SingleResponseServer> {
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!("skipping socket-based probe test in restricted sandbox: {error}");
                return None;
            }
            Err(error) => panic!("bind test listener: {error}"),
        };
        let address = listener.local_addr().expect("listener address");
        let (request_line_tx, request_line_rx) = tokio::sync::oneshot::channel();
        let (header_text_tx, header_text_rx) = tokio::sync::oneshot::channel();
        let (body_tx, body_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 2048];

            loop {
                let read = socket.read(&mut chunk).await.expect("read request");
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);

                let Some(headers_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };

                let header_text = String::from_utf8_lossy(&buffer[..headers_end]);
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);

                let body_start = headers_end + 4;
                if buffer.len() >= body_start + content_length {
                    let request_line = header_text.lines().next().unwrap_or_default().to_string();
                    let body =
                        String::from_utf8_lossy(&buffer[body_start..body_start + content_length])
                            .to_string();
                    let _ = request_line_tx.send(request_line);
                    let _ = header_text_tx.send(header_text.to_string());
                    let _ = body_tx.send(body);
                    break;
                }
            }

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        Some(SingleResponseServer {
            base_url: format!("http://{address}/v1"),
            request_line_rx,
            header_text_rx,
            body_rx,
        })
    }

    #[tokio::test]
    async fn test_provider_auto_probe_uses_chat_completions_for_text_models() {
        let Some(server) = spawn_single_response_server(r#"{"id":"x","model":"text-model"}"#).await
        else {
            return;
        };

        let result = SystemService::test_provider(ProviderTestRequest {
            provider_type: "openai-compat".into(),
            model: "text-model".into(),
            api_key: String::new(),
            api_key_env: String::new(),
            base_url: Some(server.base_url),
            headers: std::collections::HashMap::new(),
            tags: vec![],
            capabilities: vec![ProviderCapability::Text],
            probe_mode: "auto".into(),
        })
        .await
        .expect("text probe should succeed");

        assert!(server
            .request_line_rx
            .await
            .expect("request line")
            .contains("/v1/chat/completions"),);
        let body = server.body_rx.await.expect("request body");
        assert!(body.contains("\"messages\""));
        assert!(result.contains("text-model"));
    }

    #[tokio::test]
    async fn test_provider_auto_probe_uses_image_generation_for_image_models() {
        let Some(server) = spawn_single_response_server(
            r#"{"data":[{"b64_json":"aGVsbG8="}],"model":"seedream"}"#,
        )
        .await
        else {
            return;
        };

        let result = SystemService::test_provider(ProviderTestRequest {
            provider_type: "openai-compat".into(),
            model: "seedream".into(),
            api_key: String::new(),
            api_key_env: String::new(),
            base_url: Some(server.base_url),
            headers: std::collections::HashMap::new(),
            tags: vec!["image".into()],
            capabilities: vec![ProviderCapability::ImageGeneration],
            probe_mode: "auto".into(),
        })
        .await
        .expect("image probe should succeed");

        assert!(server
            .request_line_rx
            .await
            .expect("request line")
            .contains("/v1/images/generations"),);
        let body = server.body_rx.await.expect("request body");
        assert!(body.contains("\"prompt\":\"ping\""));
        assert!(body.contains("\"response_format\":\"b64_json\""));
        assert!(result.contains("Connection successful"));
    }

    #[tokio::test]
    async fn test_provider_probe_sends_custom_headers() {
        let Some(server) = spawn_single_response_server(r#"{"id":"x","model":"text-model"}"#).await
        else {
            return;
        };
        let headers = std::collections::HashMap::from([(
            "X-LLM-Tenant".to_string(),
            "workspace-a".to_string(),
        )]);

        SystemService::test_provider(ProviderTestRequest {
            provider_type: "openai-compat".into(),
            model: "text-model".into(),
            api_key: String::new(),
            api_key_env: String::new(),
            base_url: Some(server.base_url),
            headers,
            tags: vec![],
            capabilities: vec![ProviderCapability::Text],
            probe_mode: "auto".into(),
        })
        .await
        .expect("text probe should succeed");

        let header_text = server.header_text_rx.await.expect("request headers");
        assert!(header_text.contains("x-llm-tenant: workspace-a"));
    }
}
