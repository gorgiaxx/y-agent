//! Workflow management commands.

use std::collections::HashSet;

use anyhow::Result;
use clap::Subcommand;

use y_agent::orchestrator::dag::TaskDag;
use y_agent::orchestrator::expression_dsl;
use y_service::scheduler_service::SchedulerService;
use y_service::workflow_service::ValidationResult;
use y_service::{
    DagVisualization, ExecutionSummary, UpdateWorkflowRequest, WorkflowRow, WorkflowService,
};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Workflow subcommands.
#[derive(Debug, Subcommand)]
pub enum WorkflowAction {
    /// List all saved workflows.
    List,

    /// Show detailed info about a workflow (by ID or name).
    Get {
        /// Workflow ID or name.
        identifier: String,
    },

    /// Create and persist a new workflow.
    Create {
        /// Workflow name (unique).
        #[arg(long)]
        name: String,

        /// Workflow definition (Expression DSL string).
        #[arg(long)]
        def: String,

        /// Description.
        #[arg(long)]
        description: Option<String>,

        /// Comma-separated tags.
        #[arg(long)]
        tags: Option<String>,
    },

    /// Delete a workflow by ID.
    Delete {
        /// Workflow ID.
        id: String,
    },

    /// Parse a DSL expression and print the AST + compiled DAG (no persistence).
    Parse {
        /// Expression DSL string, e.g. "a >> (b | c) >> d".
        expression: String,
    },

    /// Update an existing workflow's definition, description, or tags.
    Update {
        /// Workflow ID.
        id: String,

        /// Updated definition body (DSL expression or TOML content).
        #[arg(long)]
        def: Option<String>,

        /// Updated description.
        #[arg(long)]
        description: Option<String>,

        /// Updated comma-separated tags.
        #[arg(long)]
        tags: Option<String>,
    },

    /// Validate a workflow definition without persisting.
    Validate {
        /// Definition body (DSL expression or TOML content).
        def: String,

        /// Definition format: "`expression_dsl`" or "toml".
        #[arg(long, default_value = "expression_dsl")]
        format: String,
    },

    /// Show the DAG (nodes and edges) for a stored workflow.
    Dag {
        /// Workflow ID.
        id: String,
    },

    /// Manually execute a workflow.
    Execute {
        /// Workflow ID.
        id: String,
    },
}

/// Run a workflow subcommand.
pub async fn run(action: &WorkflowAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        WorkflowAction::List => cmd_list(services, mode).await,
        WorkflowAction::Get { identifier } => cmd_get(services, identifier, mode).await,
        WorkflowAction::Create {
            name,
            def,
            description,
            tags,
        } => {
            cmd_create(
                services,
                name,
                def,
                description.as_deref(),
                tags.as_deref(),
                mode,
            )
            .await
        }
        WorkflowAction::Delete { id } => cmd_delete(services, id, mode).await,
        WorkflowAction::Parse { expression } => cmd_parse(expression, mode),
        WorkflowAction::Update {
            id,
            def,
            description,
            tags,
        } => {
            cmd_update(
                services,
                id,
                def.as_deref(),
                description.as_deref(),
                tags.as_deref(),
                mode,
            )
            .await
        }
        WorkflowAction::Validate { def, format } => cmd_validate(def, format, mode),
        WorkflowAction::Dag { id } => cmd_dag(services, id, mode).await,
        WorkflowAction::Execute { id } => cmd_execute(services, id, mode).await,
    }
}

// ---------------------------------------------------------------------------
// Subcommand handlers
// ---------------------------------------------------------------------------

async fn cmd_list(services: &AppServices, mode: OutputMode) -> Result<()> {
    let rows = services.workflow_store.list().await?;

    match mode {
        OutputMode::Json => {
            let json = serde_json::to_string_pretty(&rows)?;
            println!("{json}");
        }
        _ => {
            if rows.is_empty() {
                output::print_info("No workflows found");
            } else {
                let headers = &["ID", "Name", "Format", "Tags", "Creator"];
                let table_rows: Vec<TableRow> = rows
                    .iter()
                    .map(|r| TableRow {
                        cells: vec![
                            r.id.clone(),
                            r.name.clone(),
                            r.format.clone(),
                            r.tags.clone(),
                            r.creator.clone(),
                        ],
                    })
                    .collect();
                let table = output::format_table(headers, &table_rows);
                print!("{table}");
            }
        }
    }

    Ok(())
}

