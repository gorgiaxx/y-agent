//! `rewind` CLI command -- file history rollback.
//!
//! Subcommands:
//! - `list` -- list available rewind points for a session
//! - `execute` -- execute a full rewind (transcript + files + checkpoints)
//! - `restore` -- restore files only (no transcript/checkpoint changes)

use anyhow::Result;
use clap::Subcommand;

use y_core::types::SessionId;
use y_service::{RewindError, RewindService};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Rewind subcommands.
#[derive(Debug, Subcommand)]
pub enum RewindAction {
    /// List available rewind points for a session.
    List {
        /// Session ID to list rewind points for.
        session_id: String,
    },

    /// Execute a rewind to a specific message boundary.
    ///
    /// Performs three-phase rollback: transcript truncation, file
    /// restoration, and checkpoint invalidation.
    Execute {
        /// Session ID to rewind.
        session_id: String,

        /// Target message ID to rewind to.
        target_message_id: String,
    },

    /// Restore files to a message boundary without touching transcripts.
    Restore {
        /// Session ID to restore files for.
        session_id: String,

        /// Target message ID to restore files to.
        target_message_id: String,
    },
}

/// Run a rewind subcommand.
pub async fn run(action: &RewindAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        RewindAction::List { session_id } => run_list(session_id, services, mode).await,
        RewindAction::Execute {
            session_id,
            target_message_id,
        } => run_execute(session_id, target_message_id, services, mode).await,
        RewindAction::Restore {
            session_id,
            target_message_id,
        } => run_restore(session_id, target_message_id, services, mode).await,
    }
}

/// `rewind list <session_id>` -- list rewind points.
async fn run_list(session_id: &str, services: &AppServices, mode: OutputMode) -> Result<()> {
    let sid = SessionId(session_id.to_string());

    let points = RewindService::list_rewind_points(services, &sid)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&points, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            if points.is_empty() {
                output::print_info("No rewind points found for this session");
                return Ok(());
            }

            output::print_info(&format!(
                "{} rewind point(s) for session '{}':",
                points.len(),
                session_id
            ));

            let headers = &["Message ID", "Preview", "Files Changed", "Timestamp"];
            let rows: Vec<TableRow> = points
                .iter()
                .map(|p| {
                    let files_changed = p.diff_stats.files_changed + p.diff_stats.files_created;
                    TableRow {
                        cells: vec![
                            p.message_id.clone(),
                            p.message_preview.clone(),
                            files_changed.to_string(),
                            format_timestamp(p.timestamp),
                        ],
                    }
                })
                .collect();

            let table = output::format_table(headers, &rows);
            print!("{table}");
        }
    }

    Ok(())
}

/// `rewind execute <session_id> <target_message_id>` -- execute rewind.
async fn run_execute(
    session_id: &str,
    target_message_id: &str,
    services: &AppServices,
    mode: OutputMode,
) -> Result<()> {
    let sid = SessionId(session_id.to_string());

    let result = RewindService::execute_rewind(services, &sid, target_message_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&result, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            output::print_success(&format!(
                "Rewound to message '{}'",
                result.target_message_id
            ));
            output::print_status("Messages removed", &result.messages_removed.to_string());
            output::print_status(
                "Checkpoints invalidated",
                &result.checkpoints_invalidated.to_string(),
            );
            output::print_status(
                "Files restored",
                &result.file_report.restored.len().to_string(),
            );
            output::print_status(
                "Files deleted",
                &result.file_report.deleted.len().to_string(),
            );
            if !result.file_report.conflicts.is_empty() {
                output::print_warning(&format!(
                    "{} file conflict(s) encountered:",
                    result.file_report.conflicts.len()
                ));
                for c in &result.file_report.conflicts {
                    println!("  {} -- {}", c.path, c.reason);
                }
            }
        }
    }

    Ok(())
}

/// `rewind restore <session_id> <target_message_id>` -- restore files only.
async fn run_restore(
    session_id: &str,
    target_message_id: &str,
    services: &AppServices,
    mode: OutputMode,
) -> Result<()> {
    let sid = SessionId(session_id.to_string());

    let report = match RewindService::restore_files_only(services, &sid, target_message_id).await {
        Ok(report) => report,
        // File restoration is best-effort: if no file history exists for
        // this session, silently succeed (matching GUI behavior).
        Err(RewindError::NoHistory(_)) => {
            output::print_info("No file history for this session; nothing to restore");
            return Ok(());
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&report, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            output::print_success(&format!("Restored files to message '{target_message_id}'"));
            output::print_status("Files restored", &report.restored.len().to_string());
            output::print_status("Files deleted", &report.deleted.len().to_string());
            if !report.conflicts.is_empty() {
                output::print_warning(&format!(
                    "{} file conflict(s) encountered:",
                    report.conflicts.len()
                ));
                for c in &report.conflicts {
                    println!("  {} -- {}", c.path, c.reason);
                }
            }
        }
    }

    Ok(())
}

/// Format a Unix timestamp (seconds) as a human-readable UTC string.
fn format_timestamp(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Utc.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        _ => ts.to_string(),
    }
}
