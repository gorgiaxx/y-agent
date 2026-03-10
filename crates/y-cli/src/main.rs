use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "y-agent",
    version,
    about = "Yet Another Agent - AI Agent framework"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Start an interactive agent session
    Chat,
    /// Show system status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Chat) => {
            tracing::info!("Starting interactive session...");
            // TODO: implement
        }
        Some(Commands::Status) => {
            tracing::info!("System status: OK");
            // TODO: implement
        }
        None => {
            tracing::info!("y-agent v{}", env!("CARGO_PKG_VERSION"));
            println!("Use --help for available commands.");
        }
    }

    Ok(())
}
