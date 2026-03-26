//! Dependency wiring: thin delegation to `y-service::ServiceContainer`.
//!
//! All service construction logic now lives in `y-service`. This module
//! re-exports `ServiceContainer` as `AppServices` for backward compatibility
//! within `y-cli` and handles the conversion from `YAgentConfig` (CLI-specific)
//! to `ServiceConfig` (service-layer).

use anyhow::Result;

use crate::config::YAgentConfig;

// Re-export ServiceContainer as AppServices for backward compatibility.
pub use y_service::ServiceContainer as AppServices;

/// Wire all services from a CLI configuration.
///
/// Converts `YAgentConfig` → `ServiceConfig` and delegates to
/// `ServiceContainer::from_config()`.
pub async fn wire(config: &YAgentConfig) -> Result<AppServices> {
    // Derive prompts override directory from the user config directory.
    let prompts_dir = crate::config::dirs_user_config()
        .map(|d| d.join("prompts"))
        .filter(|d| d.is_dir());

    // Derive skills store directory from the user config directory.
    let skills_dir = crate::config::dirs_user_config().map(|d| d.join("skills"));

    let service_config = y_service::ServiceConfig {
        providers: config.providers.clone(),
        storage: config.storage.clone(),
        session: config.session.clone(),
        runtime: config.runtime.clone(),
        hooks: config.hooks.clone(),
        tools: config.tools.clone(),
        guardrails: config.guardrails.clone(),
        browser: config.browser.clone(),
        knowledge: config.knowledge.clone(),
        pruning: config.pruning.clone(),
        prompts_dir,
        skills_dir,
    };

    AppServices::from_config(&service_config).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-002-01: test_wire_creates_all_services
    #[tokio::test]
    async fn test_wire_creates_all_services() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let result = wire(&config).await;
        assert!(result.is_ok(), "wiring with default config should succeed");

        let services = result.unwrap();
        let _ = services.provider_pool().await;
        let _ = &services.session_manager;
        let _ = &services.hook_system;
        let _ = &services.tool_registry;
        let _ = &services.runtime_manager;
        let _ = &services.context_pipeline;
        let _ = &services.guardrail_manager;
        let _ = &services.agent_pool;
        let _ = &services.prompt_context;
    }

    // T-CLI-002-02: test_wire_registers_middleware
    #[tokio::test]
    async fn test_wire_registers_middleware() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let services = wire(&config).await.unwrap();
        let _tool_guard = services.guardrail_manager.tool_guard();
        let _loop_detector = services.guardrail_manager.loop_detector();
        let _llm_guard = services.guardrail_manager.llm_guard();
    }

    // T-CLI-002-03: test_build_providers_skips_missing_key
    #[test]
    fn test_build_providers_skips_missing_key() {
        let pool_config = y_service::ProviderPoolConfig {
            providers: vec![y_service::ProviderConfig {
                id: "test-no-key".into(),
                provider_type: "openai".into(),
                model: "gpt-4".into(),
                enabled: true,
                tags: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_NONEXISTENT_KEY_12345".into()),
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
                icon: None,
            }],
            ..Default::default()
        };
        let providers = y_service::container::build_providers_from_config(&pool_config);
        assert!(providers.is_empty());
    }

    // T-CLI-002-04: test_build_providers_skips_unsupported_type
    #[test]
    fn test_build_providers_skips_unsupported_type() {
        std::env::set_var("Y_AGENT_TEST_WIRE_KEY", "test-key");

        let pool_config = y_service::ProviderPoolConfig {
            providers: vec![y_service::ProviderConfig {
                id: "test-unsupported".into(),
                provider_type: "unsupported_backend".into(),
                model: "some-model".into(),
                enabled: true,
                tags: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_TEST_WIRE_KEY".into()),
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
                icon: None,
            }],
            ..Default::default()
        };
        let providers = y_service::container::build_providers_from_config(&pool_config);
        assert!(providers.is_empty());

        std::env::remove_var("Y_AGENT_TEST_WIRE_KEY");
    }

    // T-CLI-002-05: test_wire_registers_context_providers
    #[tokio::test]
    async fn test_wire_registers_context_providers() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let services = wire(&config).await.unwrap();
        assert_eq!(services.context_pipeline.provider_count(), 5);
    }
}
