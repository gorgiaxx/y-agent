//! `y-agent tui` subcommand entry point.
//!
//! Constructs the `TuiApp` with wired application services and delegates
//! to its main event loop.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::tui::state::Toast;
use crate::tui::TuiApp;
use crate::wire::AppServices;

/// Run the TUI interface.
///
/// `toast_rx` receives toast messages from the tracing bridge layer.
pub async fn run(
    services: AppServices,
    toast_rx: Option<mpsc::UnboundedReceiver<Toast>>,
) -> Result<()> {
    let services = Arc::new(services);
    services.init_agent_runner();
    let mut app = TuiApp::new(services, toast_rx)?;
    app.run().await
}
