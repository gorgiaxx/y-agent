//! Provider pool and individual provider configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::ProviderPoolError;
use crate::router::SelectionStrategy;
use y_core::provider::{ProviderCapability, ToolCallingMode};

/// HTTP protocol preference for provider-facing requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HttpProtocol {
    /// Force HTTP/1.1 and send title-case HTTP/1.1 header names.
    #[default]
    Http1,
    /// Force HTTP/2. Header names remain lowercase per HTTP/2 requirements.
    Http2,
}

/// Configuration for the entire provider pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPoolConfig {
    /// Individual provider configurations.
    pub providers: Vec<ProviderConfig>,

    /// Multi-level proxy configuration (provider > tag > global cascade).
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Default freeze duration in seconds (before adaptive scaling).
    #[serde(default = "default_freeze_duration_secs")]
    pub default_freeze_duration_secs: u64,

    /// Maximum freeze duration in seconds (cap for exponential backoff).
    #[serde(default = "default_max_freeze_duration_secs")]
    pub max_freeze_duration_secs: u64,

    /// Health check interval in seconds for frozen providers.
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,

    /// Provider selection strategy.
    #[serde(default)]
    pub selection_strategy: SelectionStrategy,

    /// Global concurrency limit across all providers.
    /// `None` means no global limit (only per-provider limits apply).
    #[serde(default)]
    pub max_global_concurrency: Option<usize>,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique provider ID.
    pub id: String,

    /// Provider backend type.
    pub provider_type: String,

    /// Model name (e.g., "gpt-4o", "claude-3-opus").
    pub model: String,

    /// Whether this provider is enabled. Disabled providers are excluded from
    /// the pool and will not receive any requests.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Tags for routing (e.g., `["reasoning", "fast", "code"]`).
    #[serde(default = "default_tags", deserialize_with = "deserialize_tags")]
    pub tags: Vec<String>,

    /// Explicit provider capabilities used for request shaping.
    ///
    /// When empty, capabilities are derived from legacy configuration hints
    /// (primarily routing tags) for backward compatibility.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ProviderCapability>,

    /// Maximum concurrent requests to this provider.
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,

    /// Context window size in tokens.
    #[serde(default = "default_context_window")]
    pub context_window: usize,

    /// Cost per 1000 input tokens.
    #[serde(default)]
    pub cost_per_1k_input: f64,

    /// Cost per 1000 output tokens.
    #[serde(default)]
    pub cost_per_1k_output: f64,

    /// API key (written directly in config).
    /// Takes priority over `api_key_env`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Environment variable name containing the API key (fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,

    /// API base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Additional HTTP headers sent with every provider request.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,

    /// HTTP protocol used by this provider's client. Defaults to HTTP/1.1.
    #[serde(default)]
    pub http_protocol: HttpProtocol,

    /// Default sampling temperature for this provider (0.0 - 2.0).
    /// Applied to requests that do not specify a temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Default nucleus sampling top-p for this provider (0.0 - 1.0).
    /// Applied to requests that do not specify a `top_p`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Tool calling mode override for this provider.
    ///
    /// `None` means auto-detect based on `provider_type`:
    /// - `Native` for `openai`, `anthropic`, `azure`, `gemini`, `deepseek`
    /// - `PromptBased` for `openai-compat`, `custom`, `ollama`, and others
    ///
    /// See [`ProviderConfig::resolve_tool_calling_mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calling_mode: Option<ToolCallingMode>,

    /// Optional icon identifier for GUI display (e.g. "openai", "anthropic").
    /// Matches icon IDs from the @lobehub/icons library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Multi-level proxy configuration.
///
/// Cascade priority: provider-specific > tag-level > global.
/// Default protocol is SOCKS5 when no scheme is specified in the URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    /// Default scheme prepended when proxy URL has no scheme.
    /// Supported: "socks5", "socks5h", "http", "https".
    /// Default: "socks5".
    #[serde(default = "default_proxy_scheme")]
    pub default_scheme: String,

    /// Global proxy (lowest priority fallback).
    pub global: Option<ProxyEntry>,

    /// Per-tag proxy overrides.
    pub tags: HashMap<String, ProxyEntry>,

    /// Per-provider proxy overrides (highest priority).
    pub providers: HashMap<String, ProxyEntry>,
}

