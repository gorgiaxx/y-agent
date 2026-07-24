//! Command handlers: execute commands against `AppState` and services.
//!
//! Each handler receives a parsed command line, the mutable `AppState`,
//! and returns a `CommandResult` indicating success, failure, or output.
//!
//! Commands that require async service access return `CommandResult::Async`
//! with an `AsyncCommand` variant that the TUI event loop executes.

use std::fmt::Write as _;

use crate::tui::commands::copy::{self, CopyTarget};
use crate::tui::state::{AppState, ChatMessage, TurnMode};

/// Result of executing a command.
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// Command succeeded with an optional message to display.
    Ok(Option<String>),
    /// Command failed with an error message.
    Error(String),
    /// Quit the application.
    Quit,
    /// A new session was requested -- state has been reset.
    /// The TUI event loop should handle any async follow-up.
    NewSession,
    /// Command requires async service access -- deferred to TUI event loop.
    Async(AsyncCommand),
    /// Submit a chat turn using the requested orchestration mode.
    SubmitTurn { input: String, mode: TurnMode },
    /// Change the orchestration mode used by subsequent normal messages.
    SetTurnMode(TurnMode),
    /// Copy selected conversation content to the system clipboard.
    Copy(CopyTarget),
}

/// Deferred async commands that require `AppServices` access.
///
/// The TUI event loop matches on these to perform the real work
/// with access to `SessionManager`, `ProviderPool`, etc.
#[derive(Debug, Clone)]
pub enum AsyncCommand {
    /// `/list` -- list all sessions.
    ListSessions,
    /// `/switch <id|label>` -- switch to another session.
    SwitchSession(String),
    /// `/resume [id|title]` -- open the session picker or resume directly.
    ResumeSession(Option<String>),
    /// `/delete <id>` -- delete a session.
    DeleteSession(String),
    /// `/branch [label]` -- fork session from current point.
    BranchSession(Option<String>),
    /// `/export [format]` -- export session transcript.
    ExportSession(Option<String>),
    /// `/stats` -- show token/cost statistics.
    ShowStats,
    /// `/compact` -- trigger manual context compaction.
    CompactContext,
    /// `/model [provider-id]` -- list models or select a specific provider.
    ModelCommand(Option<String>),
    /// `/agent [subcommand]` -- agent management.
    ShowAgents,
}

