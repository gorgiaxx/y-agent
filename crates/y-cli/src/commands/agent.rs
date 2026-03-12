//! Agent management commands.

use anyhow::Result;
use clap::Subcommand;

use y_agent::agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
use y_agent::TrustTier;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Agent subcommands.
#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// List all defined agents.
    List,

    /// Define a new agent.
    Define {
        /// Agent name.
        name: String,

        /// Agent description.
        #[arg(long)]
        description: Option<String>,
    },

    /// Delegate a task to an agent.
    Delegate {
        /// Target agent name.
        agent: String,

        /// Task description.
        task: String,
    },
}

/// Run an agent subcommand.
pub async fn run(action: &AgentAction, services: &AppServices, mode: OutputMode) -> Result<()> {
    match action {
        AgentAction::List => {
            let registry = services.agent_registry.lock().await;
            let agents = registry.list();

            let headers = &["Name", "Description", "Mode", "Trust"];
            let rows: Vec<TableRow> = agents
                .iter()
                .map(|def| TableRow {
                    cells: vec![
                        def.name.clone(),
                        def.description.clone(),
                        format!("{:?}", def.mode),
                        format!("{:?}", def.trust_tier),
                    ],
                })
                .collect();

            match mode {
                OutputMode::Json => {
                    let json = serde_json::to_string_pretty(&agents)?;
                    println!("{json}");
                }
                _ => {
                    if rows.is_empty() {
                        output::print_info("No agents defined");
                    } else {
                        let table = output::format_table(headers, &rows);
                        print!("{table}");
                    }
                }
            }
        }
        AgentAction::Define { name, description } => {
            let desc = description
                .clone()
                .unwrap_or_else(|| "(no description)".to_string());

            let definition = AgentDefinition {
                id: name.clone(),
                name: name.clone(),
                description: desc,
                mode: AgentMode::General,
                trust_tier: TrustTier::UserDefined,
                capabilities: vec![],
                allowed_tools: vec![],
                denied_tools: vec![],
                system_prompt: String::new(),
                skills: vec![],
                preferred_models: vec![],
                fallback_models: vec![],
                temperature: None,
                top_p: None,
                max_iterations: 20,
                max_tool_calls: 50,
                timeout_secs: 300,
                context_sharing: ContextStrategy::None,
                max_context_tokens: 4096,
            };

            let mut registry = services.agent_registry.lock().await;
            match registry.register(definition) {
                Ok(()) => {
                    output::print_success(&format!("Agent '{name}' defined"));
                }
                Err(e) => {
                    output::print_error(&format!("Failed to define agent: {e}"));
                }
            }
        }
        AgentAction::Delegate { agent, task } => {
            // Verify agent exists in registry.
            let registry = services.agent_registry.lock().await;
            match registry.get(agent) {
                Some(_def) => {
                    output::print_info(&format!("Delegating to '{agent}': {task}"));
                    output::print_warning(
                        "Delegation execution requires orchestrator integration (future work)",
                    );
                }
                None => {
                    output::print_error(&format!("Agent not found: {agent}"));
                }
            }
        }
    }

    Ok(())
}
