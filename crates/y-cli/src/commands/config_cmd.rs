//! Configuration management commands.

use anyhow::Result;
use clap::Subcommand;

use crate::config::{self, YAgentConfig};
use crate::output::{self, OutputMode};

/// Configuration subcommands.
#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Show the resolved configuration.
    Show,

    /// Validate the configuration file.
    Validate {
        /// Path to config file to validate.
        #[arg(default_value = "y-agent.toml")]
        path: String,
    },
}

/// Run a config subcommand.
pub async fn run(action: &ConfigAction, config: &YAgentConfig, mode: OutputMode) -> Result<()> {
    match action {
        ConfigAction::Show => match mode {
            OutputMode::Json => {
                println!("{}", output::format_value(config, mode));
            }
            OutputMode::Table | OutputMode::Plain => {
                let toml_str = toml::to_string_pretty(config)?;
                println!("{toml_str}");
            }
        },
        ConfigAction::Validate { path } => {
            let content = std::fs::read_to_string(path)?;
            let parsed: YAgentConfig = toml::from_str(&content)?;
            config::validate_config(&parsed)?;
            output::print_success("Configuration is valid");
        }
    }

    Ok(())
}
