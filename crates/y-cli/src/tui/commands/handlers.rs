//! Command handlers: execute commands against `AppState` and services.
//!
//! Each handler receives a parsed command line, the mutable `AppState`,
//! and returns a `CommandResult` indicating success, failure, or output.

use crate::tui::state::{AppState, ChatMessage, MessageRole};
use chrono::Utc;

/// Result of executing a command.
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// Command succeeded with an optional message to display.
    Ok(Option<String>),
    /// Command failed with an error message.
    Error(String),
    /// Quit the application.
    Quit,
    /// A new session was requested — state has been reset.
    /// The TUI event loop should handle any async follow-up.
    NewSession,
}

/// Parse and execute a command string.
///
/// The input is the raw text after the `/` prefix, e.g. `"new my session"`.
/// In Phase T5+, this will also accept `&AppServices` for real operations.
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
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: help_text,
                timestamp: Utc::now(),
                is_streaming: false,
                is_cancelled: false,
            });
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
                "Messages: {} | Streaming: {} | Sidebar: {} | Mode: {:?} | Focus: {:?}",
                state.messages.len(),
                state.is_streaming,
                state.sidebar_visible,
                state.mode,
                state.focus,
            );
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: msg,
                timestamp: Utc::now(),
                is_streaming: false,
                is_cancelled: false,
            });
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

        // Stub handlers for commands that need AppServices (Phase T5).
        "list" | "switch" | "delete" | "branch" | "export" | "agent" | "model" | "stats" => {
            CommandResult::Ok(Some(format!(
                "/{resolved} requires server connection (coming in Phase T5)."
            )))
        }

        "shortcuts" => {
            let text = generate_shortcuts_text();
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: text,
                timestamp: Utc::now(),
                is_streaming: false,
                is_cancelled: false,
            });
            CommandResult::Ok(None)
        }

        "copy" => {
            if state.messages.is_empty() {
                return CommandResult::Ok(Some("No messages to copy.".into()));
            }
            let formatted: String = state
                .messages
                .iter()
                .map(|m| {
                    let role = match m.role {
                        MessageRole::User => "You",
                        MessageRole::Assistant => "Assistant",
                        MessageRole::System => "System",
                        MessageRole::Tool => "Tool",
                    };
                    format!("[{role}]\n{}", m.content)
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            #[cfg(feature = "tui")]
            {
                match arboard::Clipboard::new() {
                    Ok(mut clipboard) => match clipboard.set_text(&formatted) {
                        Ok(()) => CommandResult::Ok(Some(format!(
                            "Copied {} messages to clipboard.",
                            state.messages.len()
                        ))),
                        Err(e) => CommandResult::Error(format!("Failed to set clipboard: {e}")),
                    },
                    Err(e) => CommandResult::Error(format!("Failed to access clipboard: {e}")),
                }
            }

            #[cfg(not(feature = "tui"))]
            CommandResult::Error("Clipboard not available without TUI feature.".into())
        }

        _ => CommandResult::Error(format!(
            "Unknown command: /{cmd_name}. Type /help for a list."
        )),
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
            text.push_str(&format!("  [{}]\n", cmd.category.label()));
        }
        let alias_str = cmd.alias.map(|a| format!(" (/{a})")).unwrap_or_default();
        text.push_str(&format!(
            "    /{}{:<10} {}\n",
            cmd.name, alias_str, cmd.description
        ));
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
                "/{} {}\n{}{}\n\nCategory: {}",
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
    Ctrl+B                    Toggle sidebar\n\n",
    );

    text.push_str(
        "  [Input Panel]
    Enter                     Send message
    Shift+Enter               New line
    Tab                       Cycle focus (Input → Chat → Sidebar)
    /                         Open command palette (on empty input)
    :                         Open command palette (vim-style)
    Esc                       Return to normal mode\n\n",
    );

    text.push_str(
        "  [Chat Panel]
    j / ↓ / PageDown          Scroll down
    k / ↑ / PageUp            Scroll up
    i                         Return focus to input
    Tab                       Cycle focus\n\n",
    );

    text.push_str(
        "  [Sidebar]
    Tab                       Cycle focus
    Shift+Tab                 Switch sidebar view
    Esc                       Return focus to input\n\n",
    );

    text.push_str(
        "  [Command Palette]
    ↑ / ↓                     Navigate suggestions
    Tab                       Next suggestion
    Enter                     Execute selected command
    Esc                       Close palette\n\n",
    );

    text.push_str(
        "  [Mouse]
    Click                     Focus panel (Chat/Input/Sidebar)
    Scroll wheel              Scroll chat history
    Shift + drag              Native text selection (terminal)
    /copy                     Copy full transcript to clipboard\n",
    );

    text
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        });
        let result = execute("reset", &mut state);
        assert!(matches!(result, CommandResult::Ok(Some(_))));
        assert!(state.messages.is_empty());
    }
}
