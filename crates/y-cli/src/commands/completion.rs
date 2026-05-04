//! Shell completion generation.

use std::io::Write;

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
    generate_completion_to(args, &mut std::io::stdout());
}

/// Generate shell completions and write them to the provided writer.
pub fn generate_completion_to<W>(args: &CompletionArgs, writer: &mut W)
where
    W: Write,
{
    let mut cmd = Cli::command();
    let writer: &mut dyn Write = writer;
    generate(args.shell, &mut cmd, "y-agent", writer);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completion_output(shell: Shell) -> String {
        let args = CompletionArgs { shell };
        let mut output = Vec::new();
        generate_completion_to(&args, &mut output);
        String::from_utf8(output).expect("completion output should be valid UTF-8")
    }

    // T-CLI-COMP-01: Bash completions are captured by the provided writer.
    #[test]
    fn test_bash_completion_generates() {
        let output = completion_output(Shell::Bash);

        assert!(output.contains("_y-agent"));
        assert!(output.contains("complete -F"));
    }

    // T-CLI-COMP-02: Zsh completions are captured by the provided writer.
    #[test]
    fn test_zsh_completion_generates() {
        let output = completion_output(Shell::Zsh);

        assert!(output.contains("#compdef y-agent"));
        assert!(output.contains("_y-agent"));
    }

    // T-CLI-COMP-03: Fish completions are captured by the provided writer.
    #[test]
    fn test_fish_completion_generates() {
        let output = completion_output(Shell::Fish);

        assert!(output.contains("complete -c y-agent"));
        assert!(output.contains("__fish_y_agent"));
    }
}
