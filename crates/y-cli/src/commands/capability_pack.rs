//! Capability Pack lifecycle commands.

use std::path::Path;

use anyhow::{bail, Result};
use clap::Subcommand;
use y_service::capability_pack::{
    CapabilityPackInstallOptions, CapabilityPackService, InstalledCapabilityPackSummary,
};

use crate::output::{self, OutputMode, TableRow};
use crate::wire::AppServices;

/// Capability Pack subcommands.
#[derive(Debug, Subcommand)]
pub enum CapabilityPackAction {
    /// List installed packs and their active resources.
    List,
    /// Validate and preview a local pack without installing it.
    Inspect {
        /// Capability Pack root directory.
        path: String,
    },
    /// Install a validated local pack.
    Install {
        /// Capability Pack root directory.
        path: String,
        /// Explicitly allow replacement of resources owned outside the pack.
        #[arg(long)]
        allow_replacements: bool,
    },
    /// Roll back the latest installed version of a pack.
    Rollback {
        /// Installed pack ID.
        pack_id: String,
        /// Confirm the destructive lifecycle action.
        #[arg(long)]
        yes: bool,
    },
    /// Remove every installed version of a pack.
    Remove {
        /// Installed pack ID.
        pack_id: String,
        /// Confirm the destructive lifecycle action.
        #[arg(long)]
        yes: bool,
    },
}

/// Run a Capability Pack subcommand.
pub async fn run(
    action: &CapabilityPackAction,
    services: &AppServices,
    mode: OutputMode,
) -> Result<()> {
    match action {
        CapabilityPackAction::List => {
            let packs = CapabilityPackService::list_installed(services).await?;
            print_pack_list(&packs, mode);
        }
        CapabilityPackAction::Inspect { path } => {
            let inspection =
                CapabilityPackService::inspect_local(services, Path::new(path)).await?;
            println!("{}", output::format_value(&inspection, mode));
        }
        CapabilityPackAction::Install {
            path,
            allow_replacements,
        } => {
            let receipt = CapabilityPackService::install_local(
                services,
                Path::new(path),
                CapabilityPackInstallOptions {
                    allow_replacements: *allow_replacements,
                },
            )
            .await?;
            println!("{}", output::format_value(&receipt, mode));
        }
        CapabilityPackAction::Rollback { pack_id, yes } => {
            require_confirmation("rollback", *yes)?;
            let receipt = CapabilityPackService::rollback(services, pack_id).await?;
            println!("{}", output::format_value(&receipt, mode));
        }
        CapabilityPackAction::Remove { pack_id, yes } => {
            require_confirmation("remove", *yes)?;
            let receipt = CapabilityPackService::remove(services, pack_id).await?;
            println!("{}", output::format_value(&receipt, mode));
        }
    }
    Ok(())
}

fn require_confirmation(action: &str, confirmed: bool) -> Result<()> {
    if !confirmed {
        bail!("capability-pack {action} requires --yes");
    }
    Ok(())
}

fn print_pack_list(packs: &[InstalledCapabilityPackSummary], mode: OutputMode) {
    if mode == OutputMode::Json {
        println!("{}", output::format_value(&packs, mode));
        return;
    }
    if packs.is_empty() {
        output::print_info("No Capability Packs installed");
        return;
    }
    let rows = packs
        .iter()
        .map(|pack| TableRow {
            cells: vec![
                pack.pack_id.clone(),
                pack.current_version.clone(),
                pack.resources.len().to_string(),
                pack.live_resources.len().to_string(),
            ],
        })
        .collect::<Vec<_>>();
    print!(
        "{}",
        output::format_table(&["PACK", "VERSION", "RESOURCES", "LIVE"], &rows)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destructive_pack_actions_require_explicit_confirmation() {
        let error = require_confirmation("remove", false).unwrap_err();

        assert!(error.to_string().contains("requires --yes"));
    }

    #[test]
    fn test_confirmed_pack_action_is_allowed() {
        assert!(require_confirmation("rollback", true).is_ok());
    }
}