/// Parse and execute a command string.
///
/// The input is the raw text after the `/` prefix, e.g. `"new my session"`.
/// Commands that need async service access return `CommandResult::Async`.
pub fn execute(input: &str, state: &mut AppState) -> CommandResult {
    let parts: Vec<&str> = input.trim().splitn(2, ' ').collect();
    let cmd_name = parts.first().copied().unwrap_or("");
    let args = parts.get(1).copied().unwrap_or("");

    // Resolve alias via registry.
    let resolved = crate::tui::commands::registry::CommandRegistry::new()
        .resolve_alias(cmd_name)
        .to_string();

    match resolved.as_str() {
        "quit" | "exit" => CommandResult::Quit,

        "clear" => {
            state.messages.clear();
            state.scroll_offset = 0;
            CommandResult::Ok(Some("Chat cleared.".into()))
        }

        "help" => {
            let help_text = if args.is_empty() {
                generate_help_text()
            } else {
                generate_command_help(args)
            };
            // Display help as a system message.
            state.messages.push(ChatMessage::system(help_text));
            CommandResult::Ok(None)
        }

        "new" => {
            // Reset chat state for a fresh session.
            // Actual DB session creation is deferred to first message (lazy).
            state.messages.clear();
            state.scroll_offset = 0;
            state.current_session_id = None;
            state.user_message_count = 0;
            CommandResult::NewSession
        }

        "reset" => {
            state.messages.clear();
            state.scroll_offset = 0;
            CommandResult::Ok(Some("Session reset.".into()))
        }

        "status" => {
            let msg = format!(
                "Messages: {} | Streaming: {} | Turn mode: {} | UI mode: {:?} | Focus: {:?}",
                state.messages.len(),
                state.is_streaming,
                state.turn_mode.label(),
                state.mode,
                state.focus,
            );
            state.messages.push(ChatMessage::system(msg));
            CommandResult::Ok(None)
        }

        "debug" => {
            let msg = match args {
                "--on" | "on" => "Debug mode enabled.".to_string(),
                "--off" | "off" => "Debug mode disabled.".to_string(),
                _ => "Usage: /debug [--on|--off]".to_string(),
            };
            CommandResult::Ok(Some(msg))
        }

        // Async commands -- delegate to TUI event loop with service access.
        "list" => CommandResult::Async(AsyncCommand::ListSessions),

        "switch" => {
            if args.is_empty() {
                CommandResult::Error("Usage: /switch <session-id|label>".into())
            } else {
                CommandResult::Async(AsyncCommand::SwitchSession(args.to_string()))
            }
        }

        "resume" => {
            let target = if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            };
            CommandResult::Async(AsyncCommand::ResumeSession(target))
        }

        "goal" => {
            if args.is_empty() {
                CommandResult::Error("Usage: /goal <objective>".into())
            } else {
                CommandResult::SubmitTurn {
                    input: args.to_string(),
                    mode: TurnMode::Auto,
                }
            }
        }

        "mode" => {
            if args.is_empty() {
                CommandResult::Ok(Some(format!(
                    "Turn mode: {}. Use /mode fast|auto|plan|loop.",
                    state.turn_mode.label()
                )))
            } else {
                TurnMode::parse(args).map_or_else(
                    || CommandResult::Error("Usage: /mode fast|auto|plan|loop".into()),
                    CommandResult::SetTurnMode,
                )
            }
        }

        "fast" => mode_command(TurnMode::Fast, args),
        "auto" => mode_command(TurnMode::Auto, args),
        "plan" => mode_command(TurnMode::Plan, args),
        "loop" => mode_command(TurnMode::Loop, args),

        "delete" => {
            if args.is_empty() {
                CommandResult::Error("Usage: /delete <session-id>".into())
            } else {
                CommandResult::Async(AsyncCommand::DeleteSession(args.to_string()))
            }
        }

        "branch" => {
            let label = if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            };
            CommandResult::Async(AsyncCommand::BranchSession(label))
        }

        "export" => {
            let format = if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            };
            CommandResult::Async(AsyncCommand::ExportSession(format))
        }

        "stats" => CommandResult::Async(AsyncCommand::ShowStats),

        "compact" => CommandResult::Async(AsyncCommand::CompactContext),

        "model" => {
            let provider_arg = if args.is_empty() {
                None
            } else {
                Some(args.to_string())
            };
            CommandResult::Async(AsyncCommand::ModelCommand(provider_arg))
        }

        "agent" => CommandResult::Async(AsyncCommand::ShowAgents),

        "shortcuts" => {
            let text = generate_shortcuts_text();
            state.messages.push(ChatMessage::system(text));
            CommandResult::Ok(None)
        }

        "copy" => match copy::parse_target(args) {
            Ok(target) => CommandResult::Copy(target),
            Err(message) => CommandResult::Error(message),
        },

        _ => CommandResult::Error(format!(
            "Unknown command: /{cmd_name}. Type /help for a list."
        )),
    }
}

fn mode_command(mode: TurnMode, args: &str) -> CommandResult {
    if args.is_empty() {
        CommandResult::SetTurnMode(mode)
    } else {
        CommandResult::SubmitTurn {
            input: args.to_string(),
            mode,
        }
    }
}

/// Generate the full help text.
fn generate_help_text() -> String {
    let reg = crate::tui::commands::registry::CommandRegistry::new();
    let mut text = String::from("Available commands:\n\n");

    let mut current_category = None;
    for cmd in reg.all() {
        if current_category != Some(cmd.category) {
            current_category = Some(cmd.category);
            let _ = writeln!(text, "  [{}]", cmd.category.label());
        }
        let alias_str = cmd.alias.map(|a| format!(" (/{a})")).unwrap_or_default();
        let _ = writeln!(
            text,
            "    /{}{:<10} {}",
            cmd.name, alias_str, cmd.description
        );
    }

    text.push_str("\nPress Esc to dismiss. Type /help <command> for details.");
    text
}

/// Generate help for a specific command.
fn generate_command_help(cmd_name: &str) -> String {
    let reg = crate::tui::commands::registry::CommandRegistry::new();
    match reg.find(cmd_name) {
        Some(cmd) => {
            let alias_str = cmd
                .alias
                .map(|a| format!(" (alias: /{a})"))
                .unwrap_or_default();
            format!(
                "/{} {}\n{}{}\\n\nCategory: {}",
                cmd.name,
                cmd.args,
                cmd.description,
                alias_str,
                cmd.category.label()
            )
        }
        None => format!("Unknown command: /{cmd_name}"),
    }
}