/// A single proxy endpoint entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyEntry {
    /// Proxy URL (e.g., `socks5://host:1080`, `http://host:8080`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Whether proxy is enabled. Set `false` to bypass proxy for local providers.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Environment variable name containing proxy credentials in `username:password` format.
    /// If set, the credentials are applied to the proxy URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_env: Option<String>,
}

/// Resolved proxy specification containing the URL and optional authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySpec {
    /// The normalized proxy URL.
    pub url: String,
    /// Optional proxy credentials `(username, password)`.
    pub auth: Option<(String, String)>,
}

fn default_true() -> bool {
    true
}

fn default_tags() -> Vec<String> {
    vec!["general".to_string()]
}

fn deserialize_tags<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tags: Vec<String> = serde::Deserialize::deserialize(deserializer)?;
    if tags.is_empty() {
        Ok(default_tags())
    } else {
        Ok(tags)
    }
}

fn default_proxy_scheme() -> String {
    "socks5".to_string()
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            default_scheme: default_proxy_scheme(),
            global: None,
            tags: HashMap::new(),
            providers: HashMap::new(),
        }
    }
}

fn default_freeze_duration_secs() -> u64 {
    30
}

fn default_max_freeze_duration_secs() -> u64 {
    3600
}

fn default_health_check_interval_secs() -> u64 {
    60
}

fn default_max_concurrency() -> usize {
    5
}

fn default_context_window() -> usize {
    128_000
}

impl Default for ProviderPoolConfig {
    fn default() -> Self {
        Self {
            providers: Vec::new(),
            proxy: ProxyConfig::default(),
            default_freeze_duration_secs: default_freeze_duration_secs(),
            max_freeze_duration_secs: default_max_freeze_duration_secs(),
            health_check_interval_secs: default_health_check_interval_secs(),
            selection_strategy: SelectionStrategy::default(),
            max_global_concurrency: None,
        }
    }
}

impl ProviderPoolConfig {
    /// Validate the pool configuration.
    pub fn validate(&self) -> Result<(), ProviderPoolError> {
        if self.providers.is_empty() {
            return Err(ProviderPoolError::Config {
                message: "at least one provider must be configured".into(),
            });
        }

        // Check for duplicate IDs.
        let mut seen_ids = std::collections::HashSet::new();
        for p in &self.providers {
            if !seen_ids.insert(&p.id) {
                return Err(ProviderPoolError::DuplicateProvider { id: p.id.clone() });
            }
            crate::http_headers::custom_header_map(&p.headers).map_err(|message| {
                ProviderPoolError::Config {
                    message: format!("provider '{}' has invalid custom header: {message}", p.id),
                }
            })?;
        }

        Ok(())
    }

    /// Resolve the effective proxy URL for a given provider.
    ///
    /// Cascade: provider-specific > tag-level (first match) > global.
    /// Returns `None` if proxy is disabled or not configured at any level.
    ///
    /// URLs without a scheme (e.g., `proxy.company.com:1080`) are normalized
    /// by prepending the `default_scheme` (default: `socks5`).
    pub fn resolve_proxy_url(&self, provider_id: &str, tags: &[String]) -> Option<String> {
        // 1. Provider-level override (highest priority).
        if let Some(entry) = self.proxy.providers.get(provider_id) {
            if !entry.enabled {
                return None;
            }
            if let Some(ref url) = entry.url {
                return Some(self.normalize_proxy_url(url));
            }
        }

        // 2. Tag-level override (first matching tag).
        for tag in tags {
            if let Some(entry) = self.proxy.tags.get(tag) {
                if !entry.enabled {
                    return None;
                }
                if let Some(ref url) = entry.url {
                    return Some(self.normalize_proxy_url(url));
                }
            }
        }

        // 3. Global fallback (lowest priority).
        if let Some(ref global) = self.proxy.global {
            if !global.enabled {
                return None;
            }
            if let Some(ref url) = global.url {
                return Some(self.normalize_proxy_url(url));
            }
        }

        None
    }

