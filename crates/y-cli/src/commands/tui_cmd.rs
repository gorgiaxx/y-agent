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

/// Information captured when the TUI exits, for printing a summary.
pub struct ExitInfo {
    /// Session ID that was active at exit time.
    pub session_id: Option<String>,
    /// Cumulative input tokens consumed.
    pub input_tokens: u64,
    /// Cumulative output tokens consumed.
    pub output_tokens: u64,
}

/// Run the TUI interface.
///
/// `toast_rx` receives toast messages from the tracing bridge layer.
/// `resume_session` optionally specifies a session ID (or prefix) to resume on startup.
pub async fn run(
    services: AppServices,
    toast_rx: Option<mpsc::UnboundedReceiver<Toast>>,
    resume_session: Option<String>,
) -> Result<ExitInfo> {
    let services = Arc::new(services);
    services.start_background_services().await;
    let mut app = TuiApp::new(services, toast_rx)?;

    // If a session was specified, switch to it before entering the main loop.
    if let Some(ref target) = resume_session {
        app.resume_session(target).await;
    }

    app.run().await?;

    let exit_info = ExitInfo {
        session_id: app.exit_session_id(),
        input_tokens: app.exit_input_tokens(),
        output_tokens: app.exit_output_tokens(),
    };

    Ok(exit_info)
}