/// Generate keyboard shortcuts reference text.
fn generate_shortcuts_text() -> String {
    let mut text = String::from("Keyboard Shortcuts:\n\n");

    text.push_str(
        "  [Global]
    Ctrl+Q / Ctrl+D / Ctrl+C  Quit
    Ctrl+H                    Show help\n\n",
    );

    text.push_str(
        "  [Input Panel]
    Enter                     Send message or queue follow-up while busy
    Shift+Enter               New line
    Tab                       Cycle focus (Input -> Chat)
    /                         Open command palette (on empty input)
    :                         Open command palette (vim-style)
    Esc                       Cancel active response / return to normal\n\n",
    );

    text.push_str(
        "  [Chat Panel]
    j / Down / PageDown       Scroll down
    k / Up / PageUp           Scroll up
    i                         Return focus to input
    Tab                       Cycle focus\n\n",
    );

    text.push_str(
        "  [Command Palette]
    Up / Down                 Navigate suggestions
    Tab                       Next suggestion
    Enter                     Execute selected command
    Esc                       Close palette\n\n",
    );

    text.push_str(
        "  [Mouse]
    Click                     Focus conversation or input
    Scroll wheel              Scroll chat history
    Shift + drag              Native text selection (terminal)
    /resume                   Pick a recent session
    /mode auto                Use automatic orchestration for later messages
    /plan <prompt>            Switch to plan mode and submit immediately
    /copy                     Copy latest assistant response
    /copy code                Copy latest fenced code block
    /copy transcript          Copy full transcript\n",
    );

    text
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::MessageRole;
    use chrono::Utc;

    // T-TUI-04-04: /clear clears messages.
    #[test]
    fn test_clear_command() {
        let mut state = AppState::default();
        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        });

        let result = execute("clear", &mut state);
        assert!(matches!(result, CommandResult::Ok(Some(ref msg)) if msg.contains("cleared")));
        assert!(state.messages.is_empty());
    }

    // T-TUI-04-05: /new resets state and returns NewSession.
    #[test]
    fn test_new_command() {
        let mut state = AppState::default();
        state.current_session_id = Some("old-session".into());
        state.user_message_count = 5;
        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".into(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        });

        let result = execute("new", &mut state);
        assert!(matches!(result, CommandResult::NewSession));
        assert!(state.messages.is_empty());
        assert!(state.current_session_id.is_none());
        assert_eq!(state.user_message_count, 0);
    }

    // T-TUI-04-06: unknown command returns error.
    #[test]
    fn test_unknown_command() {
        let mut state = AppState::default();
        let result = execute("foobar", &mut state);
        assert!(matches!(result, CommandResult::Error(ref msg) if msg.contains("Unknown")));
    }

    #[test]
    fn test_quit_command() {
        let mut state = AppState::default();
        let result = execute("quit", &mut state);
        assert!(matches!(result, CommandResult::Quit));
    }

    #[test]
    fn test_quit_alias() {
        let mut state = AppState::default();
        let result = execute("q", &mut state);
        assert!(matches!(result, CommandResult::Quit));
    }

    #[test]
    fn test_help_command() {
        let mut state = AppState::default();
        let result = execute("help", &mut state);
        assert!(matches!(result, CommandResult::Ok(None)));
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].content.contains("Available commands"));
    }

    #[test]
    fn test_status_command() {
        let mut state = AppState::default();
        let result = execute("status", &mut state);
        assert!(matches!(result, CommandResult::Ok(None)));
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].content.contains("Messages:"));
    }

    #[test]
    fn test_reset_command() {
        let mut state = AppState::default();
        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "msg".into(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        });
        let result = execute("reset", &mut state);
        assert!(matches!(result, CommandResult::Ok(Some(_))));
        assert!(state.messages.is_empty());
    }

    // T-TUI-04-07: async commands return Async variant.
    #[test]
    fn test_list_returns_async() {
        let mut state = AppState::default();
        let result = execute("list", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ListSessions)
        ));
    }

    #[test]
    fn test_switch_requires_args() {
        let mut state = AppState::default();
        let result = execute("switch", &mut state);
        assert!(matches!(result, CommandResult::Error(_)));

        let result = execute("switch my-session", &mut state);
        assert!(
            matches!(result, CommandResult::Async(AsyncCommand::SwitchSession(ref s)) if s == "my-session")
        );
    }

    #[test]
    fn test_delete_requires_args() {
        let mut state = AppState::default();
        let result = execute("delete", &mut state);
        assert!(matches!(result, CommandResult::Error(_)));

        let result = execute("delete abc-123", &mut state);
        assert!(
            matches!(result, CommandResult::Async(AsyncCommand::DeleteSession(ref s)) if s == "abc-123")
        );
    }

    #[test]
    fn test_branch_optional_label() {
        let mut state = AppState::default();
        let result = execute("branch", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::BranchSession(None))
        ));

        let result = execute("branch my-branch", &mut state);
        assert!(
            matches!(result, CommandResult::Async(AsyncCommand::BranchSession(Some(ref s))) if s == "my-branch")
        );
    }

    #[test]
    fn test_compact_returns_async() {
        let mut state = AppState::default();
        let result = execute("compact", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::CompactContext)
        ));
    }

    #[test]
    fn test_stats_returns_async() {
        let mut state = AppState::default();
        let result = execute("stats", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ShowStats)
        ));
    }

    #[test]
    fn test_model_no_args_returns_async_none() {
        let mut state = AppState::default();
        let result = execute("model", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ModelCommand(None))
        ));
    }

    #[test]
    fn test_model_with_args_returns_async_some() {
        let mut state = AppState::default();
        let result = execute("model deepseek", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ModelCommand(Some(ref id))) if id == "deepseek"
        ));
    }

    #[test]
    fn test_agent_returns_async() {
        let mut state = AppState::default();
        let result = execute("agent", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ShowAgents)
        ));
    }

    #[test]
    fn test_resume_supports_picker_and_direct_target() {
        let mut state = AppState::default();
        assert!(matches!(
            execute("resume", &mut state),
            CommandResult::Async(AsyncCommand::ResumeSession(None))
        ));
        assert!(matches!(
            execute("resume recent work", &mut state),
            CommandResult::Async(AsyncCommand::ResumeSession(Some(ref target)))
                if target == "recent work"
        ));
    }

    #[test]
    fn test_goal_submits_auto_orchestrated_turn() {
        let mut state = AppState::default();
        assert!(matches!(
            execute("goal finish the release", &mut state),
            CommandResult::SubmitTurn { ref input, mode: TurnMode::Auto }
                if input == "finish the release"
        ));
        assert!(matches!(
            execute("goal", &mut state),
            CommandResult::Error(ref message) if message.contains("/goal <objective>")
        ));
    }

    #[test]
    fn test_copy_parses_precise_targets() {
        let mut state = AppState::default();
        assert!(matches!(
            execute("copy", &mut state),
            CommandResult::Copy(super::copy::CopyTarget::AssistantResponse(1))
        ));
        assert!(matches!(
            execute("copy 3", &mut state),
            CommandResult::Copy(super::copy::CopyTarget::AssistantResponse(3))
        ));
        assert!(matches!(
            execute("copy code", &mut state),
            CommandResult::Copy(super::copy::CopyTarget::LastCodeBlock)
        ));
        assert!(matches!(
            execute("copy transcript", &mut state),
            CommandResult::Copy(super::copy::CopyTarget::Transcript)
        ));
    }

    #[test]
    fn test_mode_command_sets_persistent_turn_mode() {
        let mut state = AppState::default();
        assert!(matches!(
            execute("mode plan", &mut state),
            CommandResult::SetTurnMode(TurnMode::Plan)
        ));
        assert!(matches!(
            execute("mode invalid", &mut state),
            CommandResult::Error(ref message) if message.contains("fast|auto|plan|loop")
        ));
    }

    #[test]
    fn test_mode_shorthand_can_set_or_submit() {
        let mut state = AppState::default();
        assert!(matches!(
            execute("loop", &mut state),
            CommandResult::SetTurnMode(TurnMode::Loop)
        ));
        assert!(matches!(
            execute("plan inspect the repository", &mut state),
            CommandResult::SubmitTurn { ref input, mode: TurnMode::Plan }
                if input == "inspect the repository"
        ));
    }
}
