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
use y_core::permission_types::PermissionMode;

/// Refusal message for session-destroying commands issued while a response
/// streams. Shared with the TUI's async command paths (`/switch`, `/delete`,
/// Resume-overlay confirm) so every entry point refuses with the same text.
pub const STREAMING_ACTIVE_MESSAGE: &str =
    "A response is active. Wait for the current turn to finish or press Esc to cancel it.";

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
    /// Change the permission mode of the active session.
    ///
    /// Applied by the TUI event loop, which has `AppServices` access to the
    /// session permission map.
    SetPermissionMode(PermissionMode),
    /// Copy selected conversation content to the system clipboard.
    Copy(CopyTarget),
    /// Open the full-screen copy target selector.
    OpenCopyPicker,
    /// Open generated keyboard help backed by the active semantic keymap.
    OpenHelpOverlay,
    /// Open the follow-up queue overlay for the active run.
    OpenQueueOverlay,
    /// Add a TODO to the service-owned queue for the active run.
    QueueFollowUp(String),
    /// Open the background task and subagent overlay.
    OpenTasksOverlay,
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
    /// `/rename <id> <title>` -- set a manual session title.
    RenameSession { target: String, title: String },
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
    /// `/prompt [template-id|default]` -- select or apply a session prompt template.
    PromptTemplate(Option<String>),
    /// `/attach <path>` -- add a typed file to the composer.
    AttachFile(String),
}

