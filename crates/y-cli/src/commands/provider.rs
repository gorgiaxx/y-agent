//! `provider` CLI command — provider management.
//!
//! Subcommands:
//! - `list` — list all configured providers
//! - `test` — test a provider configuration
//! - `models` — list available models from a provider
//! - `thaw` — thaw all frozen providers

use std::collections::HashMap;

use anyhow::Result;
use clap::Subcommand;

use crate::output::{self, OutputMode, TableRow};

/// Provider subcommands.
#[derive(Debug, Subcommand)]
pub enum ProviderAction {
    /// List all configured providers.
    List,

    /// Test a provider configuration.
    Test {
        /// Provider type (e.g. "openai", "anthropic", "ollama").
        #[arg(long)]
        provider_type: String,

        /// Model name to test.
        #[arg(long)]
        model: String,

        /// Direct API key value.
        #[arg(long)]
        api_key: Option<String>,

        /// Environment variable name holding the API key.
        #[arg(long)]
        api_key_env: Option<String>,

        /// Optional base URL override.
        #[arg(long)]
        base_url: Option<String>,
    },

    /// List available models from a provider.
    Models {
        /// Base URL of the provider's API.
        #[arg(long)]
        base_url: String,

        /// Direct API key value.
        #[arg(long)]
        api_key: Option<String>,

        /// Environment variable name holding the API key.
        #[arg(long)]
        api_key_env: Option<String>,
    },

    /// Thaw all frozen providers.
    Thaw,
}

/// Run a provider subcommand.
pub async fn run(
    action: &ProviderAction,
    services: &y_service::ServiceContainer,
    _mode: OutputMode,
) -> Result<()> {
    match action {
        ProviderAction::List => {
            let providers = y_service::SystemService::list_providers(services).await;

            if providers.is_empty() {
                output::print_info("No providers configured");
            } else {
                output::print_info(&format!("{} provider(s):", providers.len()));
                let headers = &["ID", "Model", "Type", "Capabilities"];
                let rows: Vec<TableRow> = providers
                    .iter()
                    .map(|p| TableRow {
                        cells: vec![
                            p.id.clone(),
                            p.model.clone(),
                            p.provider_type.clone(),
                            format!("{:?}", p.capabilities),
                        ],
                    })
                    .collect();
                let table = output::format_table(headers, &rows);
                print!("{table}");
            }
        }

        ProviderAction::Test {
            provider_type,
            model,
            api_key,
            api_key_env,
            base_url,
        } => {
            let request = y_service::ProviderTestRequest {
                provider_type: provider_type.clone(),
                model: model.clone(),
                api_key: api_key.clone().unwrap_or_default(),
                api_key_env: api_key_env.clone().unwrap_or_default(),
                base_url: base_url.clone(),
                headers: HashMap::new(),
                http_protocol: y_service::HttpProtocol::default(),
                tags: vec![],
                capabilities: vec![],
                probe_mode: "auto".to_string(),
            };

            output::print_info(&format!(
                "Testing provider '{provider_type}' with model '{model}'..."
            ));

            match y_service::SystemService::test_provider(request).await {
                Ok(detail) => {
                    output::print_success(&format!("Provider test passed: {detail}"));
                }
                Err(e) => {
                    output::print_error(&format!("Provider test failed: {e}"));
                }
            }
        }

        ProviderAction::Models {
            base_url,
            api_key,
            api_key_env,
        } => {
            let key = api_key.clone().unwrap_or_default();
            let key_env = api_key_env.clone().unwrap_or_default();

            output::print_info(&format!("Fetching models from '{base_url}'..."));

            match y_service::list_provider_models(
                base_url,
                &key,
                &key_env,
                None::<&HashMap<String, String>>,
                y_service::HttpProtocol::default(),
                None,
                None,
                None,
                None,
            )
            .await
            {
                Ok(value) => {
                    // Extract model IDs from the common OpenAI-compatible
                    // response shape: { "data": [{ "id": "..." }, ...] }.
                    let models: Vec<String> = value
                        .get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    m.get("id")
                                        .and_then(|i| i.as_str())
                                        .map(std::string::ToString::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    if models.is_empty() {
                        output::print_info("No models returned (or unrecognized response format)");
                        // Print the raw JSON so the user can inspect it.
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        );
                    } else {
                        output::print_info(&format!("{} model(s) available:", models.len()));
                        for m in &models {
                            println!("  {m}");
                        }
                    }
                }
                Err(e) => {
                    output::print_error(&format!("Failed to list models: {e}"));
                }
            }
        }

        ProviderAction::Thaw => {
            let count = y_service::SystemService::thaw_frozen_providers(services).await;
            if count > 0 {
                output::print_success(&format!("Thawed {count} frozen provider(s)"));
            } else {
                output::print_info("No frozen providers to thaw");
            }
        }
    }

    Ok(())
}