async fn cmd_get(services: &AppServices, identifier: &str, mode: OutputMode) -> Result<()> {
    // Try by ID first, then by name.
    let row = match services.workflow_store.get(identifier).await? {
        Some(r) => r,
        None => {
            if let Some(r) = services.workflow_store.get_by_name(identifier).await? {
                r
            } else {
                output::print_error(&format!("Workflow not found: {identifier}"));
                return Ok(());
            }
        }
    };

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&row)?;
        println!("{json}");
    } else {
        println!("ID:          {}", row.id);
        println!("Name:        {}", row.name);
        println!("Format:      {}", row.format);
        println!("Creator:     {}", row.creator);
        println!("Tags:        {}", row.tags);
        if let Some(ref desc) = row.description {
            println!("Description: {desc}");
        }
        println!("Created:     {}", row.created_at);
        println!("Updated:     {}", row.updated_at);
        println!();
        println!("─── Definition ───");
        println!("{}", row.definition);
        println!();
        // Try to parse and show DAG info if it's a DSL expression.
        if row.format == "expression_dsl" {
            if let Ok(ast) = expression_dsl::parse(&row.definition) {
                println!("─── AST ───");
                println!("{ast}");
                println!();
                if let Ok(dag) = ast.to_task_dag() {
                    print_dag_info(&dag);
                }
            }
        }
    }

    Ok(())
}

