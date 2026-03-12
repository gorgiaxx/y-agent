//! System status command.

use anyhow::Result;
use serde::Serialize;

use y_core::provider::ProviderPool;
use y_core::runtime::RuntimeAdapter;

use crate::output::{self, OutputMode};
use crate::wire::AppServices;

/// Status report for display.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub version: String,
    pub providers_registered: usize,
    pub tools_registered: usize,
    pub runtime_backend: String,
    pub storage_status: String,
}

/// Run the status command.
pub async fn run(services: &AppServices, mode: OutputMode) -> Result<()> {
    let provider_count = services.provider_pool.provider_statuses().await.len();
    let tool_count = services.tool_registry.len().await;
    let runtime_backend = format!("{:?}", services.runtime_manager.backend());

    let report = StatusReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        providers_registered: provider_count,
        tools_registered: tool_count,
        runtime_backend,
        storage_status: "connected".to_string(),
    };

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&report, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            println!("y-agent status");
            println!("==============");
            output::print_status("Version", &report.version);
            output::print_status("Providers", &report.providers_registered.to_string());
            output::print_status("Tools", &report.tools_registered.to_string());
            output::print_status("Runtime", &report.runtime_backend);
            output::print_status("Storage", &report.storage_status);
        }
    }

    Ok(())
}
