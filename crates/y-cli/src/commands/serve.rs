//! `y-agent serve` -- start the Web API server.

use std::sync::Arc;

use anyhow::Result;
use tracing::{error, info, warn};

use y_service::ServiceContainer;
use y_web::{create_router, AppState, WebConfig};

/// CLI arguments for the serve subcommand.
#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Host to bind to.
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,

    /// Port to bind to.
    #[arg(long, default_value_t = 3000)]
    pub port: u16,
}

/// Run the web API server.
pub async fn run(services: Arc<ServiceContainer>, args: &ServeArgs) -> Result<()> {
    let config = WebConfig {
        host: args.host.clone(),
        port: args.port,
    };

    let mut state = AppState::new(services, env!("CARGO_PKG_VERSION"));

    // Load bot adapters from bots.toml in the user config directory.
    load_bots(&mut state);

    // Start the Discord Gateway (WebSocket) if the bot is loaded.
    // This makes the bot appear online and receive MESSAGE_CREATE events.
    if let Some(ref discord_bot) = state.discord_bot {
        start_discord_gateway(Arc::clone(discord_bot), Arc::clone(&state.container));
    }

    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    info!("Starting y-agent API server on http://{addr}");
    println!("y-agent API server listening on http://{addr}");
    println!("   Health:  GET  http://{addr}/health");
    println!("   Status:  GET  http://{addr}/api/v1/status");
    println!("   Chat:    POST http://{addr}/api/v1/chat");
    println!("   Docs:    See docs/api/openapi.yaml");
    println!();
    println!("Press Ctrl+C to stop.");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Bot loading
// ---------------------------------------------------------------------------

/// Bot configuration file structure (mirrors `config/bots.toml`).
#[derive(Debug, Default, serde::Deserialize)]
struct BotFileConfig {
    #[serde(default)]
    feishu: Option<y_bot::feishu::FeishuBotConfig>,
    #[serde(default)]
    discord: Option<y_bot::discord::DiscordBotConfig>,
}

/// Load bot adapters from `~/.config/y-agent/bots.toml` and inject into `AppState`.
fn load_bots(state: &mut AppState) {
    let Some(config_dir) = crate::config::dirs_user_config() else {
        return;
    };

    let bots_path = config_dir.join("bots.toml");
    if !bots_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&bots_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, path = %bots_path.display(), "Failed to read bots.toml");
            return;
        }
    };

    let bots_config: BotFileConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, path = %bots_path.display(), "Failed to parse bots.toml");
            return;
        }
    };

    // Wire Feishu bot if configured.
    if let Some(feishu_config) = bots_config.feishu {
        if !feishu_config.app_id.is_empty() {
            info!(app_id = %feishu_config.app_id, "Feishu bot loaded");
            let bot = y_bot::feishu::FeishuBot::new(feishu_config);
            state.feishu_bot = Some(Arc::new(bot));
        }
    }

    // Wire Discord bot if configured.
    if let Some(discord_config) = bots_config.discord {
        if !discord_config.token.is_empty() {
            info!(
                application_id = %discord_config.application_id,
                "Discord bot loaded"
            );
            let bot = y_bot::discord::DiscordBot::new(discord_config);
            state.discord_bot = Some(Arc::new(bot));
        }
    }
}

// ---------------------------------------------------------------------------
// Discord Gateway
// ---------------------------------------------------------------------------

/// Start the Discord Gateway WebSocket connection and message processing loop.
///
/// Spawns two background tasks:
/// 1. Gateway connection (heartbeat, reconnect, event dispatch)
/// 2. Message processor (reads `InboundMessage` channel -> `BotService`)
fn start_discord_gateway(
    discord_bot: Arc<y_bot::discord::DiscordBot>,
    container: Arc<ServiceContainer>,
) {
    let gateway_config = Arc::new(discord_bot.config().clone());
    let mut handle = y_bot::discord_gateway::start_gateway(gateway_config);

    info!("Discord Gateway: started background connection");

    // Spawn the message processing loop.
    tokio::spawn(async move {
        while let Some(message) = handle.rx.recv().await {
            let container = Arc::clone(&container);
            let bot = Arc::clone(&discord_bot);

            // Process each message concurrently.
            tokio::spawn(async move {
                if let Err(e) =
                    y_service::BotService::handle_message(&container, bot.as_ref(), message).await
                {
                    error!(error = %e, "Discord Gateway: message handling failed");
                }
            });
        }

        warn!("Discord Gateway: message channel closed");
    });
}
