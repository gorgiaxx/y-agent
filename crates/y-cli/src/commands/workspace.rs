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
//! - `trust` / `untrust` / `trust-status` -- manage project config activation

use std::path::Path;

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

    /// Trust project-origin configuration from a workspace directory.
    Trust {
        /// Workspace directory to trust.
        #[arg(long, default_value = ".")]
        path: String,
    },

    /// Explicitly block project-origin configuration from a workspace directory.
    Untrust {
        /// Workspace directory to block.
        #[arg(long, default_value = ".")]
        path: String,
    },

    /// Show the trust decision for a workspace directory.
    TrustStatus {
        /// Workspace directory to inspect.
        #[arg(long, default_value = ".")]
        path: String,
    },
}

/// Build a [`WorkspaceService`] rooted at the user config directory.
fn svc(config_dir: &Path) -> WorkspaceService {
    WorkspaceService::new(config_dir)
}

/// Run a workspace subcommand.
pub fn run(
    action: &WorkspaceAction,
    mode: OutputMode,
    user_config_dir: Option<&Path>,
) -> Result<()> {
    let default_config_dir;
    let config_dir = if let Some(config_dir) = user_config_dir {
        config_dir
    } else {
        default_config_dir = crate::config::dirs_user_config().expect("config dir");
        &default_config_dir
    };
    run_with_config_dir(action, mode, config_dir)
}

fn run_with_config_dir(
    action: &WorkspaceAction,
    mode: OutputMode,
    config_dir: &Path,
) -> Result<()> {
    let service = svc(config_dir);
    match action {
        WorkspaceAction::List => {
            let workspaces = service.list();
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
            let record: WorkspaceRecord = service.create(name.clone(), path.clone())?;
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
            service.update(id, name.clone(), path.clone())?;
            output::print_success(&format!("Updated workspace {id}"));
        }
        WorkspaceAction::Delete { id } => {
            service.delete(id)?;
            output::print_success(&format!("Deleted workspace {id}"));
        }
        WorkspaceAction::Assign {
            workspace_id,
            session_id,
        } => {
            service.assign_session(workspace_id.clone(), session_id.clone())?;
            output::print_success(&format!(
                "Assigned session {session_id} to workspace {workspace_id}"
            ));
        }
        WorkspaceAction::Unassign { session_id } => {
            service.unassign_session(session_id)?;
            output::print_success(&format!("Unassigned session {session_id}"));
        }
        WorkspaceAction::Map => {
            let map = service.session_map();
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
        WorkspaceAction::Trust { path } => {
            let decision = service.trust_workspace(Path::new(path))?;
            match mode {
                OutputMode::Json => println!("{}", output::format_value(&decision, mode)),
                OutputMode::Table | OutputMode::Plain => {
                    output::print_success(&format!(
                        "Trusted workspace {}",
                        decision.canonical_path
                    ));
                }
            }
        }
        WorkspaceAction::Untrust { path } => {
            let decision = service.untrust_workspace(Path::new(path))?;
            match mode {
                OutputMode::Json => println!("{}", output::format_value(&decision, mode)),
                OutputMode::Table | OutputMode::Plain => {
                    output::print_success(&format!(
                        "Blocked workspace {}",
                        decision.canonical_path
                    ));
                }
            }
        }
        WorkspaceAction::TrustStatus { path } => {
            let decision = service.workspace_trust(Path::new(path))?;
            match mode {
                OutputMode::Json => println!("{}", output::format_value(&decision, mode)),
                OutputMode::Table | OutputMode::Plain => output::print_info(&format!(
                    "Workspace {} is {:?}",
                    decision.canonical_path, decision.status
                )),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_service::WorkspaceTrustStatus;

    #[test]
    fn trust_commands_persist_decisions_in_selected_user_config_dir() {
        let config_dir = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        run_with_config_dir(
            &WorkspaceAction::Trust {
                path: project_dir.path().to_string_lossy().into_owned(),
            },
            OutputMode::Plain,
            config_dir.path(),
        )
        .unwrap();
        assert_eq!(
            WorkspaceService::new(config_dir.path())
                .workspace_trust(project_dir.path())
                .unwrap()
                .status,
            WorkspaceTrustStatus::Trusted
        );

        run_with_config_dir(
            &WorkspaceAction::Untrust {
                path: project_dir.path().to_string_lossy().into_owned(),
            },
            OutputMode::Plain,
            config_dir.path(),
        )
        .unwrap();
        assert_eq!(
            WorkspaceService::new(config_dir.path())
                .workspace_trust(project_dir.path())
                .unwrap()
                .status,
            WorkspaceTrustStatus::Untrusted
        );
    }
}
