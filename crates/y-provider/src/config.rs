//! Provider pool and individual provider configuration.

use serde::Deserialize;

use crate::error::ProviderPoolError;

/// Configuration for the entire provider pool.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPoolConfig {
    /// Individual provider configurations.
    pub providers: Vec<ProviderConfig>,

    /// Default freeze duration in seconds (before adaptive scaling).
    #[serde(default = "default_freeze_duration_secs")]
    pub default_freeze_duration_secs: u64,

    /// Maximum freeze duration in seconds (cap for exponential backoff).
    #[serde(default = "default_max_freeze_duration_secs")]
    pub max_freeze_duration_secs: u64,

    /// Health check interval in seconds for frozen providers.
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
}

/// Configuration for a single LLM provider.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Unique provider ID.
    pub id: String,

    /// Provider backend type.
    pub provider_type: String,

    /// Model name (e.g., "gpt-4o", "claude-3-opus").
    pub model: String,

    /// Tags for routing (e.g., ["reasoning", "fast", "code"]).
    #[serde(default)]
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

    /// Environment variable name containing the API key.
    pub api_key_env: Option<String>,

    /// API base URL override.
    pub base_url: Option<String>,
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
            default_freeze_duration_secs: default_freeze_duration_secs(),
            max_freeze_duration_secs: default_max_freeze_duration_secs(),
            health_check_interval_secs: default_health_check_interval_secs(),
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
                return Err(ProviderPoolError::DuplicateProvider {
                    id: p.id.clone(),
                });
            }
        }

        Ok(())
    }
}

impl ProviderConfig {
    /// Resolve the API key from the environment variable.
    pub fn resolve_api_key(&self) -> Option<String> {
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
                    api_key_env: None,
                    base_url: None,
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
                    api_key_env: None,
                    base_url: None,
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
            api_key_env: Some("Y_AGENT_TEST_KEY_XYZ".into()),
            base_url: None,
        };

        // Without env var set, should return None.
        assert!(config.resolve_api_key().is_none());

        // With env var set.
        std::env::set_var("Y_AGENT_TEST_KEY_XYZ", "sk-test-123");
        assert_eq!(config.resolve_api_key(), Some("sk-test-123".into()));
        std::env::remove_var("Y_AGENT_TEST_KEY_XYZ");
    }
}
