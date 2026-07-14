//! Command dispatch — defines all CLI subcommands and routes to handlers.

pub mod agent;
pub mod chat;
pub mod common;
pub mod completion;
pub mod config_cmd;
pub mod diag;
pub mod init;
pub mod kb;
pub mod mcp;
pub mod observe;
pub mod print;
pub mod provider;
pub mod rewind;
pub mod rpc;
pub mod serve;
pub mod session;
pub mod skills;
pub mod status;
pub mod tool;
#[cfg(feature = "tui")]
pub mod tui_cmd;
pub mod workflow;
pub mod workspace;

use clap::Subcommand;

/// All CLI subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start an interactive chat session.
    Chat {
        /// Session ID to resume (optional).
        #[arg(long)]
        session: Option<String>,

        /// Agent name to use.
        #[arg(long, default_value = "default")]
        agent: String,

        /// Initial prompt (one or more words). After sending the first message,
        /// the REPL continues if stdin is a TTY; otherwise the process exits.
        /// Bare `y-agent "do X"` is forwarded here.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },

    /// Initialize a new y-agent project.
    Init(init::InitArgs),

    /// Show system status.
    Status,

    /// Configuration management.
    Config {
        #[command(subcommand)]
        action: config_cmd::ConfigAction,
    },

    /// Session management.
    Session {
        #[command(subcommand)]
        action: session::SessionAction,
    },

    /// Tool management.
    Tool {
        #[command(subcommand)]
        action: tool::ToolAction,
    },

    /// Agent management.
    Agent {
        #[command(subcommand)]
        action: agent::AgentAction,
    },

    /// Workflow management.
    Workflow {
        #[command(subcommand)]
        action: workflow::WorkflowAction,
    },

    /// Diagnostics and observability.
    Diag {
        #[command(subcommand)]
        action: diag::DiagAction,
    },

    /// Skill management.
    Skill {
        #[command(subcommand)]
        action: skills::SkillAction,
    },

    /// Knowledge base management.
    Kb {
        #[command(subcommand)]
        action: kb::KbAction,
    },

    /// MCP server management (add / remove / list / status).
    Mcp {
        #[command(subcommand)]
        action: mcp::McpAction,
    },

    /// Generate shell completions.
    Completion(completion::CompletionArgs),
    /// Single-shot prompt: send one message, print the response, exit.
    /// Useful for scripting (`y-agent print "summarize this"`).
    Print {
        /// Output mode: `text` (final response only) or `json` (structured result).
        #[arg(long, default_value = "text")]
        mode: String,

        /// Session ID to resume for this single turn (optional).
        #[arg(long)]
        session: Option<String>,

        /// Agent name to use.
        #[arg(long, default_value = "default")]
        agent: String,

        /// The prompt. Use `--` before prompts starting with `-`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        prompt: Vec<String>,
    },

    /// Headless JSONL stdio protocol for embedding in other processes.
    /// Reads JSON commands from stdin, writes JSON events/responses to stdout.
    Rpc,

    /// Launch the TUI interface.
    #[cfg(feature = "tui")]
    Tui {
        /// Session ID (or prefix) to resume.
        #[arg(long)]
        session: Option<String>,
    },

    /// Start the Web API server.
    Serve(serve::ServeArgs),

    /// Resume a previous session in TUI mode.
    Resume {
        /// Session ID (or prefix) to resume. Uses the most recent if omitted.
        session: Option<String>,
    },

    /// Fork a previous session into a new session in TUI mode.
    Fork {
        /// Session ID (or prefix) to fork. Uses the most recent if omitted.
        session: Option<String>,

        /// Label for the forked session.
        #[arg(long)]
        label: Option<String>,
    },

    /// Workspace management.
    Workspace {
        #[command(subcommand)]
        action: workspace::WorkspaceAction,
    },

    /// Provider management.
    Provider {
        #[command(subcommand)]
        action: provider::ProviderAction,
    },

    /// Observability and live system state.
    Observe {
        #[command(subcommand)]
        action: observe::ObserveAction,
    },

    /// Rewind (file history rollback).
    Rewind {
        #[command(subcommand)]
        action: rewind::RewindAction,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[derive(Parser)]
    #[command(name = "y-agent")]
    struct TestCli {
        #[command(subcommand)]
        command: Option<super::Commands>,
    }

    // T-CLI-003-01: test_parse_chat_command
    #[test]
    fn test_parse_chat_command() {
        let cli = TestCli::parse_from(["y-agent", "chat"]);
        assert!(matches!(cli.command, Some(super::Commands::Chat { .. })));
    }

    // T-CLI-003-02: test_parse_status_command
    #[test]
    fn test_parse_status_command() {
        let cli = TestCli::parse_from(["y-agent", "status"]);
        assert!(matches!(cli.command, Some(super::Commands::Status)));
    }

    // T-CLI-003-03: test_parse_session_list
    #[test]
    fn test_parse_session_list() {
        let cli = TestCli::parse_from(["y-agent", "session", "list"]);
        assert!(matches!(
            cli.command,
            Some(super::Commands::Session {
                action: super::session::SessionAction::List
            })
        ));
    }

    // T-CLI-003-04: bare unknown tokens fail at clap parse level; the
    // bare-prompt resolver in main.rs (not clap) forwards them to `chat`.
    #[test]
    fn test_parse_unknown_command() {
        let result = TestCli::try_parse_from(["y-agent", "foobar"]);
        assert!(result.is_err(), "unknown command should fail at clap level");
    }

    // T-CLI-003-05: test_parse_init_command
    #[test]
    fn test_parse_init_command() {
        let cli = TestCli::parse_from(["y-agent", "init"]);
        assert!(matches!(cli.command, Some(super::Commands::Init(..))));
    }

    // T-CLI-003-06: test_parse_init_with_provider
    #[test]
    fn test_parse_init_with_provider() {
        let cli = TestCli::parse_from([
            "y-agent",
            "init",
            "--provider",
            "openai",
            "--non-interactive",
        ]);
        match cli.command {
            Some(super::Commands::Init(args)) => {
                assert_eq!(args.provider, Some("openai".to_string()));
                assert!(args.non_interactive);
            }
            _ => panic!("expected Init command"),
        }
    }

    // T-CLI-003-07: test_parse_init_invalid_provider
    #[test]
    fn test_parse_init_invalid_provider() {
        let result = TestCli::try_parse_from(["y-agent", "init", "--provider", "invalid-provider"]);
        assert!(result.is_err(), "invalid provider should fail");
    }
}
