//! Schedule management commands.

use anyhow::{bail, Context, Result};
use clap::Subcommand;

use y_service::{
    CreateScheduleRequest, ExecutionSummary, SchedulePolicies, ScheduleSummary, SchedulerService,
    TriggerConfig, UpdateScheduleRequest,
};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Schedule subcommands.
#[derive(Debug, Subcommand)]
pub enum ScheduleAction {
    /// List all schedules.
    List,

    /// Create a schedule from a typed JSON trigger.
    Create {
        /// Human-readable schedule name.
        #[arg(long)]
        name: String,
        /// Workflow ID executed by the schedule.
        #[arg(long)]
        workflow_id: String,
        /// Trigger JSON, for example `{"type":"interval","interval_secs":300}`.
        #[arg(long)]
        trigger: String,
        /// Workflow parameter values as a JSON object.
        #[arg(long, default_value = "{}")]
        parameters: String,
        /// Human-readable schedule description.
        #[arg(long, default_value = "")]
        description: String,
        /// Tag attached to the schedule. May be repeated.
        #[arg(long = "tag")]
        tags: Vec<String>,
    },

    /// Show detailed info about a schedule by ID.
    Get {
        /// Schedule ID.
        id: String,
    },

    /// Update selected fields on an existing schedule.
    Update {
        /// Schedule ID.
        id: String,
        /// Updated human-readable name.
        #[arg(long)]
        name: Option<String>,
        /// Updated workflow ID.
        #[arg(long)]
        workflow_id: Option<String>,
        /// Updated typed trigger JSON.
        #[arg(long)]
        trigger: Option<String>,
        /// Updated workflow parameter values as a JSON object.
        #[arg(long)]
        parameters: Option<String>,
        /// Updated description.
        #[arg(long)]
        description: Option<String>,
        /// Replacement tag set. May be repeated.
        #[arg(long = "tag")]
        tags: Vec<String>,
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

    /// Show one execution record by execution ID.
    Execution {
        /// Execution ID.
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
        ScheduleAction::Create {
            name,
            workflow_id,
            trigger,
            parameters,
            description,
            tags,
        } => {
            cmd_create(
                services,
                CreateScheduleRequest {
                    name: name.clone(),
                    trigger: parse_trigger(trigger)?,
                    workflow_id: workflow_id.clone(),
                    parameter_values: parse_json_object(parameters, "parameters")?,
                    policies: SchedulePolicies::default(),
                    description: description.clone(),
                    tags: tags.clone(),
                },
                mode,
            )
            .await
        }
        ScheduleAction::Get { id } => cmd_get(services, id, mode).await,
        ScheduleAction::Update {
            id,
            name,
            workflow_id,
            trigger,
            parameters,
            description,
            tags,
        } => {
            if name.is_none()
                && workflow_id.is_none()
                && trigger.is_none()
                && parameters.is_none()
                && description.is_none()
                && tags.is_empty()
            {
                bail!("schedule update requires at least one changed field");
            }
            cmd_update(
                services,
                id,
                UpdateScheduleRequest {
                    name: name.clone(),
                    trigger: trigger.as_deref().map(parse_trigger).transpose()?,
                    workflow_id: workflow_id.clone(),
                    parameter_values: parameters
                        .as_deref()
                        .map(|value| parse_json_object(value, "parameters"))
                        .transpose()?,
                    policies: None,
                    description: description.clone(),
                    tags: (!tags.is_empty()).then(|| tags.clone()),
                },
                mode,
            )
            .await
        }
        ScheduleAction::Delete { id } => cmd_delete(services, id, mode).await,
        ScheduleAction::Pause { id } => cmd_pause(services, id, mode).await,
        ScheduleAction::Resume { id } => cmd_resume(services, id, mode).await,
        ScheduleAction::History { id } => cmd_history(services, id, mode).await,
        ScheduleAction::Execution { id } => cmd_execution(services, id, mode).await,
        ScheduleAction::Trigger { id } => cmd_trigger(services, id, mode).await,
    }
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

async fn cmd_list(services: &AppServices, mode: OutputMode) -> Result<()> {
    let schedules: Vec<ScheduleSummary> = SchedulerService::list(&services.scheduler_manager).await;

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

async fn cmd_create(
    services: &AppServices,
    request: CreateScheduleRequest,
    mode: OutputMode,
) -> Result<()> {
    let schedule = SchedulerService::create(
        &services.scheduler_manager,
        &request,
        Some(&services.schedule_store),
    )
    .await?;
    print_schedule(&schedule, mode);
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

    print_schedule(&schedule, mode);

    Ok(())
}

async fn cmd_update(
    services: &AppServices,
    id: &str,
    request: UpdateScheduleRequest,
    mode: OutputMode,
) -> Result<()> {
    let schedule = SchedulerService::update(
        &services.scheduler_manager,
        id,
        &request,
        Some(&services.schedule_store),
    )
    .await?;
    print_schedule(&schedule, mode);
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
                            e.started_at.clone().unwrap_or_else(|| "—".to_string()),
                            e.duration_ms
                                .map_or_else(|| "—".to_string(), |d| format!("{d}ms")),
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

async fn cmd_execution(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    let execution = SchedulerService::get_execution(&services.scheduler_manager, id).await?;
    if mode == OutputMode::Json {
        println!("{}", serde_json::to_string_pretty(&execution)?);
    } else {
        println!("Execution ID: {}", execution.execution_id);
        println!("Schedule ID:  {}", execution.schedule_id);
        println!("Status:       {}", execution.status);
        println!("Triggered At: {}", execution.triggered_at);
        if let Some(duration_ms) = execution.duration_ms {
            println!("Duration:     {duration_ms}ms");
        }
        if let Some(error) = execution.error_message {
            println!("Error:        {error}");
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

fn parse_trigger(value: &str) -> Result<TriggerConfig> {
    serde_json::from_str(value).context("invalid schedule trigger JSON")
}

fn parse_json_object(value: &str, label: &str) -> Result<serde_json::Value> {
    let parsed: serde_json::Value =
        serde_json::from_str(value).with_context(|| format!("invalid {label} JSON"))?;
    if !parsed.is_object() {
        bail!("{label} must be a JSON object");
    }
    Ok(parsed)
}

fn print_schedule(schedule: &ScheduleSummary, mode: OutputMode) {
    if mode == OutputMode::Json {
        println!("{}", output::format_value(schedule, mode));
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trigger_accepts_interval_configuration() {
        let trigger = parse_trigger(r#"{"type":"interval","interval_secs":60}"#).unwrap();

        assert!(matches!(
            trigger,
            TriggerConfig::Interval { interval_secs: 60 }
        ));
    }

    #[test]
    fn test_parse_json_object_rejects_non_object_parameters() {
        let error = parse_json_object("[]", "parameters").unwrap_err();

        assert!(error.to_string().contains("must be a JSON object"));
    }
}
