//! `y-agent serve` — start the Web API server.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use y_service::ServiceContainer;
use y_web::{AppState, WebConfig, create_router};

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

    let state = AppState::new(
        services,
        env!("CARGO_PKG_VERSION"),
    );

    let app = create_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    info!("Starting y-agent API server on http://{addr}");
    println!("🚀 y-agent API server listening on http://{addr}");
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