    /// Normalize a proxy URL by prepending the default scheme if no scheme is present.
    fn normalize_proxy_url(&self, url: &str) -> String {
        if url.contains("://") {
            url.to_string()
        } else {
            format!("{}://{}", self.proxy.default_scheme, url)
        }
    }

    /// Resolve the effective proxy specification (URL + auth) for a given provider.
    ///
    /// Same cascade as `resolve_proxy_url()` but also resolves `auth_env`
    /// credentials from the environment.
    pub fn resolve_proxy_spec(&self, provider_id: &str, tags: &[String]) -> Option<ProxySpec> {
        // Helper to resolve auth from an entry.
        fn resolve_auth(entry: &ProxyEntry) -> Option<(String, String)> {
            entry.auth_env.as_ref().and_then(|env_var| {
                std::env::var(env_var).ok().and_then(|val| {
                    let (user, pass) = val.split_once(':')?;
                    Some((user.to_string(), pass.to_string()))
                })
            })
        }

        // 1. Provider-level override (highest priority).
        if let Some(entry) = self.proxy.providers.get(provider_id) {
            if !entry.enabled {
                return None;
            }
            if let Some(ref url) = entry.url {
                return Some(ProxySpec {
                    url: self.normalize_proxy_url(url),
                    auth: resolve_auth(entry),
                });
            }
        }

        // 2. Tag-level override (first matching tag).
        for tag in tags {
            if let Some(entry) = self.proxy.tags.get(tag) {
                if !entry.enabled {
                    return None;
                }
                if let Some(ref url) = entry.url {
                    return Some(ProxySpec {
                        url: self.normalize_proxy_url(url),
                        auth: resolve_auth(entry),
                    });
                }
            }
        }

        // 3. Global fallback (lowest priority).
        if let Some(ref global) = self.proxy.global {
            if !global.enabled {
                return None;
            }
            if let Some(ref url) = global.url {
                return Some(ProxySpec {
                    url: self.normalize_proxy_url(url),
                    auth: resolve_auth(global),
                });
            }
        }

        None
    }
}

impl ProviderConfig {
    /// Resolve the API key.
    ///
    /// Priority: `api_key` (direct) > `api_key_env` (environment variable).
    pub fn resolve_api_key(&self) -> Option<String> {
        // 1. Direct key in config.
        if let Some(ref key) = self.api_key {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }

        // 2. Environment variable fallback.
        self.api_key_env
            .as_ref()
            .and_then(|env_var| std::env::var(env_var).ok())
    }

    /// Resolve the effective tool calling mode for this provider.
    ///
    /// Priority: explicit `tool_calling_mode` override > auto-detect from
    /// `provider_type`.
    ///
    /// First-party providers (`openai`, `anthropic`, `azure`, `gemini`,
    /// `deepseek`) default to [`ToolCallingMode::Native`] because their APIs
    /// support structured tool call fields.
    ///
    /// Compatibility/local providers (`openai-compat`, `custom`, `ollama`)
    /// default to [`ToolCallingMode::PromptBased`] because many relay APIs
    /// and local models do not reliably support native tool calling.
    pub fn resolve_tool_calling_mode(&self) -> ToolCallingMode {
        if let Some(mode) = self.tool_calling_mode {
            return mode;
        }
        match self.provider_type.as_str() {
            "openai" | "anthropic" | "azure" | "gemini" | "deepseek" => ToolCallingMode::Native,
            // openai-compat, custom, ollama, and any unknown type.
            _ => ToolCallingMode::PromptBased,
        }
    }

