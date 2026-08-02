//! Command registry: stores command definitions, aliases, and search.
//!
//! Each command has a name, alias, description, and argument synopsis.
//! The registry supports prefix-based fuzzy search for the command palette.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::tui::keys::KeyAction;

/// Information about a registered command.
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// Primary command name (e.g. "new").
    pub name: &'static str,
    /// Short alias (e.g. "n").
    pub alias: Option<&'static str>,
    /// One-line description.
    pub description: &'static str,
    /// Argument synopsis (e.g. "\[label\]").
    pub args: &'static str,
    /// Category for grouping.
    pub category: CommandCategory,
}

impl CommandInfo {
    /// Semantic action that opens the same surface without typing the command.
    pub fn shortcut_action(&self) -> Option<KeyAction> {
        match self.name {
            "resume" => Some(KeyAction::OpenSessionHub),
            "retry" => Some(KeyAction::RetryLastRequest),
            "shortcuts" => Some(KeyAction::ShowHelp),
            "copy" => Some(KeyAction::OpenCopy),
            "queue" => Some(KeyAction::OpenQueue),
            "tasks" => Some(KeyAction::OpenTasks),
            "quit" => Some(KeyAction::Quit),
            _ => None,
        }
    }
}

/// Command categories for grouping in palette and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Session,
    Agent,
    Mode,
    Model,
    Debug,
    General,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Agent => "Agent",
            Self::Mode => "Mode",
            Self::Model => "Model",
            Self::Debug => "Debug",
            Self::General => "General",
        }
    }
}

/// Registry of all available commands.
pub struct CommandRegistry {
    /// Commands indexed by primary name.
    commands: Vec<CommandInfo>,
    /// Alias → primary name mapping.
    aliases: HashMap<&'static str, &'static str>,
}

impl CommandRegistry {
    /// Create a new registry with all built-in commands.
    ///
    /// Prefer [`CommandRegistry::shared`] for runtime use; `new()` remains
    /// for tests that want an isolated instance.
    pub fn new() -> Self {
        let commands = builtin_commands();
        let mut aliases = HashMap::new();
        for cmd in &commands {
            if let Some(alias) = cmd.alias {
                aliases.insert(alias, cmd.name);
            }
        }
        Self { commands, aliases }
    }

