//! Runtime-managed background task commands.

use std::fmt::Write as _;

use anyhow::Result;
use clap::Subcommand;
use y_service::{
    BackgroundTaskInfo, BackgroundTaskPollRequest, BackgroundTaskService, BackgroundTaskSnapshot,
    BackgroundTaskWriteRequest,
};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Background task subcommands.
#[derive(Debug, Subcommand)]
pub enum BackgroundTaskAction {
    /// List background tasks owned by a session.
    List {
        /// Session that owns the tasks.
        #[arg(long)]
        session: String,
    },
    /// Poll incremental output from a task.
    Poll {
        /// Session that owns the task.
        #[arg(long)]
        session: String,
        /// Runtime process ID.
        process_id: String,
        /// Maximum time to wait for immediate output.
        #[arg(long)]
        yield_time_ms: Option<u64>,
        /// Maximum combined output bytes returned by the runtime.
        #[arg(long)]
        max_output_bytes: Option<usize>,
    },
    /// Write to a task's stdin and return its next output snapshot.
    Write {
        /// Session that owns the task.
        #[arg(long)]
        session: String,
        /// Runtime process ID.
        process_id: String,
        /// Text written to stdin.
        input: String,
        /// Maximum time to wait for immediate output.
        #[arg(long)]
        yield_time_ms: Option<u64>,
        /// Maximum combined output bytes returned by the runtime.
        #[arg(long)]
        max_output_bytes: Option<usize>,
    },
    /// Terminate a task and return its final output snapshot.
    Kill {
        /// Session that owns the task.
        #[arg(long)]
        session: String,
        /// Runtime process ID.
        process_id: String,
        /// Maximum time to wait for final output.
        #[arg(long)]
        yield_time_ms: Option<u64>,
        /// Maximum combined output bytes returned by the runtime.
        #[arg(long)]
        max_output_bytes: Option<usize>,
    },
}

/// Run a background task subcommand.
pub async fn run(
    action: &BackgroundTaskAction,
    services: &AppServices,
    mode: OutputMode,
) -> Result<()> {
    match action {
        BackgroundTaskAction::List { session } => {
            let tasks = BackgroundTaskService::list(services, session.clone()).await?;
            print_task_list(&tasks, mode);
        }
        BackgroundTaskAction::Poll {
            session,
            process_id,
            yield_time_ms,
            max_output_bytes,
        } => {
            let snapshot = BackgroundTaskService::poll(
                services,
                poll_request(session, process_id, *yield_time_ms, *max_output_bytes),
            )
            .await?;
            print!("{}", format_snapshot(&snapshot, mode));
        }
        BackgroundTaskAction::Write {
            session,
            process_id,
            input,
            yield_time_ms,
            max_output_bytes,
        } => {
            let snapshot = BackgroundTaskService::write(
                services,
                BackgroundTaskWriteRequest {
                    session_id: session.clone(),
                    process_id: process_id.clone(),
                    input: input.clone(),
                    yield_time_ms: *yield_time_ms,
                    max_output_bytes: *max_output_bytes,
                },
            )
            .await?;
            print!("{}", format_snapshot(&snapshot, mode));
        }
        BackgroundTaskAction::Kill {
            session,
            process_id,
            yield_time_ms,
            max_output_bytes,
        } => {
            let snapshot = BackgroundTaskService::kill(
                services,
                poll_request(session, process_id, *yield_time_ms, *max_output_bytes),
            )
            .await?;
            print!("{}", format_snapshot(&snapshot, mode));
        }
    }
    Ok(())
}

fn poll_request(
    session: &str,
    process_id: &str,
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
) -> BackgroundTaskPollRequest {
    BackgroundTaskPollRequest {
        session_id: session.to_string(),
        process_id: process_id.to_string(),
        yield_time_ms,
        max_output_bytes,
    }
}

fn print_task_list(tasks: &[BackgroundTaskInfo], mode: OutputMode) {
    if mode == OutputMode::Json {
        println!("{}", output::format_value(&tasks, mode));
        return;
    }
    if tasks.is_empty() {
        output::print_info("No background tasks found for this session");
        return;
    }
    let rows = tasks
        .iter()
        .map(|task| TableRow {
            cells: vec![
                task.process_id.clone(),
                task.status.clone(),
                task.backend.clone(),
                task.duration_ms.to_string(),
                task.command.clone(),
            ],
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        output::format_table(
            &["PROCESS", "STATUS", "BACKEND", "DURATION_MS", "COMMAND"],
            &rows
        )
    );
}

fn format_snapshot(snapshot: &BackgroundTaskSnapshot, mode: OutputMode) -> String {
    if mode == OutputMode::Json {
        return format!("{}\n", output::format_value(snapshot, mode));
    }
    let mut rendered = String::new();
    let _ = writeln!(rendered, "Process:  {}", snapshot.process_id);
    let _ = writeln!(rendered, "Status:   {}", snapshot.status);
    let _ = writeln!(rendered, "Backend:  {}", snapshot.backend);
    let _ = writeln!(rendered, "Duration: {}ms", snapshot.duration_ms);
    if let Some(exit_code) = snapshot.exit_code {
        let _ = writeln!(rendered, "Exit:     {exit_code}");
    }
    if let Some(error) = &snapshot.error {
        let _ = writeln!(rendered, "Error:    {error}");
    }
    if !snapshot.stdout.is_empty() {
        let _ = writeln!(rendered, "\nstdout:\n{}", snapshot.stdout);
    }
    if !snapshot.stderr.is_empty() {
        let _ = writeln!(rendered, "\nstderr:\n{}", snapshot.stderr);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_snapshot_plain_includes_output_and_status() {
        let snapshot = BackgroundTaskSnapshot {
            process_id: "proc-1".to_string(),
            backend: "native".to_string(),
            status: "completed".to_string(),
            exit_code: Some(0),
            error: None,
            stdout: "done".to_string(),
            stderr: String::new(),
            duration_ms: 25,
        };

        let rendered = format_snapshot(&snapshot, OutputMode::Plain);

        assert!(rendered.contains("Process:  proc-1"));
        assert!(rendered.contains("Status:   completed"));
        assert!(rendered.contains("stdout:\ndone"));
    }

    #[test]
    fn test_poll_request_preserves_runtime_bounds() {
        let request = poll_request("session-1", "proc-1", Some(500), Some(1024));

        assert_eq!(request.session_id, "session-1");
        assert_eq!(request.process_id, "proc-1");
        assert_eq!(request.yield_time_ms, Some(500));
        assert_eq!(request.max_output_bytes, Some(1024));
    }
}
