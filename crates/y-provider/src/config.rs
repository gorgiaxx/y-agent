//! Provider pool and individual provider configuration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::ProviderPoolError;
use crate::router::SelectionStrategy;
use y_core::provider::ToolCallingMode;

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

    /// Tags for routing (e.g., `["reasoning", "fast", "code"]`).
    #[serde(default = "default_tags", deserialize_with = "deserialize_tags")]
    pub tags: Vec<String>,

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

    /// Default sampling temperature for this provider (0.0 - 2.0).
    /// Applied to requests that do not specify a temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    /// Default nucleus sampling top-p for this provider (0.0 - 1.0).
    /// Applied to requests that do not specify a `top_p`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    /// Tool calling mode override for this provider.
    /// `None` means use the global default (Native).
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
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
                ProviderConfig {
                    id: "dup".into(),
                    provider_type: "anthropic".into(),
                    model: "claude".into(),
                    tags: vec![],
                    max_concurrency: 5,
                    context_window: 200_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
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
            tags: vec![],
            max_concurrency: 5,
            context_window: 128_000,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: None,
            api_key_env: Some("Y_AGENT_TEST_KEY_XYZ".into()),
            base_url: None,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            icon: None,
        };

        // Without env var set, should return None.
        assert!(config.resolve_api_key().is_none());

        // With env var set.
        std::env::set_var("Y_AGENT_TEST_KEY_XYZ", "sk-test-123");
        assert_eq!(config.resolve_api_key(), Some("sk-test-123".into()));
        std::env::remove_var("Y_AGENT_TEST_KEY_XYZ");
    }

    #[test]
    fn test_config_direct_api_key() {
        let config = ProviderConfig {
            id: "test".into(),
            provider_type: "openai".into(),
            model: "gpt-4".into(),
            tags: vec![],
            max_concurrency: 5,
            context_window: 128_000,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: Some("sk-direct-key".into()),
            api_key_env: Some("Y_AGENT_TEST_KEY_DIRECT".into()),
            base_url: None,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            icon: None,
        };

        // Direct key takes priority over env var.
        std::env::set_var("Y_AGENT_TEST_KEY_DIRECT", "sk-env-key");
        assert_eq!(config.resolve_api_key(), Some("sk-direct-key".into()));
        std::env::remove_var("Y_AGENT_TEST_KEY_DIRECT");
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
        std::env::set_var("Y_AGENT_TEST_PROXY_AUTH", "user1:secret123");
        let spec = config
            .resolve_proxy_spec("any", &["any".into()])
            .expect("should resolve");
        assert_eq!(spec.url, "socks5://proxy.company.com:1080");
        assert_eq!(spec.auth, Some(("user1".into(), "secret123".into())));
        std::env::remove_var("Y_AGENT_TEST_PROXY_AUTH");
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
}