    /// Process-wide shared registry, built once on first access.
    ///
    /// The built-in command set is static, so a single instance can serve
    /// the command palette, help generation, and alias resolution without
    /// rebuilding per keystroke, frame, or command execution.
    pub fn shared() -> &'static Self {
        static SHARED: OnceLock<CommandRegistry> = OnceLock::new();
        SHARED.get_or_init(Self::new)
    }

    /// Resolve an alias to its primary command name.
    pub fn resolve_alias<'a>(&self, input: &'a str) -> &'a str {
        self.aliases.get(input).copied().unwrap_or(input)
    }

    /// Find a command by name or alias.
    pub fn find(&self, input: &str) -> Option<&CommandInfo> {
        let name = self.resolve_alias(input);
        self.commands.iter().find(|c| c.name == name)
    }

    /// Search commands by prefix (for command palette fuzzy filter).
    ///
    /// Returns commands whose name or alias starts with the given prefix, or
    /// whose description contains it. Name/alias prefix matches always rank
    /// before description-substring matches so that e.g. `plan` outranks
    /// `auto` ("...select fast, plan, or loop") for the query "plan".
    pub fn search(&self, prefix: &str) -> Vec<&CommandInfo> {
        let prefix = prefix.to_lowercase();
        let prefix_match = |c: &CommandInfo| {
            c.name.starts_with(&prefix) || c.alias.is_some_and(|a| a.starts_with(&prefix))
        };
        let (primary, secondary): (Vec<_>, Vec<_>) = self
            .commands
            .iter()
            .filter(|c| prefix_match(c) || c.description.to_lowercase().contains(&prefix))
            .partition(|c| prefix_match(c));
        primary.into_iter().chain(secondary).collect()
    }

    /// Get all commands, grouped by category.
    pub fn all(&self) -> &[CommandInfo] {
        &self.commands
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in command definitions.
fn builtin_commands() -> Vec<CommandInfo> {
    vec![
        // Session commands
        CommandInfo {
            name: "new",
            alias: Some("n"),
            description: "Create a new session",
            args: "[label]",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "switch",
            alias: Some("sw"),
            description: "Switch to another session",
            args: "<session-id|label>",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "resume",
            alias: Some("r"),
            description: "Resume a recent session",
            args: "[session-id|title]",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "retry",
            alias: None,
            description: "Retry the most recent LLM request",
            args: "",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "list",
            alias: Some("ls"),
            description: "List all sessions",
            args: "",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "delete",
            alias: Some("del"),
            description: "Delete a session",
            args: "<session-id>",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "rename",
            alias: Some("rn"),
            description: "Rename a session",
            args: "<session-id> <title>",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "reset",
            alias: None,
            description: "Reset current session (clear messages)",
            args: "",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "branch",
            alias: Some("br"),
            description: "Branch from current point",
            args: "[label]",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "compact",
            alias: None,
            description: "Compact context (summarize older messages)",
            args: "",
            category: CommandCategory::Session,
        },
        CommandInfo {
            name: "export",
            alias: None,
            description: "Export session to file",
            args: "[format: md|json]",
            category: CommandCategory::Session,
        },
        // Agent commands
        CommandInfo {
            name: "agent",
            alias: Some("a"),
            description: "Agent management (list, select, info)",
            args: "<subcommand> [args]",
            category: CommandCategory::Agent,
        },
        CommandInfo {
            name: "goal",
            alias: Some("g"),
            description: "Run an objective with automatic orchestration",
            args: "<objective>",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "mode",
            alias: Some("md"),
            description: "Select the mode for subsequent turns",
            args: "[fast|auto|plan|loop]",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "permission",
            alias: Some("perm"),
            description: "View or change the session permission mode",
            args: "[default|plan|accept_edits|bypass_permissions|dont_ask]",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "fast",
            alias: None,
            description: "Use direct execution for subsequent turns",
            args: "[prompt]",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "auto",
            alias: None,
            description: "Let y-agent select fast, plan, or loop",
            args: "[prompt]",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "plan",
            alias: Some("p"),
            description: "Use reviewed structured planning",
            args: "[prompt]",
            category: CommandCategory::Mode,
        },
        CommandInfo {
            name: "loop",
            alias: Some("l"),
            description: "Use iterative execution and self-review",
            args: "[prompt]",
            category: CommandCategory::Mode,
        },
        // Model commands
        CommandInfo {
            name: "model",
            alias: Some("m"),
            description: "List models or switch active provider",
            args: "[provider-id]",
            category: CommandCategory::Model,
        },
        CommandInfo {
            name: "prompt",
            alias: Some("pr"),
            description: "Select the session prompt template",
            args: "[template-id|default]",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "attach",
            alias: Some("file"),
            description: "Attach a local file to the next turn",
            args: "<path>",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "theme",
            alias: None,
            description: "Choose a built-in or custom color scheme",
            args: "[name]",
            category: CommandCategory::General,
        },
        // Debug commands
        CommandInfo {
            name: "debug",
            alias: None,
            description: "Toggle debug mode",
            args: "[--on|--off]",
            category: CommandCategory::Debug,
        },
        CommandInfo {
            name: "status",
            alias: Some("st"),
            description: "Show system status",
            args: "",
            category: CommandCategory::Debug,
        },
        CommandInfo {
            name: "stats",
            alias: None,
            description: "Show token/cost statistics",
            args: "",
            category: CommandCategory::Debug,
        },
        // General commands
        CommandInfo {
            name: "help",
            alias: Some("h"),
            description: "Show help / keyboard shortcuts",
            args: "[command]",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "clear",
            alias: Some("cl"),
            description: "Clear chat display",
            args: "",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "shortcuts",
            alias: Some("keys"),
            description: "Show keyboard shortcuts",
            args: "",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "copy",
            alias: Some("cp"),
            description: "Copy a response, code block, or transcript",
            args: "[N|code|transcript]",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "queue",
            alias: None,
            description: "Manage TODOs queued for the active run",
            args: "",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "todo",
            alias: None,
            description: "Add a TODO to the active run",
            args: "<text>",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "tasks",
            alias: None,
            description: "Show background tasks and subagents",
            args: "",
            category: CommandCategory::General,
        },
        CommandInfo {
            name: "quit",
            alias: Some("q"),
            description: "Quit the TUI",
            args: "",
            category: CommandCategory::General,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T-TUI-04-01: CommandRegistry resolves aliases.
    #[test]
    fn test_resolve_aliases() {
        let reg = CommandRegistry::new();
        assert_eq!(reg.resolve_alias("n"), "new");
        assert_eq!(reg.resolve_alias("sw"), "switch");
        assert_eq!(reg.resolve_alias("ls"), "list");
        assert_eq!(reg.resolve_alias("q"), "quit");
        assert_eq!(reg.resolve_alias("h"), "help");
        // Unknown alias returns itself.
        assert_eq!(reg.resolve_alias("unknown"), "unknown");
    }

    // T-TUI-04-02: search("sw") returns matching commands.
    #[test]
    fn test_search_prefix() {
        let reg = CommandRegistry::new();

        let results = reg.search("sw");
        let names: Vec<&str> = results.iter().map(|c| c.name).collect();
        assert!(
            names.contains(&"switch"),
            "should find 'switch' by alias 'sw'"
        );
    }

    #[test]
    fn test_search_by_name() {
        let reg = CommandRegistry::new();

        let results = reg.search("new");
        let names: Vec<&str> = results.iter().map(|c| c.name).collect();
        assert!(names.contains(&"new"));
    }

    #[test]
    fn test_search_by_description() {
        let reg = CommandRegistry::new();

        let results = reg.search("session");
        assert!(
            results.len() >= 3,
            "should find multiple session-related commands"
        );
    }

    // Regression: name/alias prefix matches must outrank description-substring
    // matches, so typing `/plan` does not highlight `/auto` ("...fast, plan,
    // or loop") first.
    #[test]
    fn test_search_prefix_matches_rank_before_description_matches() {
        let reg = CommandRegistry::new();

        let results = reg.search("plan");
        assert_eq!(results.first().map(|c| c.name), Some("plan"));
        assert!(
            results.iter().any(|c| c.name == "auto"),
            "auto should still be found via its description"
        );
    }

    #[test]
    fn test_find_by_name_and_alias() {
        let reg = CommandRegistry::new();

        let cmd = reg.find("new").expect("should find 'new'");
        assert_eq!(cmd.name, "new");

        let cmd = reg.find("n").expect("should find 'n' → 'new'");
        assert_eq!(cmd.name, "new");

        assert!(reg.find("nonexistent").is_none());
    }

    #[test]
    fn test_all_commands_registered() {
        let reg = CommandRegistry::new();
        assert!(reg.all().len() >= 18, "should have at least 18 commands");
        assert!(
            reg.find("goal").is_some(),
            "goal command should be discoverable"
        );
        assert!(
            reg.find("resume").is_some(),
            "resume command should be discoverable"
        );
        assert!(
            reg.find("mode").is_some(),
            "mode command should be discoverable"
        );
        assert!(
            reg.find("plan").is_some(),
            "plan command should be discoverable"
        );
        assert!(
            reg.find("loop").is_some(),
            "loop command should be discoverable"
        );
        assert!(
            reg.find("prompt").is_some(),
            "prompt command should be discoverable"
        );
        assert!(
            reg.find("queue").is_some(),
            "queue command should be discoverable"
        );
        assert!(
            reg.find("todo").is_some(),
            "todo command should be discoverable"
        );
        assert!(
            reg.find("theme").is_some(),
            "theme command should be discoverable"
        );
        assert!(
            reg.find("tasks").is_some(),
            "tasks command should be discoverable"
        );
    }

    #[test]
    fn test_command_first_workflow_metadata() {
        let reg = CommandRegistry::new();

        let goal = reg.find("goal").expect("goal command");
        assert_eq!(goal.args, "<objective>");

        let resume = reg.find("resume").expect("resume command");
        assert_eq!(resume.args, "[session-id|title]");

        let copy = reg.find("copy").expect("copy command");
        assert_eq!(copy.args, "[N|code|transcript]");

        let prompt = reg.find("prompt").expect("prompt command");
        assert_eq!(prompt.args, "[template-id|default]");

        let todo = reg.find("todo").expect("todo command");
        assert_eq!(todo.args, "<text>");

        let retry = reg.find("retry").expect("retry command");
        assert_eq!(retry.shortcut_action(), Some(KeyAction::RetryLastRequest));
    }

    #[test]
    fn test_command_categories() {
        let reg = CommandRegistry::new();
        let session_cmds: Vec<_> = reg
            .all()
            .iter()
            .filter(|c| c.category == CommandCategory::Session)
            .collect();
        assert!(session_cmds.len() >= 5, "should have >= 5 session commands");
    }

    // T-REGISTRY-SHARED-01: shared() returns the same instance across calls.
    #[test]
    fn test_shared_returns_same_instance() {
        let first = CommandRegistry::shared();
        let second = CommandRegistry::shared();
        assert!(
            std::ptr::eq(first, second),
            "shared() must return the same registry instance"
        );
    }

    // T-REGISTRY-SHARED-02: shared() behaves like a fresh registry.
    #[test]
    fn test_shared_resolves_aliases_and_searches() {
        let reg = CommandRegistry::shared();
        assert_eq!(reg.resolve_alias("n"), "new");
        assert!(reg.find("sw").is_some_and(|c| c.name == "switch"));
        assert!(!reg.search("session").is_empty());
    }
}