    /// Resolve the effective provider capabilities.
    ///
    /// Priority: explicit `capabilities` > legacy tag-based inference.
    pub fn resolve_capabilities(&self) -> Vec<ProviderCapability> {
        if !self.capabilities.is_empty() {
            return self.capabilities.clone();
        }

        let tag_set = self
            .tags
            .iter()
            .map(|tag| tag.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();

        let has_image_generation =
            tag_set.contains("image") || tag_set.contains("image_generation");
        let has_vision = tag_set.contains("vision");

        let mut capabilities = Vec::new();
        if has_image_generation {
            capabilities.push(ProviderCapability::ImageGeneration);
        }
        if has_vision {
            capabilities.push(ProviderCapability::Text);
            capabilities.push(ProviderCapability::Vision);
        }
        if capabilities.is_empty() {
            capabilities.push(ProviderCapability::Text);
        }

        capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_deserialize_from_toml() {
        let toml_str = r#"
            default_freeze_duration_secs = 60

            [[providers]]
            id = "openai-gpt4"
            provider_type = "openai"
            model = "gpt-4o"
            tags = ["reasoning", "general"]
            max_concurrency = 10
            context_window = 128000
            api_key_env = "OPENAI_API_KEY"

            [[providers]]
            id = "anthropic-claude"
            provider_type = "anthropic"
            model = "claude-3-opus"
            tags = ["reasoning", "code"]
            api_key_env = "ANTHROPIC_API_KEY"
        "#;
        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse TOML");
        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.providers[0].id, "openai-gpt4");
        assert_eq!(config.providers[0].tags, vec!["reasoning", "general"]);
        assert_eq!(config.providers[1].model, "claude-3-opus");
        assert_eq!(config.default_freeze_duration_secs, 60);
    }

    #[test]
    fn test_config_validate_no_providers_fails() {
        let config = ProviderPoolConfig::default();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validate_duplicate_ids_fails() {
        let config = ProviderPoolConfig {
            providers: vec![
                ProviderConfig {
                    id: "dup".into(),
                    provider_type: "openai".into(),
                    model: "gpt-4".into(),
                    tags: vec![],
                    capabilities: vec![],
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    headers: HashMap::new(),
                    http_protocol: HttpProtocol::Http1,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                    enabled: true,
                },
                ProviderConfig {
                    id: "dup".into(),
                    provider_type: "anthropic".into(),
                    model: "claude".into(),
                    tags: vec![],
                    capabilities: vec![],
                    max_concurrency: 5,
                    context_window: 200_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    headers: HashMap::new(),
                    http_protocol: HttpProtocol::Http1,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                    enabled: true,
                },
            ],
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("dup"));
    }

    #[test]
    fn test_config_default_concurrency() {
        let toml_str = r#"
            [[providers]]
            id = "test"
            provider_type = "openai"
            model = "gpt-4"
        "#;
        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("parse");
        assert_eq!(config.providers[0].max_concurrency, 5);
    }

    #[test]
    fn test_config_api_key_from_env() {
        let config = ProviderConfig {
            id: "test".into(),
            provider_type: "openai".into(),
            model: "gpt-4".into(),
            enabled: true,
            tags: vec![],
            capabilities: vec![],
            max_concurrency: 5,
            context_window: 128_000,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: None,
            api_key_env: Some("Y_AGENT_TEST_KEY_XYZ".into()),
            base_url: None,
            headers: HashMap::new(),
            http_protocol: HttpProtocol::Http1,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            icon: None,
        };

        // Without env var set, should return None.
        assert!(config.resolve_api_key().is_none());

        // With env var set.
        temp_env::with_var("Y_AGENT_TEST_KEY_XYZ", Some("sk-test-123"), || {
            assert_eq!(config.resolve_api_key(), Some("sk-test-123".into()));
        });
    }

    #[test]
    fn test_config_direct_api_key() {
        let config = ProviderConfig {
            id: "test".into(),
            provider_type: "openai".into(),
            model: "gpt-4".into(),
            enabled: true,
            tags: vec![],
            capabilities: vec![],
            max_concurrency: 5,
            context_window: 128_000,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: Some("sk-direct-key".into()),
            api_key_env: Some("Y_AGENT_TEST_KEY_DIRECT".into()),
            base_url: None,
            headers: HashMap::new(),
            http_protocol: HttpProtocol::Http1,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            icon: None,
        };

        // Direct key takes priority over env var.
        temp_env::with_var("Y_AGENT_TEST_KEY_DIRECT", Some("sk-env-key"), || {
            assert_eq!(config.resolve_api_key(), Some("sk-direct-key".into()));
        });
    }

    // -----------------------------------------------------------------------
    // Proxy configuration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_proxy_config_deserialize() {
        let toml_str = r#"
            [[providers]]
            id = "test"
            provider_type = "openai"
            model = "gpt-4"

            [proxy.global]
            url = "socks5://proxy.company.com:1080"

            [proxy.tags.china]
            url = "http://cn-proxy.company.com:8080"

            [proxy.providers.ollama-local]
            enabled = false
        "#;
        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse");

        let global = config.proxy.global.as_ref().expect("global proxy");
        assert_eq!(
            global.url.as_deref(),
            Some("socks5://proxy.company.com:1080")
        );
        assert!(global.enabled);

        let china = config.proxy.tags.get("china").expect("china tag proxy");
        assert_eq!(
            china.url.as_deref(),
            Some("http://cn-proxy.company.com:8080")
        );

        let ollama = config
            .proxy
            .providers
            .get("ollama-local")
            .expect("ollama proxy");
        assert!(!ollama.enabled);
    }

    #[test]
    fn test_proxy_resolve_provider_override() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("http://global:8080".into()),
            enabled: true,
            auth_env: None,
        });
        config.proxy.providers.insert(
            "special".into(),
            ProxyEntry {
                url: Some("socks5://special:1080".into()),
                enabled: true,
                auth_env: None,
            },
        );

        // Provider-level should override global.
        let result = config.resolve_proxy_url("special", &["general".into()]);
        assert_eq!(result.as_deref(), Some("socks5://special:1080"));
    }

    #[test]
    fn test_proxy_resolve_tag_override() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("http://global:8080".into()),
            enabled: true,
            auth_env: None,
        });
        config.proxy.tags.insert(
            "china".into(),
            ProxyEntry {
                url: Some("http://cn:8080".into()),
                enabled: true,
                auth_env: None,
            },
        );

        // Tag-level should override global for providers with that tag.
        let result = config.resolve_proxy_url("some-provider", &["china".into()]);
        assert_eq!(result.as_deref(), Some("http://cn:8080"));
    }

    #[test]
    fn test_proxy_resolve_global_fallback() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("http://global:8080".into()),
            enabled: true,
            auth_env: None,
        });

        let result = config.resolve_proxy_url("any", &["unmatched".into()]);
        assert_eq!(result.as_deref(), Some("http://global:8080"));
    }

    #[test]
    fn test_proxy_resolve_disabled() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("http://global:8080".into()),
            enabled: true,
            auth_env: None,
        });
        config.proxy.providers.insert(
            "local".into(),
            ProxyEntry {
                url: None,
                enabled: false,
                auth_env: None,
            },
        );

        // Provider with enabled=false should bypass all proxy levels.
        let result = config.resolve_proxy_url("local", &["general".into()]);
        assert!(result.is_none());
    }

    #[test]
    fn test_proxy_resolve_none() {
        let config = ProviderPoolConfig::default();
        let result = config.resolve_proxy_url("any", &["any".into()]);
        assert!(result.is_none());
    }

    #[test]
    fn test_proxy_default_scheme_is_socks5() {
        let config = ProviderPoolConfig::default();
        assert_eq!(config.proxy.default_scheme, "socks5");
    }

    #[test]
    fn test_proxy_normalize_url_without_scheme() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("proxy.company.com:1080".into()),
            enabled: true,
            auth_env: None,
        });

        let result = config.resolve_proxy_url("any", &["any".into()]);
        assert_eq!(result.as_deref(), Some("socks5://proxy.company.com:1080"));
    }

    #[test]
    fn test_proxy_normalize_url_preserves_explicit_scheme() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("http://proxy.company.com:8080".into()),
            enabled: true,
            auth_env: None,
        });

        let result = config.resolve_proxy_url("any", &["any".into()]);
        assert_eq!(result.as_deref(), Some("http://proxy.company.com:8080"));
    }

    #[test]
    fn test_proxy_custom_default_scheme() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.default_scheme = "socks5h".into();
        config.proxy.global = Some(ProxyEntry {
            url: Some("proxy.company.com:1080".into()),
            enabled: true,
            auth_env: None,
        });

        let result = config.resolve_proxy_url("any", &["any".into()]);
        assert_eq!(result.as_deref(), Some("socks5h://proxy.company.com:1080"));
    }

    #[test]
    fn test_proxy_normalize_in_provider_override() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.providers.insert(
            "special".into(),
            ProxyEntry {
                url: Some("10.0.0.1:1080".into()),
                enabled: true,
                auth_env: None,
            },
        );

        let result = config.resolve_proxy_url("special", &[]);
        assert_eq!(result.as_deref(), Some("socks5://10.0.0.1:1080"));
    }

    #[test]
    fn test_proxy_normalize_in_tag_override() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.tags.insert(
            "china".into(),
            ProxyEntry {
                url: Some("cn-proxy.local:1080".into()),
                enabled: true,
                auth_env: None,
            },
        );

        let result = config.resolve_proxy_url("any", &["china".into()]);
        assert_eq!(result.as_deref(), Some("socks5://cn-proxy.local:1080"));
    }

    #[test]
    fn test_proxy_config_deserialize_with_default_scheme() {
        let toml_str = r#"
            [[providers]]
            id = "test"
            provider_type = "openai"
            model = "gpt-4"

            [proxy]
            default_scheme = "socks5h"

            [proxy.global]
            url = "proxy.company.com:1080"
        "#;
        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse");
        assert_eq!(config.proxy.default_scheme, "socks5h");

        let result = config.resolve_proxy_url("test", &[]);
        assert_eq!(result.as_deref(), Some("socks5h://proxy.company.com:1080"));
    }

    #[test]
    fn test_proxy_auth_env_resolve() {
        let mut config = ProviderPoolConfig::default();
        config.proxy.global = Some(ProxyEntry {
            url: Some("socks5://proxy.company.com:1080".into()),
            enabled: true,
            auth_env: Some("Y_AGENT_TEST_PROXY_AUTH".into()),
        });

        // Without env var set — spec should have auth = None.
        let spec = config
            .resolve_proxy_spec("any", &["any".into()])
            .expect("should resolve");
        assert_eq!(spec.url, "socks5://proxy.company.com:1080");
        assert!(spec.auth.is_none());

        // With env var set — spec should resolve credentials.
        temp_env::with_var("Y_AGENT_TEST_PROXY_AUTH", Some("user1:secret123"), || {
            let spec = config
                .resolve_proxy_spec("any", &["any".into()])
                .expect("should resolve");
            assert_eq!(spec.url, "socks5://proxy.company.com:1080");
            assert_eq!(spec.auth, Some(("user1".into(), "secret123".into())));
        });
    }

    #[test]
    fn test_proxy_auth_env_deserialize() {
        let toml_str = r#"
            [[providers]]
            id = "test"
            provider_type = "openai"
            model = "gpt-4"

            [proxy.global]
            url = "socks5://proxy.company.com:1080"
            auth_env = "MY_PROXY_CREDS"
        "#;
        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse");
        let global = config.proxy.global.as_ref().expect("global proxy");
        assert_eq!(global.auth_env.as_deref(), Some("MY_PROXY_CREDS"));
    }

    #[test]
    fn test_provider_config_deserializes_custom_headers() {
        let toml_str = r#"
            [[providers]]
            id = "gateway"
            provider_type = "openai-compat"
            model = "gateway-model"

            [providers.headers]
            "X-LLM-Tenant" = "workspace-a"
            "HTTP-Referer" = "https://y-agent.local"
        "#;

        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse");
        let headers = &config.providers[0].headers;

        assert_eq!(
            headers.get("X-LLM-Tenant").map(String::as_str),
            Some("workspace-a")
        );
        assert_eq!(
            headers.get("HTTP-Referer").map(String::as_str),
            Some("https://y-agent.local")
        );
    }

    #[test]
    fn test_provider_config_rejects_invalid_custom_header() {
        let toml_str = r#"
            [[providers]]
            id = "gateway"
            provider_type = "openai-compat"
            model = "gateway-model"

            [providers.headers]
            "Bad Header" = "value"
        "#;

        let config: ProviderPoolConfig = toml::from_str(toml_str).expect("should parse");
        let err = config
            .validate()
            .expect_err("invalid header should fail validation");

        assert!(err.to_string().contains("invalid custom header"));
        assert!(err.to_string().contains("Bad Header"));
    }

    // -----------------------------------------------------------------------
    // Tool calling mode auto-detection tests
    // -----------------------------------------------------------------------

    fn make_provider_config(provider_type: &str) -> ProviderConfig {
        ProviderConfig {
            id: "test".into(),
            provider_type: provider_type.into(),
            model: "test".into(),
            enabled: true,
            tags: vec![],
            capabilities: vec![],
            max_concurrency: 5,
            context_window: 128_000,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: None,
            api_key_env: None,
            base_url: None,
            headers: HashMap::new(),
            http_protocol: HttpProtocol::Http1,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            icon: None,
        }
    }

    #[test]
    fn test_resolve_tool_calling_mode_native_providers() {
        for pt in ["openai", "anthropic", "azure", "gemini", "deepseek"] {
            let cfg = make_provider_config(pt);
            assert_eq!(
                cfg.resolve_tool_calling_mode(),
                ToolCallingMode::Native,
                "provider_type={pt} should default to Native"
            );
        }
    }

    #[test]
    fn test_resolve_tool_calling_mode_prompt_based_providers() {
        for pt in ["openai-compat", "custom", "ollama", "unknown"] {
            let cfg = make_provider_config(pt);
            assert_eq!(
                cfg.resolve_tool_calling_mode(),
                ToolCallingMode::PromptBased,
                "provider_type={pt} should default to PromptBased"
            );
        }
    }

    #[test]
    fn test_resolve_tool_calling_mode_explicit_override() {
        let mut cfg = make_provider_config("ollama");
        cfg.tool_calling_mode = Some(ToolCallingMode::Native);
        assert_eq!(cfg.resolve_tool_calling_mode(), ToolCallingMode::Native);

        let mut cfg = make_provider_config("openai");
        cfg.tool_calling_mode = Some(ToolCallingMode::PromptBased);
        assert_eq!(
            cfg.resolve_tool_calling_mode(),
            ToolCallingMode::PromptBased
        );
    }

    #[test]
    fn test_resolve_capabilities_defaults_to_text() {
        let cfg = make_provider_config("openai");
        assert_eq!(cfg.resolve_capabilities(), vec![ProviderCapability::Text]);
    }

    #[test]
    fn test_resolve_capabilities_infers_image_generation_from_tags() {
        let mut cfg = make_provider_config("openai-compat");
        cfg.tags = vec!["image".into()];
        assert_eq!(
            cfg.resolve_capabilities(),
            vec![ProviderCapability::ImageGeneration]
        );
    }

    #[test]
    fn test_resolve_capabilities_prefers_explicit_capabilities() {
        let mut cfg = make_provider_config("openai-compat");
        cfg.tags = vec!["image".into()];
        cfg.capabilities = vec![ProviderCapability::Text, ProviderCapability::Vision];
        assert_eq!(
            cfg.resolve_capabilities(),
            vec![ProviderCapability::Text, ProviderCapability::Vision]
        );
    }
}
