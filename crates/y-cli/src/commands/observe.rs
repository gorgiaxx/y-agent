//! `observe` CLI command -- observability and live system state.
//!
//! Subcommands:
//! - `snapshot` -- show current system state snapshot
//! - `history` -- show historical metrics over a lookback window
//! - `memory` -- show in-memory collection sizes for diagnostics

use anyhow::Result;
use clap::Subcommand;

use y_service::{ObservabilityService, SystemService};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Observability subcommands.
#[derive(Debug, Subcommand)]
pub enum ObserveAction {
    /// Show current system state snapshot.
    Snapshot,

    /// Show historical metrics over a lookback window.
    History {
        /// Lookback window in hours (default: 24).
        #[arg(long, default_value = "24")]
        hours: u64,
    },

    /// Show in-memory collection sizes for diagnostics.
    Memory,
}

/// Run an observability subcommand.
pub async fn run(action: &ObserveAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        ObserveAction::Snapshot => run_snapshot(services, mode).await,
        ObserveAction::History { hours } => run_history(services, *hours, mode).await,
        ObserveAction::Memory => run_memory(services, mode).await,
    }
}

/// `observe snapshot` -- capture a point-in-time system snapshot.
async fn run_snapshot(services: &AppServices, mode: OutputMode) -> Result<()> {
    let snap = ObservabilityService::snapshot(services).await;

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&snap, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            output::print_info(&format!(
                "System snapshot at {}",
                snap.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
            ));

            if snap.providers.is_empty() {
                output::print_info("No providers registered");
            } else {
                println!("\nProviders:");
                let headers = &[
                    "Provider",
                    "Model",
                    "Frozen",
                    "Active Reqs",
                    "Total Reqs",
                    "Errors",
                    "Cost USD",
                ];
                let rows: Vec<TableRow> = snap
                    .providers
                    .iter()
                    .map(|p| TableRow {
                        cells: vec![
                            p.id.clone(),
                            p.model.clone(),
                            if p.is_frozen { "yes" } else { "no" }.to_string(),
                            p.active_requests.to_string(),
                            p.total_requests.to_string(),
                            p.total_errors.to_string(),
                            format!("{:.4}", p.estimated_cost_usd),
                        ],
                    })
                    .collect();
                let table = output::format_table(headers, &rows);
                print!("{table}");
            }

            println!("\nAgent Pool:");
            output::print_status("Total instances", &snap.agents.total_instances.to_string());
            output::print_status(
                "Active instances",
                &snap.agents.active_instances.to_string(),
            );
            output::print_status("Available slots", &snap.agents.available_slots.to_string());

            if let Some(sched) = &snap.scheduler {
                println!("\nScheduler:");
                output::print_status("Active critical", &sched.active_critical.to_string());
                output::print_status("Active normal", &sched.active_normal.to_string());
                output::print_status("Active idle", &sched.active_idle.to_string());
                output::print_status("Total capacity", &sched.total_capacity.to_string());
            }
        }
    }

    Ok(())
}

/// `observe history --hours <N>` -- historical metrics over a lookback window.
async fn run_history(services: &AppServices, hours: u64, mode: OutputMode) -> Result<()> {
    let since =
        chrono::Utc::now() - chrono::Duration::hours(i64::try_from(hours).unwrap_or(i64::MAX));
    let snap = ObservabilityService::snapshot_with_history(services, Some(since), None).await;

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&snap, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            output::print_info(&format!(
                "Historical metrics (last {hours}h) at {}",
                snap.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
            ));

            if snap.providers.is_empty() {
                output::print_info("No providers registered");
            } else {
                let headers = &[
                    "Provider",
                    "Model",
                    "Frozen",
                    "Active Reqs",
                    "Total Reqs",
                    "Errors",
                    "Cost USD",
                ];
                let rows: Vec<TableRow> = snap
                    .providers
                    .iter()
                    .map(|p| TableRow {
                        cells: vec![
                            p.id.clone(),
                            p.model.clone(),
                            if p.is_frozen { "yes" } else { "no" }.to_string(),
                            p.active_requests.to_string(),
                            p.total_requests.to_string(),
                            p.total_errors.to_string(),
                            format!("{:.4}", p.estimated_cost_usd),
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

/// `observe memory` -- in-memory collection sizes for diagnostics.
async fn run_memory(services: &AppServices, mode: OutputMode) -> Result<()> {
    // CLI does not track pending runs or turn metadata cache; pass 0.
    let stats = SystemService::memory_stats(services, 0, 0).await;

    match mode {
        OutputMode::Json => {
            println!("{}", output::format_value(&stats, mode));
        }
        OutputMode::Table | OutputMode::Plain => {
            output::print_info("In-memory collection sizes:");
            output::print_status("Pending runs", &stats.pending_runs.to_string());
            output::print_status("Turn meta cache", &stats.turn_meta_cache.to_string());
            output::print_status("Pruning watermarks", &stats.pruning_watermarks.to_string());
            output::print_status(
                "Session permission modes",
                &stats.session_permission_modes.to_string(),
            );
            output::print_status(
                "Pending interactions",
                &stats.pending_interactions.to_string(),
            );
            output::print_status(
                "Pending permissions",
                &stats.pending_permissions.to_string(),
            );
            output::print_status(
                "File-history sessions",
                &stats.file_history_sessions.to_string(),
            );
            output::print_status(
                "File-history total snapshots",
                &stats.file_history_total_snapshots.to_string(),
            );
        }
    }

    Ok(())
}
