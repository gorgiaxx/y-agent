//! Versioned provider and model compatibility profiles.

use y_core::provider::{ProviderCapability, ToolCallingMode, ToolDialect};

use crate::config::ProviderConfig;

pub const PROFILE_CATALOG_VERSION: &str = "2026-07-28";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderProfile {
    pub catalog_version: &'static str,
    pub provider_profile_id: &'static str,
    pub model_profile_id: &'static str,
    pub capabilities: Vec<ProviderCapability>,
    pub context_window: usize,
    pub include_usage: bool,
    pub use_max_completion_tokens: bool,
    pub tool_calling_mode: ToolCallingMode,
    pub tool_dialect: ToolDialect,
}

pub fn resolve(config: &ProviderConfig) -> ResolvedProviderProfile {
    let provider = provider_defaults(config);
    let model = model_defaults(&config.model);
    ResolvedProviderProfile {
        catalog_version: PROFILE_CATALOG_VERSION,
        provider_profile_id: provider.id,
        model_profile_id: model.id,
        capabilities: model.capabilities.to_vec(),
        context_window: model.context_window,
        include_usage: provider.include_usage,
        use_max_completion_tokens: model.use_max_completion_tokens,
        tool_calling_mode: provider.tool_calling_mode,
        tool_dialect: provider.tool_dialect,
    }
}

struct ProviderDefaults {
    id: &'static str,
    include_usage: bool,
    tool_calling_mode: ToolCallingMode,
    tool_dialect: ToolDialect,
}

fn provider_defaults(config: &ProviderConfig) -> ProviderDefaults {
    let endpoint = config
        .base_url
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if endpoint.contains("openrouter.ai") {
        return ProviderDefaults {
            id: "openrouter-chat-v1",
            include_usage: true,
            tool_calling_mode: ToolCallingMode::PromptBased,
            tool_dialect: ToolDialect::YAgentXml,
        };
    }
    if endpoint.contains("localhost") || endpoint.contains("127.0.0.1") {
        return ProviderDefaults {
            id: "local-openai-compatible-v1",
            include_usage: false,
            tool_calling_mode: ToolCallingMode::PromptBased,
            tool_dialect: ToolDialect::YAgentXml,
        };
    }
    match config.provider_type.as_str() {
        "openai" => ProviderDefaults {
            id: "openai-responses-v1",
            include_usage: true,
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::OpenAi,
        },
        "anthropic" => ProviderDefaults {
            id: "anthropic-messages-v1",
            include_usage: false,
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::Anthropic,
        },
        "gemini" => ProviderDefaults {
            id: "gemini-generate-content-v1beta",
            include_usage: false,
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::Gemini,
        },
        "azure" => ProviderDefaults {
            id: "azure-openai-v1",
            include_usage: true,
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::OpenAi,
        },
        "deepseek" => ProviderDefaults {
            id: "deepseek-chat-v1",
            include_usage: true,
            tool_calling_mode: ToolCallingMode::Native,
            tool_dialect: ToolDialect::OpenAi,
        },
        "ollama" => ProviderDefaults {
            id: "ollama-chat-v1",
            include_usage: false,
            tool_calling_mode: ToolCallingMode::PromptBased,
            tool_dialect: ToolDialect::YAgentXml,
        },
        _ => ProviderDefaults {
            id: "generic-openai-compatible-v1",
            include_usage: false,
            tool_calling_mode: ToolCallingMode::PromptBased,
            tool_dialect: ToolDialect::YAgentXml,
        },
    }
}

struct ModelDefaults {
    id: &'static str,
    capabilities: &'static [ProviderCapability],
    context_window: usize,
    use_max_completion_tokens: bool,
}

const TEXT: &[ProviderCapability] = &[ProviderCapability::Text];
const VISION: &[ProviderCapability] = &[ProviderCapability::Text, ProviderCapability::Vision];
const IMAGE: &[ProviderCapability] = &[ProviderCapability::ImageGeneration];

fn model_defaults(model: &str) -> ModelDefaults {
    let model = model.to_ascii_lowercase();
    if model.contains("dall-e") || model.contains("image") || model.contains("seedream") {
        return ModelDefaults {
            id: "image-generation",
            capabilities: IMAGE,
            context_window: 32_768,
            use_max_completion_tokens: false,
        };
    }
    if model.starts_with("gemini-") {
        return ModelDefaults {
            id: "gemini-multimodal",
            capabilities: VISION,
            context_window: 1_000_000,
            use_max_completion_tokens: false,
        };
    }
    if model.starts_with("claude-") {
        return ModelDefaults {
            id: "claude-multimodal",
            capabilities: VISION,
            context_window: 200_000,
            use_max_completion_tokens: false,
        };
    }
    if model.starts_with("gpt-5") || model.starts_with("o1") || model.starts_with("o3") {
        return ModelDefaults {
            id: "openai-reasoning",
            capabilities: VISION,
            context_window: 400_000,
            use_max_completion_tokens: true,
        };
    }
    if model.starts_with("gpt-4o") || model.starts_with("gpt-4.1") {
        return ModelDefaults {
            id: "openai-multimodal",
            capabilities: VISION,
            context_window: 128_000,
            use_max_completion_tokens: true,
        };
    }
    if model.contains("deepseek") {
        return ModelDefaults {
            id: "deepseek-text",
            capabilities: TEXT,
            context_window: 128_000,
            use_max_completion_tokens: false,
        };
    }
    ModelDefaults {
        id: "generic-text",
        capabilities: TEXT,
        context_window: 128_000,
        use_max_completion_tokens: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(provider_type: &str, model: &str) -> ProviderConfig {
        ProviderConfig {
            id: "test".into(),
            provider_type: provider_type.into(),
            model: model.into(),
            enabled: true,
            tags: Vec::new(),
            capabilities: Vec::new(),
            max_concurrency: 1,
            context_window: 128_000,
            max_output_tokens: None,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            api_key: Some("test".into()),
            api_key_env: None,
            base_url: None,
            headers: HashMap::new(),
            http_protocol: crate::config::HttpProtocol::Http1,
            include_usage: None,
            use_max_completion_tokens: None,
            temperature: None,
            top_p: None,
            tool_calling_mode: None,
            tool_dialect: None,
            icon: None,
            azure_resource_name: None,
            azure_api_version: None,
            azure_use_deployment_urls: None,
            azure_auth_mode: None,
        }
    }

    #[test]
    fn openai_reasoning_profile_resolves_versioned_quirks_and_vision() {
        let profile = resolve(&config("openai", "gpt-5.1"));
        assert_eq!(profile.catalog_version, PROFILE_CATALOG_VERSION);
        assert_eq!(profile.provider_profile_id, "openai-responses-v1");
        assert_eq!(profile.model_profile_id, "openai-reasoning");
        assert!(profile.use_max_completion_tokens);
        assert!(profile.capabilities.contains(&ProviderCapability::Vision));
    }

    #[test]
    fn openrouter_endpoint_selects_gateway_profile() {
        let mut config = config("openai-compat", "vendor/model");
        config.base_url = Some("https://openrouter.ai/api/v1".into());
        let profile = resolve(&config);
        assert_eq!(profile.provider_profile_id, "openrouter-chat-v1");
        assert!(profile.include_usage);
    }
}
