//! Session management commands.

use anyhow::Result;
use clap::Subcommand;

use y_core::session::{SessionFilter, SessionState};
use y_core::types::SessionId;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Session subcommands.
#[derive(Debug, Subcommand)]
pub enum SessionAction {
    /// List all sessions.
    List,

    /// Resume an existing session.
    Resume {
        /// Session ID to resume.
        id: String,
    },

    /// Branch from a session.
    Branch {
        /// Session ID to branch from.
        id: String,

        /// Label for the branch.
        #[arg(long)]
        label: Option<String>,
    },

    /// Archive a session.
    Archive {
        /// Session ID to archive.
        id: String,
    },

    /// View messages in a session.
    Messages {
        /// Session ID to view messages for.
        id: String,

        /// Show last N messages only.
        #[arg(long)]
        last: Option<usize>,

        /// Output full raw JSON for each message (no truncation).
        #[arg(long)]
        raw: bool,

        /// Output format (table, json).
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// List diagnostic traces for a session.
    Traces {
        /// Session ID to query traces for.
        id: String,

        /// Maximum number of traces to show.
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

/// Run a session subcommand.
pub async fn run(action: &SessionAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        SessionAction::List => {
            let sessions = services
                .session_manager
                .list_sessions(&SessionFilter::default())
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            let headers = &["ID", "Status", "Type", "Title", "Messages"];
            let rows: Vec<TableRow> = sessions
                .iter()
                .map(|s| TableRow {
                    cells: vec![
                        s.id.0.clone(),
                        format!("{:?}", s.state),
                        format!("{:?}", s.session_type),
                        s.title.clone().unwrap_or_default(),
                        s.message_count.to_string(),
                    ],
                })
                .collect();

            if mode == OutputMode::Json {
                let json = serde_json::to_string_pretty(&sessions)?;
                println!("{json}");
            } else {
                let table = output::format_table(headers, &rows);
                print!("{table}");
            }
        }
        SessionAction::Resume { id } => {
            let session_id = SessionId(id.clone());
            match services.session_manager.get_session(&session_id).await {
                Ok(session) => {
                    output::print_info(&format!(
                        "Resuming session: {} (state: {:?}, messages: {})",
                        session.id.0, session.state, session.message_count
                    ));
                    // Full chat resume requires orchestrator integration (future work).
                    output::print_warning(
                        "Interactive resume not yet implemented — use `y-agent chat --session` instead",
                    );
                }
                Err(e) => {
                    output::print_error(&format!("Session not found: {e}"));
                }
            }
        }
        SessionAction::Branch { id, label } => {
            let session_id = SessionId(id.clone());
            match services
                .session_manager
                .branch(&session_id, label.clone())
                .await
            {
                Ok(branch) => {
                    output::print_success(&format!(
                        "Created branch: {} (from parent: {})",
                        branch.id.0, id
                    ));
                }
                Err(e) => {
                    output::print_error(&format!("Failed to branch: {e}"));
                }
            }
        }
        SessionAction::Archive { id } => {
            let session_id = SessionId(id.clone());
            match services
                .session_manager
                .transition_state(&session_id, SessionState::Archived)
                .await
            {
                Ok(()) => {
                    output::print_success(&format!("Session {id} archived"));
                }
                Err(e) => {
                    output::print_error(&format!("Failed to archive: {e}"));
                }
            }
        }
        SessionAction::Messages {
            id,
            last,
            raw,
            format: fmt,
        } => {
            let session_id = SessionId(id.clone());
            let messages = services
                .session_manager
                .read_transcript(&session_id)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            let selected: Vec<_> = if let Some(n) = last {
                messages
                    .into_iter()
                    .rev()
                    .take(*n)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            } else {
                messages
            };

            if selected.is_empty() {
                output::print_info("No messages in this session.");
            } else if *raw || fmt == "json" {
                // Raw mode: output full JSON with no truncation.
                let json = serde_json::to_string_pretty(&selected)?;
                println!("{json}");
            } else {
                let headers = &["#", "Role", "Content", "Timestamp"];
                let rows: Vec<TableRow> = selected
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let content = if m.content.len() > 80 {
                            format!("{}…", &m.content[..80])
                        } else {
                            m.content.clone()
                        };
                        TableRow {
                            cells: vec![
                                i.to_string(),
                                format!("{:?}", m.role),
                                content,
                                m.timestamp.format("%H:%M:%S").to_string(),
                            ],
                        }
                    })
                    .collect();

                let table = output::format_table(headers, &rows);
                print!("{table}");
                output::print_info(&format!("{} message(s)", selected.len()));
            }
        }
        SessionAction::Traces { id, limit } => {
            run_traces(id, *limit, services, mode).await?;
        }
    }

