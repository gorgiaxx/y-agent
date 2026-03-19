//! Tool management commands.

use anyhow::Result;
use clap::Subcommand;

use y_core::tool::ToolRegistry;
use y_core::types::ToolName;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Tool subcommands.
#[derive(Debug, Subcommand)]
pub enum ToolAction {
    /// List all registered tools.
    List,

    /// Search tools by query.
    Search {
        /// Search query.
        query: String,
    },

    /// Show detailed info about a tool.
    Info {
        /// Tool name.
        name: String,
    },
}

/// Run a tool subcommand.
pub async fn run(action: &ToolAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        ToolAction::List => {
            let index = services.tool_registry.tool_index().await;

            let headers = &["Name", "Category", "Description"];
            let rows: Vec<TableRow> = index
                .iter()
                .map(|entry| TableRow {
                    cells: vec![
                        entry.name.0.clone(),
                        format!("{:?}", entry.category),
                        entry.description.clone(),
                    ],
                })
                .collect();

            match mode {
                OutputMode::Json => {
                    let json = serde_json::to_string_pretty(&index)?;
                    println!("{json}");
                }
                _ => {
                    if rows.is_empty() {
                        output::print_info("No tools registered");
                    } else {
                        let table = output::format_table(headers, &rows);
                        print!("{table}");
                    }
                }
            }
        }
        ToolAction::Search { query } => {
            let results = services.tool_registry.search_tools(query, None).await;

            let headers = &["Name", "Type", "Description"];
            let rows: Vec<TableRow> = results
                .iter()
                .map(|def| TableRow {
                    cells: vec![
                        def.name.0.clone(),
                        format!("{:?}", def.tool_type),
                        def.description.clone(),
                    ],
                })
                .collect();

            match mode {
                OutputMode::Json => {
                    let json = serde_json::to_string_pretty(&results)?;
                    println!("{json}");
                }
                _ => {
                    if rows.is_empty() {
                        output::print_info(&format!("No tools matching '{query}'"));
                    } else {
                        output::print_info(&format!("{} tool(s) found:", rows.len()));
                        let table = output::format_table(headers, &rows);
                        print!("{table}");
                    }
                }
            }
        }
        ToolAction::Info { name } => {
            let tool_name = ToolName(name.clone());
            match services.tool_registry.get_definition(&tool_name).await {
                Some(def) => {
                    if mode == OutputMode::Json {
                        let json = serde_json::to_string_pretty(&def)?;
                        println!("{json}");
                    } else {
                        println!("Tool: {}", def.name.0);
                        println!("Type: {:?}", def.tool_type);
                        println!("Description: {}", def.description);
                        println!("Category: {:?}", def.category);
                        println!("Capabilities: {:?}", def.capabilities);
                    }
                }
                None => {
                    output::print_error(&format!("Tool not found: {name}"));
                }
            }
        }
    }

    Ok(())
}
