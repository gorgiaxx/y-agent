//! Schedule management commands.

use anyhow::Result;
use clap::Subcommand;

use y_service::{ExecutionSummary, ScheduleSummary, SchedulerService};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Schedule subcommands.
#[derive(Debug, Subcommand)]
pub enum ScheduleAction {
    /// List all schedules.
    List,

    /// Show detailed info about a schedule by ID.
    Get {
        /// Schedule ID.
        id: String,
    },

    /// Delete a schedule by ID.
    Delete {
        /// Schedule ID.
        id: String,
    },

    /// Pause a schedule (disable without removing).
    Pause {
        /// Schedule ID.
        id: String,
    },

    /// Resume a paused schedule.
    Resume {
        /// Schedule ID.
        id: String,
    },

    /// List execution history for a schedule.
    History {
        /// Schedule ID.
        id: String,
    },

    /// Manually trigger a schedule execution.
    Trigger {
        /// Schedule ID.
        id: String,
    },
}

/// Run a schedule subcommand.
pub async fn run(action: &ScheduleAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        ScheduleAction::List => cmd_list(services, mode).await,
        ScheduleAction::Get { id } => cmd_get(services, id, mode).await,
        ScheduleAction::Delete { id } => cmd_delete(services, id, mode).await,
        ScheduleAction::Pause { id } => cmd_pause(services, id, mode).await,
        ScheduleAction::Resume { id } => cmd_resume(services, id, mode).await,
        ScheduleAction::History { id } => cmd_history(services, id, mode).await,
        ScheduleAction::Trigger { id } => cmd_trigger(services, id, mode).await,
    }
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

async fn cmd_list(services: &AppServices, mode: OutputMode) -> Result<()> {
    let schedules: Vec<ScheduleSummary> =
        SchedulerService::list(&services.scheduler_manager).await;

    match mode {
        OutputMode::Json => {
            let json = serde_json::to_string_pretty(&schedules)?;
            println!("{json}");
        }
        _ => {
            if schedules.is_empty() {
                output::print_info("No schedules found");
            } else {
                let headers = &["ID", "Name", "Workflow", "Status", "Next Run"];
                let rows: Vec<TableRow> = schedules
                    .iter()
                    .map(|s| TableRow {
                        cells: vec![
                            s.id.clone(),
                            s.name.clone(),
                            s.workflow_id.clone(),
                            if s.enabled {
                                "active".to_string()
                            } else {
                                "paused".to_string()
                            },
                            s.trigger_value.clone(),
                        ],
                    })
                    .collect();
                let table = output::format_table(headers, &rows);
                print!("{table}");
            }
        }
    }

    Ok(())
}

async fn cmd_get(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    let schedule: ScheduleSummary =
        match SchedulerService::get(&services.scheduler_manager, id).await {
            Ok(s) => s,
            Err(e) => {
                output::print_error(&format!("Schedule not found: {id} ({e})"));
                return Ok(());
            }
        };

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&schedule)?;
        println!("{json}");
    } else {
        println!("ID:           {}", schedule.id);
        println!("Name:         {}", schedule.name);
        println!(
            "Status:       {}",
            if schedule.enabled { "active" } else { "paused" }
        );
        println!("Workflow:     {}", schedule.workflow_id);
        println!(
            "Trigger:      {} ({})",
            schedule.trigger_type, schedule.trigger_value
        );
        if !schedule.description.is_empty() {
            println!("Description:  {}", schedule.description);
        }
        if !schedule.tags.is_empty() {
            println!("Tags:         {}", schedule.tags.join(", "));
        }
        println!("Created:      {}", schedule.created_at);
        if let Some(last) = &schedule.last_fire {
            println!("Last Fire:    {last}");
        }
    }

    Ok(())
}

async fn cmd_delete(services: &AppServices, id: &str, _mode: OutputMode) -> Result<()> {
    let deleted = SchedulerService::delete(
        &services.scheduler_manager,
        id,
        Some(&services.schedule_store),
    )
    .await?;

    if deleted {
        output::print_success(&format!("Schedule deleted: {id}"));
    } else {
        output::print_error(&format!("Schedule not found: {id}"));
    }
    Ok(())
}

async fn cmd_pause(services: &AppServices, id: &str, _mode: OutputMode) -> Result<()> {
    if let Err(e) = SchedulerService::pause(
        &services.scheduler_manager,
        id,
        Some(&services.schedule_store),
    )
    .await
    {
        output::print_error(&format!("Failed to pause schedule {id}: {e}"));
        return Ok(());
    }
    output::print_success(&format!("Schedule paused: {id}"));
    Ok(())
}

async fn cmd_resume(services: &AppServices, id: &str, _mode: OutputMode) -> Result<()> {
    if let Err(e) = SchedulerService::resume(
        &services.scheduler_manager,
        id,
        Some(&services.schedule_store),
    )
    .await
    {
        output::print_error(&format!("Failed to resume schedule {id}: {e}"));
        return Ok(());
    }
    output::print_success(&format!("Schedule resumed: {id}"));
    Ok(())
}

async fn cmd_history(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    let executions: Vec<ExecutionSummary> =
        SchedulerService::execution_history(&services.scheduler_manager, id).await;

    match mode {
        OutputMode::Json => {
            let json = serde_json::to_string_pretty(&executions)?;
            println!("{json}");
        }
        _ => {
            if executions.is_empty() {
                output::print_info(&format!("No execution history for schedule: {id}"));
            } else {
                let headers = &["Execution ID", "Status", "Started", "Duration"];
                let rows: Vec<TableRow> = executions
                    .iter()
                    .map(|e| TableRow {
                        cells: vec![
                            e.execution_id.clone(),
                            e.status.clone(),
                            e.started_at
                                .clone()
                                .unwrap_or_else(|| "—".to_string()),
                            e.duration_ms
                                .map(|d| format!("{d}ms"))
                                .unwrap_or_else(|| "—".to_string()),
                        ],
                    })
                    .collect();
                let table = output::format_table(headers, &rows);
                print!("{table}");
            }
        }
    }

    Ok(())
}

async fn cmd_trigger(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    let execution: ExecutionSummary =
        match SchedulerService::trigger_now(&services.scheduler_manager, id).await {
            Ok(exec) => exec,
            Err(e) => {
                output::print_error(&format!("Failed to trigger schedule {id}: {e}"));
                return Ok(());
            }
        };

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&execution)?;
        println!("{json}");
    } else {
        output::print_success(&format!("Schedule triggered: {id}"));
        println!("Execution ID: {}", execution.execution_id);
        println!("Status:       {}", execution.status);
        println!("Triggered At: {}", execution.triggered_at);
    }

    Ok(())
}
