//! y-agent CLI — entry point.
//!
//! This is the sole Tokio runtime entry point. It initialises tracing,
//! loads configuration, wires all services, and dispatches to the
//! requested subcommand.

mod bare_prompt;
mod commands;
mod config;
mod orchestrator;
mod output;
mod slash_files;
#[cfg(feature = "tui")]
mod tui;
mod wire;

use std::path::PathBuf;

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

    /// Named profile for isolated agent state (e.g. `work`). Resolves to
    /// `~/.config/y-agent-<name>/`. Mutually exclusive with `--user-config-dir`.
    /// Can also be set via the `Y_AGENT_PROFILE` environment variable.
    #[arg(long, global = true)]
    profile: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Bare-prompt resolution: `y-agent "do X"` forwards to `chat -- "do X"`.
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let resolved_args = bare_prompt::resolve(&raw_args);
    let cli = Cli::parse_from(resolved_args);

    // Handle init command early -- it runs before config exists.
    if let Some(Commands::Init(args)) = &cli.command {
        return commands::init::run(args).await;
    }

    // Handle completion early -- no config needed.
    if let Some(Commands::Completion(args)) = &cli.command {
        commands::completion::run(args);
        return Ok(());
    }

    // Build CLI overrides.
    let mut cli_overrides = std::collections::HashMap::new();
    if let Some(level) = &cli.log_level {
        cli_overrides.insert("log_level".to_string(), level.clone());
    }
    cli_overrides.insert("output_format".to_string(), cli.output.clone());

    // Resolve profile: `--profile <name>` or `Y_AGENT_PROFILE` env var.
    // Profiles live at `~/.config/y-agent-<name>/`. Mutually exclusive with
    // `--user-config-dir`.
    let profile_dir = resolve_profile(&cli)?;
    let user_config_dir = match (&profile_dir, &cli.user_config_dir) {
        (Some(dir), None) => Some(dir.clone()),
        (None, Some(d)) => Some(PathBuf::from(d)),
        (None, None) => None,
        (Some(_), Some(_)) => {
            anyhow::bail!("--profile and --user-config-dir are mutually exclusive");
        }
    };

    // Load configuration.
    let mut loader = ConfigLoader::new().with_cli_overrides(cli_overrides);
    if let Some(config_path) = &cli.config {
        loader = loader.with_project_config(Some(config_path.into()));
    }
    if let Some(config_dir) = &cli.config_dir {
        loader = loader.with_config_dir(Some(config_dir.into()));
    }
    if let Some(dir) = &user_config_dir {
        loader = loader.with_user_config_dir(Some(dir.clone()));
    }
    let loaded_config = loader.load_with_provenance()?;
    let config = loaded_config.config;
    let project_config_sources = loaded_config.project_sources;
    config::validate_config(&config)?;

    // Determine if we are entering TUI mode.
    #[cfg(feature = "tui")]
    let is_tui = matches!(
        cli.command,
        Some(Commands::Tui { .. } | Commands::Resume { .. } | Commands::Fork { .. })
    );
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
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        // Leak the guard to keep the writer alive for the process lifetime.
        std::mem::forget(guard);
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
    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer);

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

    for source in project_config_sources
        .iter()
        .filter(|source| !source.applied)
    {
        tracing::warn!(
            source = %source.source_path.display(),
            workspace = %source.workspace_path.display(),
            trust = ?source.trust,
            reason = source.blocked_reason.as_deref().unwrap_or("workspace is not trusted"),
            hint = "run `y-agent workspace trust --path <workspace>` to activate project config",
            "project configuration source blocked by workspace trust policy"
        );
    }
    for source in project_config_sources
        .iter()
        .filter(|source| source.applied)
    {
        tracing::info!(
            source = %source.source_path.display(),
            workspace = %source.workspace_path.display(),
            "trusted project configuration source activated"
        );
    }

    let mode = OutputMode::from_str_or_default(&config.output_format);

    // Dispatch command.
    match &cli.command {
        Some(Commands::Chat {
            session,
            agent,
            prompt,
        }) => {
            let services = wire::wire(&config).await?;
            let initial_prompt = if prompt.is_empty() {
                None
            } else {
                Some(prompt.join(" "))
            };
            commands::chat::run(
                &services,
                session.as_deref(),
                agent,
                initial_prompt.as_deref(),
            )
            .await?;
        }
        Some(Commands::Status) => {
            let services = wire::wire(&config).await?;
            commands::status::run(&services, mode).await?;
        }
        Some(Commands::Config { action }) => {
            commands::config_cmd::run(action, &config, mode)?;
        }
        Some(Commands::Session { action }) => {
            let services = wire::wire(&config).await?;
            commands::session::run(action, &services, mode).await?;
        }
        Some(Commands::Tool { action }) => {
            let services = wire::wire(&config).await?;
            commands::tool::run(action, &services, mode).await?;
        }
        Some(Commands::Agent { action }) => {
            let services = wire::wire(&config).await?;
            commands::agent::run(action, &services, mode).await?;
        }
        Some(Commands::Workflow { action }) => {
            let services = wire::wire(&config).await?;
            commands::workflow::run(action, &services, mode).await?;
        }
        Some(Commands::Diag { action }) => {
            let services = wire::wire(&config).await?;
            commands::diag::run(action, &services, mode).await?;
        }
        Some(Commands::Skill { action }) => {
            let services = wire::wire(&config).await?;
            commands::skills::run(action, &services, mode).await?;
        }
        Some(Commands::Kb { action }) => {
            let services = wire::wire(&config).await?;
            commands::kb::run(action, &services, mode).await?;
        }
        Some(Commands::Mcp { action }) => match action {
            commands::mcp::McpAction::Status => {
                let services = wire::wire(&config).await?;
                let services = std::sync::Arc::new(services);
                services.start_background_services().await;
                commands::mcp::run_status(&services, mode).await?;
            }
            other => {
                let path = resolve_mcp_tools_toml_path(
                    user_config_dir
                        .as_deref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .as_deref(),
                )?;
                commands::mcp::run_offline(other, &path, mode)?;
            }
        },
        Some(Commands::Completion(_)) => {
            // Already handled above before config loading.
            unreachable!("completion is dispatched before config loading");
        }
        Some(Commands::Print {
            mode: print_mode,
            session,
            agent,
            prompt,
        }) => {
            let services = wire::wire(&config).await?;
            let args = commands::print::PrintArgs {
                mode: print_mode.clone(),
                session: session.clone(),
                agent: agent.clone(),
                prompt: prompt.clone(),
            };
            commands::print::run(&services, args).await?;
        }
        Some(Commands::Rpc) => {
            let services = wire::wire(&config).await?;
            commands::rpc::run(&services).await?;
        }
        #[cfg(feature = "tui")]
        Some(Commands::Tui { session }) => {
            let services = wire::wire(&config).await?;
            let exit_info =
                commands::tui_cmd::run(services, Some(toast_rx), session.clone()).await?;
            print_exit_summary(&exit_info);
        }
        Some(Commands::Init(_)) => {
            // Already handled above before config loading.
            unreachable!("init is dispatched before config loading");
        }
        Some(Commands::Serve(args)) => {
            let services = wire::wire(&config).await?;
            let services = std::sync::Arc::new(services);
            services.start_background_services().await;
            commands::serve::run(
                services,
                args,
                user_config_dir
                    .as_deref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .as_deref(),
            )
            .await?;
        }
        #[cfg(feature = "tui")]
        Some(Commands::Resume { session }) => {
            let services = wire::wire(&config).await?;
            // Resume uses the most recent session if none specified.
            let session_id = resolve_resume_session(session.clone(), &services).await;
            let exit_info = commands::tui_cmd::run(services, Some(toast_rx), session_id).await?;
            print_exit_summary(&exit_info);
        }
        #[cfg(feature = "tui")]
        Some(Commands::Fork { session, label }) => {
            let services = wire::wire(&config).await?;
            let forked = fork_session(session.clone(), label.clone(), &services).await?;
            let exit_info = commands::tui_cmd::run(services, Some(toast_rx), Some(forked)).await?;
            print_exit_summary(&exit_info);
        }
        Some(Commands::Workspace { action }) => {
            commands::workspace::run(action, mode, user_config_dir.as_deref())?;
        }
        Some(Commands::Provider { action }) => {
            let services = wire::wire(&config).await?;
            commands::provider::run(action, &services, mode).await?;
        }
        Some(Commands::Observe { action }) => {
            let services = wire::wire(&config).await?;
            commands::observe::run(action, &services, mode).await?;
        }
        Some(Commands::Rewind { action }) => {
            let services = wire::wire(&config).await?;
            commands::rewind::run(action, &services, mode).await?;
        }
        None => {
            println!("y-agent v{}", env!("CARGO_PKG_VERSION"));
            println!("Use --help for available commands.");
        }
    }

    Ok(())
}

