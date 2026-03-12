//! Diagnostics commands: trace listing, inspection, cost analysis, and self-check.

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use clap::Subcommand;
use std::sync::Arc;
use uuid::Uuid;

use y_diagnostics::{
    CostIntelligence, TraceReplay, TraceSearch, TraceSearchQuery, TraceStatus, TraceStore,
};

use y_core::provider::ProviderPool;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Diagnostics subcommands.
#[derive(Debug, Subcommand)]
pub enum DiagAction {
    /// List recent traces.
    List {
        /// Filter by status (running, completed, failed).
        #[arg(long)]
        status: Option<String>,

        /// Filter by tags (comma-separated).
        #[arg(long)]
        tag: Option<Vec<String>>,

        /// Maximum number of results.
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Show a specific trace with its observations.
    Trace {
        /// Trace ID (UUID).
        id: String,

        /// Show replay-ready context with input/output.
        #[arg(long)]
        replay: bool,
    },

    /// Show cost summary for a date.
    Cost {
        /// Date (YYYY-MM-DD), defaults to today.
        #[arg(long)]
        date: Option<String>,
    },

    /// System health check.
    SelfCheck,
}

/// Run a diagnostics subcommand.
pub async fn run(action: &DiagAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    // Get the store from the diagnostics subscriber.
    let store = services.diagnostics.store();

    match action {
        DiagAction::List { status, tag, limit } => {
            run_list(store, status.as_deref(), tag.as_deref(), *limit, mode).await
        }
        DiagAction::Trace { id, replay } => run_trace(store, id, *replay, mode).await,
        DiagAction::Cost { date } => run_cost(store, date.as_deref(), mode).await,
        DiagAction::SelfCheck => run_self_check(services, mode).await,
    }
}

/// `diag list` — list recent traces with optional filters.
async fn run_list(
    store: Arc<dyn TraceStore>,
    status: Option<&str>,
    tags: Option<&[String]>,
    limit: usize,
    mode: OutputMode,
) -> Result<()> {
    let mut query = TraceSearchQuery::new().with_limit(limit);

    if let Some(s) = status {
        let parsed = match s {
            "running" | "active" => TraceStatus::Active,
            "completed" => TraceStatus::Completed,
            "failed" => TraceStatus::Failed,
            other => {
                anyhow::bail!("Unknown status: {other}. Use: running/active, completed, failed")
            }
        };
        query = query.with_status(parsed);
    }

    if let Some(t) = tags {
        query = query.with_tags(t.to_vec());
    }

    let search = TraceSearch::new(store);
    let traces = search
        .search(&query)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to search traces")?;

    if traces.is_empty() {
        output::print_info("No traces found.");
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
        ];
        let rows: Vec<TableRow> = traces
            .iter()
            .map(|t| TableRow {
                cells: vec![
                    t.id.to_string()[..8].to_string(),
                    t.name.clone(),
                    format!("{:?}", t.status),
                    t.started_at.format("%H:%M:%S").to_string(),
                    format!("{:.4}", t.total_cost_usd),
                    t.total_input_tokens.to_string(),
                    t.total_output_tokens.to_string(),
                ],
            })
            .collect();

        let table = output::format_table(headers, &rows);
        print!("{table}");
        output::print_info(&format!("{} trace(s) found", traces.len()));
    }

    Ok(())
}