/// Parse and execute a command string.
///
/// The input is the raw text after the `/` prefix, e.g. `"new my session"`.
/// Commands that need async service access return `CommandResult::Async`.
pub fn execute(input: &str, state: &mut AppState) -> CommandResult {
    let parts: Vec<&str> = input.trim().splitn(2, ' ').collect();
    let cmd_name = parts.first().copied().unwrap_or("");
    let args = parts.get(1).copied().unwrap_or("");

    // Resolve alias via the shared registry (built once per process).
    let resolved =
        crate::tui::commands::registry::CommandRegistry::shared().resolve_alias(cmd_name);

    match resolved {
        "quit" | "exit" => CommandResult::Quit,

        "clear" => {
            state.messages.clear();
            state.selected_tool = None;
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

        // Session-destroying commands are refused mid-turn: the active run
        // owns the session until it finishes or is cancelled (Esc).
        "new" | "reset" if state.is_streaming => {
            CommandResult::Error(STREAMING_ACTIVE_MESSAGE.into())
        }

        "new" => {
            // Reset chat state for a fresh session.
            // Actual DB session creation is deferred to first message (lazy).
            state.messages.clear();
            state.selected_tool = None;
            state.scroll_offset = 0;
            state.current_session_id = None;
            state.user_message_count = 0;
            state.prompt_template_status = crate::tui::state::PromptTemplateStatus::Default;
            state.follow_up_queue.clear();
            // Clear status-bar counters and model/cost/context derived from the
            // previous session so a brand-new session does not inherit stale
            // token usage, cost, or the prior provider's model name.
            state.status_model.clear();
            state.status_tokens.clear();
            state.cumulative_input_tokens = 0;
            state.cumulative_output_tokens = 0;
            state.last_input_tokens = 0;
            state.last_cost = None;
            // `context_window` is provider metadata, not per-session: leave it
            // so the usage bar still renders against the active provider limit.
            CommandResult::NewSession
        }

        "reset" => {
            state.messages.clear();
            state.selected_tool = None;
            state.scroll_offset = 0;
            CommandResult::Ok(Some("Session reset.".into()))
        }

        "status" => {
            let msg = format!(
                "Messages: {} | Streaming: {} | Turn mode: {} | {} | UI mode: {:?} | Focus: {:?}",
                state.messages.len(),
                state.is_streaming,
                state.turn_mode.label(),
                state.prompt_template_status.label(),
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

        "permission" => {
            let arg = args.trim();
            if arg.is_empty() {
                CommandResult::Ok(Some(
                    "Usage: /permission default|plan|accept_edits|bypass_permissions|dont_ask"
                        .into(),
                ))
            } else {
                parse_permission_mode(arg).map_or_else(
                    || {
                        CommandResult::Error(format!(
                            "Unknown permission mode '{arg}'. \
                             Valid: default, plan, accept_edits, bypass_permissions, dont_ask"
                        ))
                    },
                    CommandResult::SetPermissionMode,
                )
            }
        }

        "delete" => {
            if args.is_empty() {
                CommandResult::Error("Usage: /delete <session-id>".into())
            } else {
                CommandResult::Async(AsyncCommand::DeleteSession(args.to_string()))
            }
        }

        "rename" => {
            let mut parts = args.trim().splitn(2, char::is_whitespace);
            let target = parts.next().unwrap_or_default().trim();
            let title = parts.next().unwrap_or_default().trim();
            if target.is_empty() || title.is_empty() {
                CommandResult::Error("Usage: /rename <session-id> <title>".into())
            } else {
                CommandResult::Async(AsyncCommand::RenameSession {
                    target: target.to_string(),
                    title: title.to_string(),
                })
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

        "prompt" => CommandResult::Async(AsyncCommand::PromptTemplate(
            (!args.is_empty()).then(|| args.to_string()),
        )),

        "attach" => {
            if args.trim().is_empty() {
                CommandResult::Error("Usage: /attach <path>".into())
            } else {
                CommandResult::Async(AsyncCommand::AttachFile(args.trim().to_string()))
            }
        }

        "shortcuts" => CommandResult::OpenHelpOverlay,

        "copy" if args.trim().is_empty() => CommandResult::OpenCopyPicker,
        "copy" => match copy::parse_target(args) {
            Ok(target) => CommandResult::Copy(target),
            Err(message) => CommandResult::Error(message),
        },

        "queue" => CommandResult::OpenQueueOverlay,

        "todo" if args.trim().is_empty() => CommandResult::Error("Usage: /todo <text>".into()),
        "todo" => CommandResult::QueueFollowUp(args.trim().to_string()),

        "tasks" => CommandResult::OpenTasksOverlay,

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

/// Parse a permission-mode name as shown in the `/permission` picker.
pub fn parse_permission_mode(arg: &str) -> Option<PermissionMode> {
    match arg {
        "default" => Some(PermissionMode::Default),
        "plan" => Some(PermissionMode::Plan),
        "accept_edits" => Some(PermissionMode::AcceptEdits),
        "bypass_permissions" => Some(PermissionMode::BypassPermissions),
        "dont_ask" => Some(PermissionMode::DontAsk),
        _ => None,
    }
}

/// Generate the full help text.
fn generate_help_text() -> String {
    let reg = crate::tui::commands::registry::CommandRegistry::shared();
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
    let reg = crate::tui::commands::registry::CommandRegistry::shared();
    match reg.find(cmd_name) {
        Some(cmd) => {
            let alias_str = cmd
                .alias
                .map(|a| format!(" (alias: /{a})"))
                .unwrap_or_default();
            format!(
                "/{} {}\n{}{}\nCategory: {}",
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::{MessageRole, ToolSelection};
    use chrono::Utc;

    // T-TUI-04-04: /clear clears messages.
    #[test]
    fn test_clear_command() {
        let mut state = AppState::new();
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
        state.selected_tool = Some(ToolSelection {
            message_index: 0,
            tool_index: 0,
        });

        let result = execute("clear", &mut state);
        assert!(matches!(result, CommandResult::Ok(Some(ref msg)) if msg.contains("cleared")));
        assert!(state.messages.is_empty());
        assert!(state.selected_tool.is_none());
    }

    // T-TUI-04-05: /new resets state and returns NewSession.
    #[test]
    fn test_new_command() {
        let mut state = AppState::new();
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
        state.selected_tool = Some(ToolSelection {
            message_index: 0,
            tool_index: 0,
        });

        let result = execute("new", &mut state);
        assert!(matches!(result, CommandResult::NewSession));
        assert!(state.messages.is_empty());
        assert!(state.current_session_id.is_none());
        assert_eq!(state.user_message_count, 0);
        assert!(state.selected_tool.is_none());
    }

    #[test]
    fn test_new_command_clears_follow_up_queue_projection() {
        let mut state = AppState::new();
        state.current_session_id = Some("old-session".into());
        state.follow_up_queue.push(y_service::FollowUpMessage::new(
            "queued follow-up".to_string(),
        ));

        let result = execute("new", &mut state);

        assert!(matches!(result, CommandResult::NewSession));
        assert!(state.follow_up_queue.is_empty());
    }

    #[test]
    fn test_queue_command_opens_queue_overlay() {
        let mut state = AppState::new();
        assert!(matches!(
            execute("queue", &mut state),
            CommandResult::OpenQueueOverlay
        ));
    }

    #[test]
    fn test_todo_command_routes_text_to_active_run_queue() {
        let mut state = AppState::new();
        assert!(matches!(
            execute("todo inspect the failing test", &mut state),
            CommandResult::QueueFollowUp(text) if text == "inspect the failing test"
        ));
        assert!(matches!(
            execute("todo", &mut state),
            CommandResult::Error(message) if message.contains("/todo <text>")
        ));
    }

    #[test]
    fn test_tasks_command_opens_tasks_overlay() {
        let mut state = AppState::new();
        assert!(matches!(
            execute("tasks", &mut state),
            CommandResult::OpenTasksOverlay
        ));
    }

    // T-TUI-04-06: unknown command returns error.
    #[test]
    fn test_unknown_command() {
        let mut state = AppState::new();
        let result = execute("foobar", &mut state);
        assert!(matches!(result, CommandResult::Error(ref msg) if msg.contains("Unknown")));
    }

    #[test]
    fn test_quit_command() {
        let mut state = AppState::new();
        let result = execute("quit", &mut state);
        assert!(matches!(result, CommandResult::Quit));
    }

    #[test]
    fn test_quit_alias() {
        let mut state = AppState::new();
        let result = execute("q", &mut state);
        assert!(matches!(result, CommandResult::Quit));
    }

    #[test]
    fn test_help_command() {
        let mut state = AppState::new();
        let result = execute("help", &mut state);
        assert!(matches!(result, CommandResult::Ok(None)));
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].content.contains("Available commands"));
    }

    #[test]
    fn test_shortcuts_command_opens_dynamic_help_overlay() {
        let mut state = AppState::new();
        let result = execute("shortcuts", &mut state);

        assert!(matches!(result, CommandResult::OpenHelpOverlay));
        assert!(state.messages.is_empty());
    }

    // T-TUI-04-08: /help <command> shows details without a literal "\n".
    #[test]
    fn test_help_specific_command_has_no_literal_backslash_n() {
        let mut state = AppState::new();
        let result = execute("help new", &mut state);
        assert!(matches!(result, CommandResult::Ok(None)));
        let content = &state.messages[0].content;
        assert!(content.contains("/new [label]"), "got: {content}");
        assert!(content.contains("(alias: /n)"), "got: {content}");
        assert!(content.contains("Category: Session"), "got: {content}");
        assert!(
            !content.contains("\\n"),
            "help output must not contain a literal backslash-n: {content}"
        );
    }

    #[test]
    fn test_status_command() {
        let mut state = AppState::new();
        let result = execute("status", &mut state);
        assert!(matches!(result, CommandResult::Ok(None)));
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].content.contains("Messages:"));
    }

    #[test]
    fn test_reset_command() {
        let mut state = AppState::new();
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

    // /new and /reset destroy the visible transcript, so they are refused
    // while a response is streaming (Esc cancels the turn first).
    #[test]
    fn test_new_and_reset_refused_while_streaming() {
        for command in ["new", "reset"] {
            let mut state = AppState::new();
            state.is_streaming = true;
            state.messages.push(ChatMessage {
                role: MessageRole::User,
                content: "in-flight turn".into(),
                timestamp: Utc::now(),
                is_streaming: false,
                is_cancelled: false,
                reasoning_content: String::new(),
                reasoning_complete: false,
                tool_calls: Vec::new(),
                segments: Vec::new(),
            });

            let result = execute(command, &mut state);

            assert!(
                matches!(result, CommandResult::Error(ref msg)
                    if msg.contains("Wait for the current turn to finish or press Esc to cancel it")),
                "/{command} must be refused while streaming: {result:?}"
            );
            assert_eq!(
                state.messages.len(),
                1,
                "/{command} must not clear the transcript mid-turn"
            );
        }
    }

    // T-TUI-04-07: async commands return Async variant.
    #[test]
    fn test_list_returns_async() {
        let mut state = AppState::new();
        let result = execute("list", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ListSessions)
        ));
    }

    #[test]
    fn test_switch_requires_args() {
        let mut state = AppState::new();
        let result = execute("switch", &mut state);
        assert!(matches!(result, CommandResult::Error(_)));

        let result = execute("switch my-session", &mut state);
        assert!(
            matches!(result, CommandResult::Async(AsyncCommand::SwitchSession(ref s)) if s == "my-session")
        );
    }

    #[test]
    fn test_delete_requires_args() {
        let mut state = AppState::new();
        let result = execute("delete", &mut state);
        assert!(matches!(result, CommandResult::Error(_)));

        let result = execute("delete abc-123", &mut state);
        assert!(
            matches!(result, CommandResult::Async(AsyncCommand::DeleteSession(ref s)) if s == "abc-123")
        );
    }

    #[test]
    fn test_branch_optional_label() {
        let mut state = AppState::new();
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
        let mut state = AppState::new();
        let result = execute("compact", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::CompactContext)
        ));
    }

    #[test]
    fn test_stats_returns_async() {
        let mut state = AppState::new();
        let result = execute("stats", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ShowStats)
        ));
    }

    #[test]
    fn test_model_no_args_returns_async_none() {
        let mut state = AppState::new();
        let result = execute("model", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ModelCommand(None))
        ));
    }

    #[test]
    fn test_model_with_args_returns_async_some() {
        let mut state = AppState::new();
        let result = execute("model deepseek", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ModelCommand(Some(ref id))) if id == "deepseek"
        ));
    }

    #[test]
    fn test_agent_returns_async() {
        let mut state = AppState::new();
        let result = execute("agent", &mut state);
        assert!(matches!(
            result,
            CommandResult::Async(AsyncCommand::ShowAgents)
        ));
    }

    #[test]
    fn test_resume_supports_picker_and_direct_target() {
        let mut state = AppState::new();
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
    fn test_prompt_supports_picker_and_direct_target() {
        let mut state = AppState::new();
        assert!(matches!(
            execute("prompt", &mut state),
            CommandResult::Async(AsyncCommand::PromptTemplate(None))
        ));
        assert!(matches!(
            execute("prompt review", &mut state),
            CommandResult::Async(AsyncCommand::PromptTemplate(Some(ref target)))
                if target == "review"
        ));
    }

    #[test]
    fn test_goal_submits_auto_orchestrated_turn() {
        let mut state = AppState::new();
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
        let mut state = AppState::new();
        assert!(matches!(
            execute("copy", &mut state),
            CommandResult::OpenCopyPicker
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
        let mut state = AppState::new();
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
        let mut state = AppState::new();
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

    #[test]
    fn test_permission_command_validates_mode() {
        let mut state = AppState::new();
        assert!(matches!(
            execute("permission plan", &mut state),
            CommandResult::SetPermissionMode(PermissionMode::Plan)
        ));
        assert!(matches!(
            execute("permission bypass_permissions", &mut state),
            CommandResult::SetPermissionMode(PermissionMode::BypassPermissions)
        ));
        // The alias resolves to the same command.
        assert!(matches!(
            execute("perm dont_ask", &mut state),
            CommandResult::SetPermissionMode(PermissionMode::DontAsk)
        ));
        assert!(matches!(
            execute("permission bogus", &mut state),
            CommandResult::Error(ref message) if message.contains("bogus")
        ));
        assert!(matches!(
            execute("permission", &mut state),
            CommandResult::Ok(Some(ref message)) if message.contains("Usage")
        ));
    }
}