    Ok(())
}

/// `session traces <id>` — list diagnostic traces for a session.
async fn run_traces(
    session_id: &str,
    limit: usize,
    services: &AppServices,
    mode: OutputMode,
) -> Result<()> {
    let store = services.diagnostics.store();
    let traces = store
        .list_traces_by_session(session_id, limit)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if traces.is_empty() {
        output::print_info("No traces found for this session.");
        return Ok(());
    }

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&traces)?;
        println!("{json}");
    } else {
        let headers = &[
            "ID",
            "Name",
            "Status",
            "Started",
            "Cost ($)",
            "Tokens In",
            "Tokens Out",
            "Duration",
        ];
        let rows: Vec<TableRow> = traces
            .iter()
            .map(|t| {
                let duration = t
                    .total_duration_ms.map_or_else(|| "…".to_string(), |d| format!("{:.1}s", d as f64 / 1000.0));
                TableRow {
                    cells: vec![
                        t.id.to_string()[..8].to_string(),
                        t.name.clone(),
                        format!("{:?}", t.status),
                        t.started_at.format("%H:%M:%S").to_string(),
                        format!("{:.4}", t.total_cost_usd),
                        t.total_input_tokens.to_string(),
                        t.total_output_tokens.to_string(),
                        duration,
                    ],
                }
            })
            .collect();

        let table = output::format_table(headers, &rows);
        print!("{table}");
        output::print_info(&format!(
            "{} trace(s) for session {}",
            traces.len(),
            &session_id[..8.min(session_id.len())]
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    #[command(name = "y-agent")]
    struct TestCli {
        #[command(subcommand)]
        command: SessionAction,
    }

    #[test]
    fn test_parse_session_list() {
        let cli = TestCli::parse_from(["y-agent", "list"]);
        assert!(matches!(cli.command, SessionAction::List));
    }

    #[test]
    fn test_parse_session_messages() {
        let cli = TestCli::parse_from(["y-agent", "messages", "abc-123"]);
        match cli.command {
            SessionAction::Messages { id, last, raw, .. } => {
                assert_eq!(id, "abc-123");
                assert!(last.is_none());
                assert!(!raw);
            }
            _ => panic!("expected Messages"),
        }
    }

    #[test]
    fn test_parse_session_messages_raw() {
        let cli = TestCli::parse_from(["y-agent", "messages", "abc-123", "--raw"]);
        match cli.command {
            SessionAction::Messages { id, raw, .. } => {
                assert_eq!(id, "abc-123");
                assert!(raw);
            }
            _ => panic!("expected Messages"),
        }
    }

    #[test]
    fn test_parse_session_traces() {
        let cli = TestCli::parse_from(["y-agent", "traces", "abc-123"]);
        match cli.command {
            SessionAction::Traces { id, limit } => {
                assert_eq!(id, "abc-123");
                assert_eq!(limit, 20);
            }
            _ => panic!("expected Traces"),
        }
    }

    #[test]
    fn test_parse_session_traces_with_limit() {
        let cli = TestCli::parse_from(["y-agent", "traces", "abc-123", "--limit", "5"]);
        match cli.command {
            SessionAction::Traces { id, limit } => {
                assert_eq!(id, "abc-123");
                assert_eq!(limit, 5);
            }
            _ => panic!("expected Traces"),
        }
    }
}

