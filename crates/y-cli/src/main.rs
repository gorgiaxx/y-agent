//! y-agent CLI — entry point.
//!
//! This is the sole Tokio runtime entry point. It initialises tracing,
//! loads configuration, wires all services, and dispatches to the
//! requested subcommand.

mod commands;
mod config;
mod orchestrator;
mod output;
#[cfg(feature = "tui")]
mod tui;
mod wire;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use commands::Commands;
use config::ConfigLoader;
use output::OutputMode;

#[derive(Parser)]
#[command(
    name = "y-agent",
    version,
    about = "Yet Another Agent — AI Agent framework"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Log level override.
    #[arg(long, global = true)]
    log_level: Option<String>,

    /// Output format override (json, table, plain).
    #[arg(long, global = true, default_value = "plain")]
    output: String,

    /// Path to config file.
    #[arg(long, global = true)]
    config: Option<String>,

    /// Path to project configuration directory (contains providers.toml, runtime.toml, etc.).
    #[arg(long, global = true)]
    config_dir: Option<String>,

    /// Path to user configuration directory (defaults to ~/.config/y-agent/).
    #[arg(long, global = true)]
    user_config_dir: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle init command early — it runs before config exists.
    if let Some(Commands::Init(ref args)) = cli.command {
        return commands::init::run(args).await;
    }

    // Build CLI overrides.
    let mut cli_overrides = std::collections::HashMap::new();
    if let Some(ref level) = cli.log_level {
        cli_overrides.insert("log_level".to_string(), level.clone());
    }
    cli_overrides.insert("output_format".to_string(), cli.output.clone());

    // Load configuration.
    let mut loader = ConfigLoader::new().with_cli_overrides(cli_overrides);
    if let Some(ref config_path) = cli.config {
        loader = loader.with_project_config(Some(config_path.into()));
    }
    if let Some(ref config_dir) = cli.config_dir {
        loader = loader.with_config_dir(Some(config_dir.into()));
    }
    if let Some(ref user_config_dir) = cli.user_config_dir {
        loader = loader.with_user_config_dir(Some(user_config_dir.into()));
    }
    let config = loader.load()?;
    config::validate_config(&config)?;

    // Determine if we are entering TUI mode.
    #[cfg(feature = "tui")]
    let is_tui = matches!(cli.command, Some(Commands::Tui));
    #[cfg(not(feature = "tui"))]
    let is_tui = false;

    // Build the env filter.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level));

    // Prepare file logging layer (always active).
    let file_layer = if let Some(log_dir) = config::dirs_log() {
        // Ensure log directory exists.
        let _ = std::fs::create_dir_all(&log_dir);
        // Clean up old logs.
        let _ = config::cleanup_old_logs(&log_dir, config.log_retention_days);

        let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("y-agent")
            .filename_suffix("log")
            .build(&log_dir)
            .expect("failed to initialize rolling file appender");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        // Leak the guard to keep the writer alive for the process lifetime.
        std::mem::forget(_guard);
        Some(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true)
                .compact()
                .with_level(true),
        )
    } else {
        None
    };

    // Create toast bridge channel for TUI mode.
    #[cfg(feature = "tui")]
    let (toast_tx, toast_rx) = tokio::sync::mpsc::unbounded_channel();

    // Build and init layered subscriber.
    let registry = tracing_subscriber::registry().with(env_filter).with(file_layer);

    if is_tui {
        // TUI mode: file layer + toast bridge (no stderr).
        #[cfg(feature = "tui")]
        {
            let bridge = tui::tracing_bridge::ToastBridgeLayer::new(toast_tx);
            registry.with(bridge).init();
        }
        #[cfg(not(feature = "tui"))]
        {
            // Should not reach here, but handle gracefully.
            registry.with(tracing_subscriber::fmt::layer()).init();
        }
    } else {
        // Non-TUI: file layer + stderr.
        registry.with(tracing_subscriber::fmt::layer()).init();
    }

    let mode = OutputMode::from_str_or_default(&config.output_format);

    // Dispatch command.
    match cli.command {
        Some(Commands::Chat { session, agent }) => {
            let services = wire::wire(&config).await?;
            commands::chat::run(&services, session.as_deref(), &agent).await?;
        }
        Some(Commands::Status) => {
            let services = wire::wire(&config).await?;
            commands::status::run(&services, mode).await?;
        }
        Some(Commands::Config { ref action }) => {
            commands::config_cmd::run(action, &config, mode).await?;
        }
        Some(Commands::Session { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::session::run(action, &services, mode).await?;
        }
        Some(Commands::Tool { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::tool::run(action, &services, mode).await?;
        }
        Some(Commands::Agent { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::agent::run(action, &services, mode).await?;
        }
        Some(Commands::Workflow { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::workflow::run(action, &services, mode).await?;
        }
        Some(Commands::Diag { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::diag::run(action, &services, mode).await?;
        }
        Some(Commands::Skill { ref action }) => {
            let services = wire::wire(&config).await?;
            commands::skills::run(action, &services, mode).await?;
        }
        #[cfg(feature = "tui")]
        Some(Commands::Tui) => {
            let services = wire::wire(&config).await?;
            commands::tui_cmd::run(services, Some(toast_rx)).await?;
        }
        Some(Commands::Init(_)) => {
            // Already handled above before config loading.
            unreachable!("init is dispatched before config loading");
        }
        Some(Commands::Serve(ref args)) => {
            let services = wire::wire(&config).await?;
            commands::serve::run(std::sync::Arc::new(services), args).await?;
        }
        None => {
            println!("y-agent v{}", env!("CARGO_PKG_VERSION"));
            println!("Use --help for available commands.");
        }
    }

    Ok(())
}