/// Resolve a named profile to its config directory.
///
/// `--profile <name>` takes precedence over the `Y_AGENT_PROFILE` env var.
/// Returns `None` if neither is set. Returns an error if the home directory
/// cannot be resolved when a profile is requested.
fn resolve_profile(cli: &Cli) -> Result<Option<PathBuf>> {
    if let Some(name) = &cli.profile {
        return Ok(Some(profile_path(name)?));
    }
    if let Ok(name) = std::env::var("Y_AGENT_PROFILE") {
        if !name.is_empty() {
            return Ok(Some(profile_path(&name)?));
        }
    }
    Ok(None)
}

/// Build the config directory path for a named profile.
///
/// Profiles live at `~/.config/y-agent-<name>/` (siblings of the default
/// `~/.config/y-agent/`).
fn profile_path(name: &str) -> Result<PathBuf> {
    config::home_dir()
        .map(|h| h.join(".config").join(format!("y-agent-{name}")))
        .ok_or_else(|| anyhow::anyhow!("cannot resolve home directory for profile `{name}`"))
}

/// Resolve the `tools.toml` path used by the `mcp` subcommands.
///
/// Honors the global `--user-config-dir` override; falls back to
/// `~/.config/y-agent/tools.toml`.
fn resolve_mcp_tools_toml_path(user_dir_override: Option<&str>) -> Result<std::path::PathBuf> {
    if let Some(dir) = user_dir_override {
        return Ok(std::path::PathBuf::from(dir).join("tools.toml"));
    }
    commands::mcp::default_tools_toml_path()
}

