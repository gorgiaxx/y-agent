//! `workspace` CLI command -- workspace and session-workspace mapping management.
//!
//! Subcommands:
//! - `list` -- list all workspaces
//! - `create` -- create a workspace
//! - `update` -- update a workspace
//! - `delete` -- delete a workspace
//! - `assign` -- assign a session to a workspace
//! - `unassign` -- unassign a session from its workspace
//! - `map` -- show the full session->workspace map

use anyhow::Result;
use clap::Subcommand;

use y_service::{WorkspaceRecord, WorkspaceService};

use crate::output::{self, OutputMode, TableRow};

/// Workspace subcommands.
#[derive(Debug, Subcommand)]
pub enum WorkspaceAction {
    /// List all workspaces.
    List,

    /// Create a new workspace.
    Create {
        /// Workspace name.
        #[arg(long)]
        name: String,

        /// Workspace path (filesystem directory).
        #[arg(long)]
        path: String,
    },

    /// Update an existing workspace's name and/or path.
    Update {
        /// Workspace ID.
        id: String,

        /// New workspace name.
        #[arg(long)]
        name: String,

        /// New workspace path.
        #[arg(long)]
        path: String,
    },

    /// Delete a workspace and remove all its session assignments.
    Delete {
        /// Workspace ID.
        id: String,
    },

    /// Assign a session to a workspace (overwrites any previous assignment).
    Assign {
        /// Target workspace ID.
        #[arg(long)]
        workspace_id: String,

        /// Session ID to assign.
        #[arg(long)]
        session_id: String,
    },

    /// Remove a session's workspace assignment.
    Unassign {
        /// Session ID to unassign.
        #[arg(long)]
        session_id: String,
    },

    /// Show the full session->workspace map.
    Map,
}

/// Build a [`WorkspaceService`] rooted at the user config directory.
fn svc() -> WorkspaceService {
    let config_dir = crate::config::dirs_user_config().expect("config dir");
    WorkspaceService::new(&config_dir)
}

/// Run a workspace subcommand.
pub fn run(action: &WorkspaceAction, mode: OutputMode) -> Result<()> {
    match action {
        WorkspaceAction::List => {
            let workspaces = svc().list();
            match mode {
                OutputMode::Json => {
                    println!("{}", output::format_value(&workspaces, mode));
                }
                OutputMode::Table | OutputMode::Plain => {
                    let headers = &["ID", "Name", "Path"];
                    let rows: Vec<TableRow> = workspaces
                        .iter()
                        .map(|w| TableRow {
                            cells: vec![w.id.clone(), w.name.clone(), w.path.clone()],
                        })
                        .collect();
                    output::print_info(&format!("{} workspace(s):", rows.len()));
                    let table = output::format_table(headers, &rows);
                    print!("{table}");
                }
            }
        }
        WorkspaceAction::Create { name, path } => {
            let record: WorkspaceRecord = svc().create(name.clone(), path.clone())?;
            match mode {
                OutputMode::Json => {
                    println!("{}", output::format_value(&record, mode));
                }
                OutputMode::Table | OutputMode::Plain => {
                    output::print_success(&format!(
                        "Created workspace {} ({}) at {}",
                        record.id, record.name, record.path
                    ));
                }
            }
        }
        WorkspaceAction::Update { id, name, path } => {
            svc().update(id, name.clone(), path.clone())?;
            output::print_success(&format!("Updated workspace {id}"));
        }
        WorkspaceAction::Delete { id } => {
            svc().delete(id)?;
            output::print_success(&format!("Deleted workspace {id}"));
        }
        WorkspaceAction::Assign {
            workspace_id,
            session_id,
        } => {
            svc().assign_session(workspace_id.clone(), session_id.clone())?;
            output::print_success(&format!(
                "Assigned session {session_id} to workspace {workspace_id}"
            ));
        }
        WorkspaceAction::Unassign { session_id } => {
            svc().unassign_session(session_id)?;
            output::print_success(&format!("Unassigned session {session_id}"));
        }
        WorkspaceAction::Map => {
            let map = svc().session_map();
            match mode {
                OutputMode::Json => {
                    println!("{}", output::format_value(&map, mode));
                }
                OutputMode::Table | OutputMode::Plain => {
                    let headers = &["Session ID", "Workspace ID"];
                    let mut rows: Vec<TableRow> = map
                        .iter()
                        .map(|(sid, wid)| TableRow {
                            cells: vec![sid.clone(), wid.clone()],
                        })
                        .collect();
                    rows.sort_by(|a, b| a.cells[0].cmp(&b.cells[0]));
                    output::print_info(&format!("{} assignment(s):", rows.len()));
                    let table = output::format_table(headers, &rows);
                    print!("{table}");
                }
            }
        }
    }

    Ok(())
}