async fn cmd_create(
    services: &AppServices,
    name: &str,
    def: &str,
    description: Option<&str>,
    tags: Option<&str>,
    _mode: OutputMode,
) -> Result<()> {
    // Validate: parse the DSL expression to ensure it's valid.
    let ast = expression_dsl::parse(def)?;
    let dag = ast.to_task_dag()?;
    dag.validate()?;

    // Compile DAG to JSON.
    let topo = dag.topological_order()?;
    let compiled_dag = serde_json::to_string(&topo)?;

    // Build tags JSON array.
    let tags_json = match tags {
        Some(t) => {
            let tag_list: Vec<&str> = t.split(',').map(str::trim).collect();
            serde_json::to_string(&tag_list)?
        }
        None => "[]".to_string(),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let row = WorkflowRow {
        id: id.clone(),
        name: name.to_string(),
        description: description.map(str::to_string),
        definition: def.to_string(),
        format: "expression_dsl".to_string(),
        compiled_dag,
        parameter_schema: None,
        tags: tags_json,
        creator: "user".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };

    services.workflow_store.save(&row).await?;
    output::print_success(&format!("Workflow created: {name} (ID: {id})"));

    Ok(())
}

async fn cmd_delete(services: &AppServices, id: &str, _mode: OutputMode) -> Result<()> {
    let deleted = services.workflow_store.delete(id).await?;
    if deleted {
        output::print_success(&format!("Workflow deleted: {id}"));
    } else {
        output::print_error(&format!("Workflow not found: {id}"));
    }
    Ok(())
}

fn cmd_parse(expression: &str, mode: OutputMode) -> Result<()> {
    // 1. Tokenize
    let tokens = expression_dsl::tokenize(expression)?;

    // 2. Parse AST
    let ast = expression_dsl::parse(expression)?;

    // 3. Compile to DAG
    let dag = ast.to_task_dag()?;
    dag.validate()?;

    if mode == OutputMode::Json {
        let topo = dag.topological_order()?;
        let output = serde_json::json!({
            "expression": expression,
            "ast": ast.to_string(),
            "tokens": tokens.iter().map(|(t, pos)| {
                serde_json::json!({"token": format!("{t:?}"), "pos": pos})
            }).collect::<Vec<_>>(),
            "topological_order": topo,
            "task_count": dag.len(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("─── Expression ───");
        println!("{expression}");
        println!();
        println!("─── Tokens ───");
        for (tok, pos) in &tokens {
            println!("  [{pos:>3}] {tok:?}");
        }
        println!();
        println!("─── AST (Display) ───");
        println!("{ast}");
        println!();
        println!("─── AST (Debug) ───");
        println!("{ast:#?}");
        println!();
        print_dag_info(&dag);
    }

    Ok(())
}

/// Print DAG info: task count, topological order, dependency edges.
fn print_dag_info(dag: &TaskDag) {
    let topo = match dag.topological_order() {
        Ok(t) => t,
        Err(e) => {
            output::print_error(&format!("Failed to compute topological order: {e}"));
            return;
        }
    };
    println!("─── DAG ({} tasks) ───", dag.len());
    println!("Topological order: {}", topo.join(" → "));
    println!();
    println!("Dependencies:");
    let completed = HashSet::new();
    let all_tasks = dag.ready_tasks(&completed);
    // Collect all tasks via iterative readiness check.
    let mut visited = HashSet::new();
    let mut queue = all_tasks;

    // Simple BFS to print dependencies for each task.
    while !queue.is_empty() {
        for task in &queue {
            if task.dependencies.is_empty() {
                println!("  {} (root)", task.id);
            } else {
                println!("  {} ← [{}]", task.id, task.dependencies.join(", "));
            }
            visited.insert(task.id.clone());
        }
        queue = dag.ready_tasks(&visited);
        // Remove already-visited to avoid infinite loop.
        queue.retain(|t| !visited.contains(&t.id));
    }
}

// ---------------------------------------------------------------------------
// New subcommand handlers (matching GUI automation commands)
// ---------------------------------------------------------------------------

async fn cmd_update(
    services: &AppServices,
    id: &str,
    def: Option<&str>,
    description: Option<&str>,
    tags: Option<&str>,
    mode: OutputMode,
) -> Result<()> {
    let req = UpdateWorkflowRequest {
        definition: def.map(str::to_string),
        // Preserve existing format when only updating the definition; the
        // service defaults to the stored format when `format` is None.
        format: None,
        description: description.map(str::to_string),
        tags: tags.map(str::to_string),
    };

    let row = WorkflowService::update(&services.workflow_store, id, &req).await?;

    match mode {
        OutputMode::Json => {
            let json = serde_json::to_string_pretty(&row)?;
            println!("{json}");
        }
        _ => {
            output::print_success(&format!("Workflow updated: {} (ID: {})", row.name, row.id));
        }
    }

    Ok(())
}

fn cmd_validate(def: &str, format: &str, mode: OutputMode) -> Result<()> {
    let result: ValidationResult = WorkflowService::validate_definition(def, format);

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&result)?;
        println!("{json}");
    } else {
        if result.valid {
            output::print_success("Definition is valid");
        } else {
            output::print_error("Definition is invalid");
        }
        if !result.errors.is_empty() {
            println!();
            println!("─── Errors ───");
            for err in &result.errors {
                println!("  - {err}");
            }
        }
        if let Some(ast_display) = &result.ast_display {
            println!();
            println!("─── AST ───");
            println!("{ast_display}");
        }
        if let Some(dag) = &result.dag {
            println!();
            print_dag_visualization(dag);
        }
    }

    Ok(())
}

async fn cmd_dag(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    let dag: DagVisualization =
        WorkflowService::get_dag_visualization(&services.workflow_store, id).await?;

    match mode {
        OutputMode::Json => {
            let json = serde_json::to_string_pretty(&dag)?;
            println!("{json}");
        }
        _ => {
            print_dag_visualization(&dag);
        }
    }

    Ok(())
}

async fn cmd_execute(services: &AppServices, id: &str, mode: OutputMode) -> Result<()> {
    // Verify the workflow exists and get its name (mirrors the GUI pattern).
    let wf = WorkflowService::get(&services.workflow_store, id).await?;

    let execution: ExecutionSummary =
        SchedulerService::execute_workflow(&services.scheduler_manager, &wf.id, &wf.name).await?;

    if mode == OutputMode::Json {
        let json = serde_json::to_string_pretty(&execution)?;
        println!("{json}");
    } else {
        output::print_success(&format!(
            "Workflow '{}' execution started (ID: {})",
            wf.name, execution.execution_id
        ));
        println!("Status:    {}", execution.status);
        println!("Triggered: {}", execution.triggered_at);
        if let Some(err) = &execution.error_message {
            output::print_error(&format!("Error: {err}"));
        }
    }

    Ok(())
}

/// Print a `DagVisualization` (nodes, edges, topological order).
fn print_dag_visualization(dag: &DagVisualization) {
    println!(
        "─── DAG ({} nodes, {} edges) ───",
        dag.nodes.len(),
        dag.edges.len()
    );
    println!();
    println!("Topological order: {}", dag.topological_order.join(" -> "));
    println!();
    println!("Nodes:");
    let headers = &[
        "ID",
        "Name",
        "Type",
        "Priority",
        "Dependencies",
        "Retry",
        "Failure",
    ];
    let table_rows: Vec<TableRow> = dag
        .nodes
        .iter()
        .map(|n| TableRow {
            cells: vec![
                n.id.clone(),
                n.name.clone(),
                n.task_type.clone(),
                n.priority.clone(),
                n.dependencies.join(", "),
                if n.has_retry { "yes" } else { "no" }.to_string(),
                n.failure_strategy.clone(),
            ],
        })
        .collect();
    let table = output::format_table(headers, &table_rows);
    print!("{table}");
    println!();
    println!("Edges:");
    for edge in &dag.edges {
        println!("  {} -> {}", edge.from, edge.to);
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[derive(Parser)]
    #[command(name = "y-agent")]
    struct TestCli {
        #[command(subcommand)]
        command: Option<crate::commands::Commands>,
    }

    #[test]
    fn test_parse_workflow_list() {
        let cli = TestCli::parse_from(["y-agent", "workflow", "list"]);
        assert!(matches!(
            cli.command,
            Some(crate::commands::Commands::Workflow {
                action: super::WorkflowAction::List
            })
        ));
    }

    #[test]
    fn test_parse_workflow_get() {
        let cli = TestCli::parse_from(["y-agent", "workflow", "get", "my-wf"]);
        match cli.command {
            Some(crate::commands::Commands::Workflow {
                action: super::WorkflowAction::Get { identifier },
            }) => assert_eq!(identifier, "my-wf"),
            _ => panic!("expected Workflow Get"),
        }
    }

    #[test]
    fn test_parse_workflow_create() {
        let cli = TestCli::parse_from([
            "y-agent",
            "workflow",
            "create",
            "--name",
            "my-wf",
            "--def",
            "a >> b >> c",
            "--tags",
            "research,llm",
        ]);
        match cli.command {
            Some(crate::commands::Commands::Workflow {
                action:
                    super::WorkflowAction::Create {
                        name, def, tags, ..
                    },
            }) => {
                assert_eq!(name, "my-wf");
                assert_eq!(def, "a >> b >> c");
                assert_eq!(tags, Some("research,llm".to_string()));
            }
            _ => panic!("expected Workflow Create"),
        }
    }

    #[test]
    fn test_parse_workflow_parse() {
        let cli = TestCli::parse_from(["y-agent", "workflow", "parse", "a >> (b | c) >> d"]);
        match cli.command {
            Some(crate::commands::Commands::Workflow {
                action: super::WorkflowAction::Parse { expression },
            }) => assert_eq!(expression, "a >> (b | c) >> d"),
            _ => panic!("expected Workflow Parse"),
        }
    }

    #[test]
    fn test_parse_workflow_delete() {
        let cli = TestCli::parse_from(["y-agent", "workflow", "delete", "wf-123"]);
        match cli.command {
            Some(crate::commands::Commands::Workflow {
                action: super::WorkflowAction::Delete { id },
            }) => assert_eq!(id, "wf-123"),
            _ => panic!("expected Workflow Delete"),
        }
    }
}