/// Print a summary after the TUI exits, including token usage and a resume hint.
#[cfg(feature = "tui")]
fn print_exit_summary(exit_info: &commands::tui_cmd::ExitInfo) {
    let total = exit_info.input_tokens + exit_info.output_tokens;
    if total > 0 {
        println!(
            "Token usage: input={} output={} total={}",
            exit_info.input_tokens, exit_info.output_tokens, total,
        );
    }
    if let Some(ref sid) = exit_info.session_id {
        let short_id = if sid.len() > 8 { &sid[..8] } else { sid };
        println!("To continue this session, run: y-agent resume {short_id}");
    }
}

/// Resolve a session ID for the resume subcommand.
///
/// If `session` is `Some`, use it as-is. Otherwise, find the most recent session.
#[cfg(feature = "tui")]
async fn resolve_resume_session(
    session: Option<String>,
    services: &wire::AppServices,
) -> Option<String> {
    use y_core::session::SessionFilter;

    if session.is_some() {
        return session;
    }

    // Find the most recent session.
    match services
        .session_manager
        .list_sessions(&SessionFilter::default())
        .await
    {
        Ok(mut nodes) => {
            nodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            nodes.first().map(|n| n.id.to_string())
        }
        Err(_) => None,
    }
}

/// Fork a session and return the new session's ID string.
#[cfg(feature = "tui")]
async fn fork_session(
    session: Option<String>,
    label: Option<String>,
    services: &wire::AppServices,
) -> anyhow::Result<String> {
    use y_core::session::SessionFilter;

    // Resolve the source session.
    let source_id = if let Some(ref target) = session {
        let nodes = services
            .session_manager
            .list_sessions(&SessionFilter::default())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let target_lower = target.to_lowercase();
        let matched = nodes.iter().find(|n| {
            n.id.to_string().starts_with(target.as_str())
                || n.title
                    .as_ref()
                    .is_some_and(|t| t.to_lowercase().contains(&target_lower))
        });
        matched
            .map(|n| n.id.clone())
            .ok_or_else(|| anyhow::anyhow!("no session matching '{target}'"))?
    } else {
        // Use the most recent session.
        let mut nodes = services
            .session_manager
            .list_sessions(&SessionFilter::default())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        nodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        nodes
            .first()
            .map(|n| n.id.clone())
            .ok_or_else(|| anyhow::anyhow!("no sessions to fork"))?
    };

    let fork = services
        .session_manager
        .fork_session(&source_id, usize::MAX, label)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let fork_id = fork.id.to_string();
    let fork_title = fork.title.unwrap_or_else(|| fork_id[..8].to_string());
    println!("Forked session: {fork_title} ({fork_id})");

    Ok(fork_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-PROF-01: profile_path builds `~/.config/y-agent-<name>/`.
    #[test]
    fn test_profile_path_resolution() {
        // We can't assert the absolute path without a fixed home, but we can
        // verify the suffix structure when home resolves.
        if let Some(home) = config::home_dir() {
            let path = profile_path("work").unwrap();
            assert_eq!(path, home.join(".config").join("y-agent-work"));
        }
    }

    // T-CLI-PROF-02: profile_path with empty name still produces a path
    // (validation is the caller's responsibility; the function is pure).
    #[test]
    fn test_profile_path_empty_name() {
        if config::home_dir().is_some() {
            let path = profile_path("").unwrap();
            assert!(path.ends_with("y-agent-"));
        }
    }

    // T-CLI-PROF-03: different profile names produce different paths.
    #[test]
    fn test_profile_paths_distinct() {
        if config::home_dir().is_some() {
            let work = profile_path("work").unwrap();
            let personal = profile_path("personal").unwrap();
            assert_ne!(work, personal);
        }
    }
}
