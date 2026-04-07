//! Command registry: stores command definitions, aliases, and search.
//!
//! Each command has a name, alias, description, and argument synopsis.
//! The registry supports prefix-based fuzzy search for the command palette.

use std::collections::HashMap;

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

/// Command categories for grouping in palette and help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Session,
    Agent,
    Model,
    Debug,
    General,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Agent => "Agent",
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
    /// Returns commands whose name or alias starts with the given prefix.
    pub fn search(&self, prefix: &str) -> Vec<&CommandInfo> {
        let prefix = prefix.to_lowercase();
        self.commands
            .iter()
            .filter(|c| {
                c.name.starts_with(&prefix)
                    || c.alias.is_some_and(|a| a.starts_with(&prefix))
                    || c.description.to_lowercase().contains(&prefix)
            })
            .collect()
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
        // Model commands
        CommandInfo {
            name: "model",
            alias: Some("m"),
            description: "List models or switch active provider",
            args: "[provider-id]",
            category: CommandCategory::Model,
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
            description: "Copy chat transcript to clipboard",
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
}
