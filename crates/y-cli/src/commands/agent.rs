//! Agent management commands.

use anyhow::Result;
use clap::Subcommand;

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;
use y_agent::agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
use y_agent::TrustTier;
use y_core::tool::ToolRegistry;
use y_service::SystemService;

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

    /// Reload all agents from disk.
    Reload,

    /// Reset an overridden built-in agent to its original definition.
    Reset {
        /// Agent ID.
        id: String,
    },

    /// Save an agent definition from a TOML file.
    Save {
        /// Agent ID.
        id: String,

        /// Path to a TOML file containing the agent definition.
        file: String,
    },

    /// Show full detail for a single agent.
    Show {
        /// Agent ID.
        id: String,
    },

    /// Show the raw TOML source for an agent.
    Source {
        /// Agent ID.
        id: String,
    },

    /// List all tools available for agent configuration.
    Tools,
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
                icon: None,
                working_directory: None,
                workspace_isolation: y_core::agent::WorkspaceIsolationPreference::default(),
                toolcall_enabled: None,
                skills_enabled: None,
                knowledge_enabled: None,
                allowed_tools: vec![],
                system_prompt: String::new(),
                skills: vec![],
                knowledge_collections: vec![],
                prompt_section_ids: vec![],
                provider_id: None,
                preferred_models: vec![],
                fallback_models: vec![],
                provider_tags: vec![],
                fallback_provider_tags: vec![],
                temperature: None,
                top_p: None,
                plan_mode: None,
                thinking_effort: None,
                permission_mode: None,
                max_iterations: 20,
                max_tool_calls: 50,
                timeout_secs: 300,
                context_sharing: ContextStrategy::None,
                max_context_tokens: 4096,
                max_completion_tokens: None,
                user_callable: false,
                mcp_mode: None,
                mcp_servers: vec![],
                prune_tool_history: false,
                auto_update: true,
                response_format: None,
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
        AgentAction::Reload => {
            let (loaded, errored) = SystemService::reload_agents(services).await;
            if errored > 0 {
                output::print_warning(&format!("Reloaded {loaded} agents with {errored} errors"));
            } else {
                output::print_success(&format!("Reloaded {loaded} agents"));
            }
        }
        AgentAction::Reset { id } => match services.reset_agent(id).await {
            Ok(()) => output::print_success(&format!("Agent '{id}' reset to default")),
            Err(e) => output::print_error(&format!("Failed to reset agent: {e}")),
        },
        AgentAction::Save { id, file } => {
            let toml_content = std::fs::read_to_string(file)
                .map_err(|e| anyhow::anyhow!("Failed to read file '{file}': {e}"))?;
            match services.save_agent(id, &toml_content).await {
                Ok(()) => output::print_success(&format!("Agent '{id}' saved")),
                Err(e) => output::print_error(&format!("Failed to save agent: {e}")),
            }
        }
        AgentAction::Show { id } => {
            let registry = services.agent_registry.lock().await;
            match registry.get(id) {
                Some(def) => {
                    if mode == OutputMode::Json {
                        let json = serde_json::to_string_pretty(def)?;
                        println!("{json}");
                    } else {
                        output::print_status("ID", &def.id);
                        output::print_status("Name", &def.name);
                        output::print_status("Description", &def.description);
                        output::print_status("Mode", &format!("{:?}", def.mode));
                        output::print_status("Trust", &format!("{:?}", def.trust_tier));
                        output::print_status("Max Iterations", &def.max_iterations.to_string());
                        output::print_status("Max Tool Calls", &def.max_tool_calls.to_string());
                        if !def.allowed_tools.is_empty() {
                            output::print_status("Allowed Tools", &def.allowed_tools.join(", "));
                        }
                        if !def.skills.is_empty() {
                            output::print_status("Skills", &def.skills.join(", "));
                        }
                        if !def.knowledge_collections.is_empty() {
                            output::print_status(
                                "Knowledge",
                                &def.knowledge_collections.join(", "),
                            );
                        }
                    }
                }
                None => {
                    output::print_error(&format!("Agent not found: {id}"));
                }
            }
        }
        AgentAction::Source { id } => match services.get_agent_source(id).await {
            Ok((path, content, is_user_file)) => {
                if mode == OutputMode::Json {
                    let json = serde_json::json!({
                        "path": path,
                        "content": content,
                        "is_user_file": is_user_file,
                    });
                    println!("{json}");
                } else {
                    output::print_info(&format!(
                        "Source: {path} ({})",
                        if is_user_file { "user" } else { "builtin" }
                    ));
                    println!();
                    println!("{content}");
                }
            }
            Err(e) => {
                output::print_error(&format!("Failed to get agent source: {e}"));
            }
        },
        AgentAction::Tools => {
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
    }

    Ok(())
}
