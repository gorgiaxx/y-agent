//! Shell completion generation.

use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::Cli;

/// Arguments for the completion subcommand.
#[derive(Debug, clap::Parser)]
pub struct CompletionArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    pub shell: Shell,
}

/// Generate shell completions and write them to stdout.
pub fn run(args: &CompletionArgs) {
    let mut cmd = Cli::command();
    generate(args.shell, &mut cmd, "y-agent", &mut std::io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-COMP-01: Bash completions generate without panic.
    #[test]
    fn test_bash_completion_generates() {
        let args = CompletionArgs { shell: Shell::Bash };
        // Should not panic.
        run(&args);
    }

    // T-CLI-COMP-02: Zsh completions generate without panic.
    #[test]
    fn test_zsh_completion_generates() {
        let args = CompletionArgs { shell: Shell::Zsh };
        run(&args);
    }

    // T-CLI-COMP-03: Fish completions generate without panic.
    #[test]
    fn test_fish_completion_generates() {
        let args = CompletionArgs { shell: Shell::Fish };
        run(&args);
    }
}
