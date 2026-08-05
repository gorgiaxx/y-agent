mod guard;

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "y-app", about = "Build and run y-agent presentation targets")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build the shared frontend, CLI, and GUI binaries together.
    Build(BuildArgs),
    /// Run the CLI binary and forward all remaining arguments.
    Cli(RunCliArgs),
    /// Build the shared frontend and run the desktop GUI binary.
    Gui(RunGuiArgs),
    /// Verify architecture and quality guards.
    #[command(subcommand)]
    Guard(guard::GuardCommand),
}

#[derive(Debug, Args)]
struct BuildArgs {
    /// Build optimized release binaries.
    #[arg(long)]
    release: bool,
    /// Require Cargo.lock to remain unchanged.
    #[arg(long)]
    locked: bool,
}

#[derive(Debug, Args)]
struct RunCliArgs {
    /// Arguments forwarded to the y-agent CLI.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct RunGuiArgs {
    /// Run an optimized release build.
    #[arg(long)]
    release: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct CommandStep {
    program: String,
    args: Vec<String>,
    current_dir: StepDirectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepDirectory {
    Repository,
    Frontend,
}

fn command_plan(command: &Command) -> Vec<CommandStep> {
    match command {
        Command::Build(args) => {
            let mut cargo_args = vec!["build", "--workspace", "--bins"];
            if args.release {
                cargo_args.push("--release");
                cargo_args.extend(["--features", "y-gui/custom-protocol"]);
            }
            if args.locked {
                cargo_args.push("--locked");
            }
            vec![
                step("npm", ["run", "build"], StepDirectory::Frontend),
                step("cargo", cargo_args, StepDirectory::Repository),
            ]
        }
        Command::Cli(args) => {
            let mut cargo_args = vec!["run", "-p", "y-cli", "--bin", "y-agent", "--"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            cargo_args.extend(args.args.iter().cloned());
            vec![CommandStep {
                program: "cargo".to_string(),
                args: cargo_args,
                current_dir: StepDirectory::Repository,
            }]
        }
        Command::Gui(args) => {
            let mut cargo_args = vec!["run", "-p", "y-gui", "--bin", "y-gui"];
            if args.release {
                cargo_args.push("--release");
                cargo_args.extend(["--features", "y-gui/custom-protocol"]);
            }
            vec![
                step("npm", ["run", "build"], StepDirectory::Frontend),
                step("cargo", cargo_args, StepDirectory::Repository),
            ]
        }
        // Guards run in-process; see `main`.
        Command::Guard(_) => Vec::new(),
    }
}

fn step<I, S>(program: &str, args: I, current_dir: StepDirectory) -> CommandStep
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    CommandStep {
        program: program.to_string(),
        args: args.into_iter().map(Into::into).collect(),
        current_dir,
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("y-xtask must live under crates/y-xtask")
        .to_path_buf()
}

fn ensure_frontend_dependencies(frontend_dir: &Path) -> Result<()> {
    if frontend_dir.join("node_modules").is_dir() {
        return Ok(());
    }
    run_process("npm", &["ci"], frontend_dir)
        .context("failed to install shared frontend dependencies")
}

fn execute_plan(plan: &[CommandStep]) -> Result<()> {
    let root = repository_root();
    let frontend_dir = root.join("crates/y-gui");
    if plan
        .iter()
        .any(|step| step.current_dir == StepDirectory::Frontend)
    {
        ensure_frontend_dependencies(&frontend_dir)?;
    }

    for step in plan {
        let current_dir = match step.current_dir {
            StepDirectory::Repository => root.as_path(),
            StepDirectory::Frontend => frontend_dir.as_path(),
        };
        let args = step.args.iter().map(String::as_str).collect::<Vec<_>>();
        run_process(&step.program, &args, current_dir)?;
    }
    Ok(())
}

fn run_process(program: &str, args: &[&str], current_dir: &Path) -> Result<()> {
    let status = ProcessCommand::new(program)
        .args(args)
        .current_dir(current_dir)
        .status()
        .with_context(|| format!("failed to start {program}"))?;
    if !status.success() {
        bail!("{program} exited with status {status}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Guard(command) => guard::run(command, &repository_root()),
        other => execute_plan(&command_plan(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_plan_builds_frontend_before_workspace_binaries() {
        let command = Command::Build(BuildArgs {
            release: false,
            locked: false,
        });

        let plan = command_plan(&command);

        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].program, "npm");
        assert_eq!(plan[0].args, ["run", "build"]);
        assert_eq!(plan[1].program, "cargo");
        assert_eq!(plan[1].args, ["build", "--workspace", "--bins"]);
    }

    #[test]
    fn test_cli_plan_forwards_arguments_to_y_agent_binary() {
        let command = Command::Cli(RunCliArgs {
            args: vec![
                "status".to_string(),
                "--output".to_string(),
                "json".to_string(),
            ],
        });

        let plan = command_plan(&command);

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].program, "cargo");
        assert_eq!(
            plan[0].args,
            ["run", "-p", "y-cli", "--bin", "y-agent", "--", "status", "--output", "json",]
        );
    }

    #[test]
    fn test_gui_plan_builds_frontend_before_running_tauri_binary() {
        let command = Command::Gui(RunGuiArgs { release: true });

        let plan = command_plan(&command);

        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].program, "npm");
        assert_eq!(plan[0].args, ["run", "build"]);
        assert_eq!(
            plan[1].args,
            [
                "run",
                "-p",
                "y-gui",
                "--bin",
                "y-gui",
                "--release",
                "--features",
                "y-gui/custom-protocol",
            ]
        );
    }
}