/// `diag trace <id>` — show a specific trace with observations.
async fn run_trace(
    store: Arc<dyn TraceStore>,
    id: &str,
    replay: bool,
    mode: OutputMode,
) -> Result<()> {
    let trace_id = Uuid::parse_str(id).context("invalid trace UUID")?;

    let trace = store
        .get_trace(trace_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to get trace")?;

    if replay {
        // Replay mode: show observations with input/output.
        let replay_engine = TraceReplay::new(store);
        let result = replay_engine
            .replay(trace_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if mode == OutputMode::Json {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "trace": trace,
                "observations": result.steps,
            }))?;
            println!("{json}");
        } else {
            // Render tree format.
            let duration = trace
                .total_duration_ms.map_or_else(|| "…".to_string(), |d| format!("{:.1}s", d as f64 / 1000.0));
            println!(
                "Trace: {} (status: {:?}, {}, ${:.4})",
                trace.id, trace.status, duration, trace.total_cost_usd
            );

            for (i, obs) in result.steps.iter().enumerate() {
                let is_last = i == result.steps.len() - 1;
                let prefix = if is_last { "└─" } else { "├─" };
                let model_info = obs
                    .model
                    .as_deref()
                    .map(|m| {
                        format!(" ({m}, {}→{} tokens)", obs.input_tokens, obs.output_tokens)
                    })
                    .unwrap_or_default();
                let duration_ms = obs.duration_ms().unwrap_or(0);

                println!("{prefix} [{i}] {}{model_info} ({duration_ms}ms)", obs.name);

                // Show input/output in replay mode.
                let cont = if is_last { "   " } else { "│  " };
                if obs.input != serde_json::Value::Null {
                    let input_str = serde_json::to_string(&obs.input).unwrap_or_default();
                    let truncated = if input_str.len() > 120 {
                        format!("{}…", &input_str[..120])
                    } else {
                        input_str
                    };
                    println!("{cont} Input:  {truncated}");
                }
                if obs.output != serde_json::Value::Null {
                    let output_str = serde_json::to_string(&obs.output).unwrap_or_default();
                    let truncated = if output_str.len() > 120 {
                        format!("{}…", &output_str[..120])
                    } else {
                        output_str
                    };
                    println!("{cont} Output: {truncated}");
                }
                if let Some(ref err) = obs.error_message {
                    println!("{cont} Error:  {err}");
                }
            }
            println!();
        }
    } else {
        // Summary mode: show trace + observation table.
        let observations = store
            .get_observations(trace_id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if mode == OutputMode::Json {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "trace": trace,
                "observations": observations,
            }))?;
            println!("{json}");
        } else {
            println!("Trace: {}", trace.id);
            println!("  Name:     {}", trace.name);
            println!("  Status:   {:?}", trace.status);
            println!("  Session:  {}", trace.session_id);
            println!("  Started:  {}", trace.started_at);
            if let Some(ended) = trace.completed_at {
                println!("  Ended:    {ended}");
            }
            if let Some(dur) = trace.total_duration_ms {
                println!("  Duration: {dur}ms");
            }
            println!(
                "  Tokens:   {} in / {} out",
                trace.total_input_tokens, trace.total_output_tokens
            );
            println!("  Cost:     ${:.4}", trace.total_cost_usd);
            println!("  Tags:     {}", trace.tags.join(", "));
            println!();

            if observations.is_empty() {
                output::print_info("No observations recorded.");
            } else {
                let headers = &[
                    "#", "Type", "Name", "Model", "Tokens", "Cost ($)", "Duration",
                ];
                let rows: Vec<TableRow> = observations
                    .iter()
                    .enumerate()
                    .map(|(i, o)| TableRow {
                        cells: vec![
                            i.to_string(),
                            format!("{:?}", o.obs_type),
                            o.name.clone(),
                            o.model.clone().unwrap_or_else(|| "—".into()),
                            format!("{}→{}", o.input_tokens, o.output_tokens),
                            format!("{:.4}", o.cost_usd),
                            o.duration_ms().map_or_else(|| "—".into(), |d| format!("{d}ms")),
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

/// `diag cost` — show daily cost summary.
async fn run_cost(
    store: Arc<dyn TraceStore>,
    date_str: Option<&str>,
    mode: OutputMode,
) -> Result<()> {
    let date = if let Some(s) = date_str {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").context("invalid date format, use YYYY-MM-DD")?
    } else {
        Utc::now().date_naive()
    };

    let cost = CostIntelligence::new(store);
    let summary = cost
        .daily_summary(date)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to compute cost summary")?;

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&summary)?;
        println!("{json}");
    } else {
        println!("Cost Summary for {}:", summary.date);
        println!("  Total traces: {}", summary.total_traces);
        println!("  Total cost:   ${:.4}", summary.total_cost_usd);

        if !summary.by_model.is_empty() {
            println!("  By model:");
            for record in &summary.by_model {
                println!(
                    "    {:<16} ${:.4}  ({} in, {} out tokens)",
                    record.model, record.cost_usd, record.input_tokens, record.output_tokens
                );
            }
        }
        println!();
    }

    Ok(())
}

/// `diag self-check` — system health check.
async fn run_self_check(services: &AppServices, _mode: OutputMode) -> Result<()> {
    println!("Diagnostics Self-Check");
    println!("──────────────────────");

    // 1. Check trace store connectivity.
    let store = services.diagnostics.store();
    match store.list_traces(None, None, 1).await {
        Ok(traces) => {
            output::print_success("Trace store: connected");
            // Count recent traces (last 30 days).
            let since = Utc::now() - chrono::Duration::days(30);
            match store.list_traces(None, Some(since), 10_000).await {
                Ok(recent) => println!("  Traces (last 30d): {}", recent.len()),
                Err(_) => println!("  Traces (last 30d): (count unavailable)"),
            }
            if !traces.is_empty() {
                println!("  Latest trace:      {}", traces[0].started_at);
            }
        }
        Err(e) => {
            output::print_error(&format!("Trace store: unavailable ({e})"));
        }
    }

    // 2. Provider status.
    let statuses = services.provider_pool.provider_statuses().await;
    let active = statuses.iter().filter(|s| !s.is_frozen).count();
    let frozen = statuses.len() - active;
    println!("  Providers:         {active} active, {frozen} frozen");

    // 3. PG configuration.
    println!(
        "  PG feature:        {}",
        if cfg!(feature = "diagnostics_pg") {
            "enabled"
        } else {
            "disabled"
        }
    );

    println!();
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
        command: DiagAction,
    }

    // T-CLI-008-01: test_parse_diag_list_command
    #[test]
    fn test_parse_diag_list_command() {
        let cli = TestCli::parse_from(["y-agent", "list"]);
        assert!(matches!(cli.command, DiagAction::List { .. }));
    }

    // T-CLI-008-02: test_parse_diag_trace_command
    #[test]
    fn test_parse_diag_trace_command() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let cli = TestCli::parse_from(["y-agent", "trace", id]);
        match cli.command {
            DiagAction::Trace {
                id: parsed_id,
                replay,
            } => {
                assert_eq!(parsed_id, id);
                assert!(!replay);
            }
            _ => panic!("expected Trace"),
        }
    }

    // T-CLI-008-03: test_parse_diag_trace_replay
    #[test]
    fn test_parse_diag_trace_replay() {
        let id = "12345678-1234-1234-1234-123456789abc";
        let cli = TestCli::parse_from(["y-agent", "trace", id, "--replay"]);
        match cli.command {
            DiagAction::Trace { replay, .. } => assert!(replay),
            _ => panic!("expected Trace"),
        }
    }

    // T-CLI-008-04: test_parse_diag_cost_command
    #[test]
    fn test_parse_diag_cost_command() {
        let cli = TestCli::parse_from(["y-agent", "cost", "--date", "2026-03-10"]);
        match cli.command {
            DiagAction::Cost { date } => assert_eq!(date, Some("2026-03-10".to_string())),
            _ => panic!("expected Cost"),
        }
    }

    // T-CLI-008-05: test_parse_diag_self_check
    #[test]
    fn test_parse_diag_self_check() {
        let cli = TestCli::parse_from(["y-agent", "self-check"]);
        assert!(matches!(cli.command, DiagAction::SelfCheck));
    }
}
