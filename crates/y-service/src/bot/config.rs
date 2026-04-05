//! Bot persona configuration schema.
//!
//! Deserialisation target for `config/persona/persona.toml` -- the structural
//! settings that the operator manages. The bot does NOT modify this file.

use serde::Deserialize;

/// Root of `persona.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct BotConfig {
    /// Persona identity settings.
    pub persona: PersonaConfig,
}

/// `[persona]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PersonaConfig {
    /// Default bot name (may be overridden by `IDENTITY.md`).
    pub name: String,
    /// Language preference: `"auto"`, `"en"`, `"zh"`, etc.
    pub language: String,
    /// Whether persona-aware prompting is enabled.
    /// When `false`, `BotService` falls back to pass-through behaviour.
    pub enabled: bool,
    /// Messaging constraints.
    pub messaging: MessagingConfig,
    /// Memory settings.
    pub memory: MemoryConfig,
    /// Tool access policy.
    pub tools: ToolsConfig,
    /// Bootstrap (first-run) settings.
    pub bootstrap: BootstrapConfig,
}

/// `[persona.messaging]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MessagingConfig {
    /// Maximum response length in characters (platform-dependent override).
    pub max_response_length: usize,
    /// Prefer short, concise replies.
    pub prefer_short_responses: bool,
    /// Markdown dialect: `"auto"`, `"discord"`, `"telegram"`, `"plain"`.
    pub markdown_dialect: String,
}

/// `[persona.memory]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Number of recent turns to keep in the context window.
    pub conversation_memory_turns: usize,
    /// Enable vector-backed persona memory.
    pub persona_memory_enabled: bool,
    /// Auto-extract user preferences from completed turns.
    pub memory_extraction_enabled: bool,
}

/// `[persona.tools]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Tool names that the bot is allowed to invoke.
    pub allowed_tools: Vec<String>,
    /// Maximum tool-call iterations per turn.
    pub max_tool_iterations: usize,
    /// Whether to enable LLM thinking/reasoning.
    pub enable_thinking: bool,
}

/// `[persona.bootstrap]` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BootstrapConfig {
    /// Whether the first-run bootstrap ritual is enabled.
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            name: "Y".to_string(),
            language: "auto".to_string(),
            enabled: false,
            messaging: MessagingConfig::default(),
            memory: MemoryConfig::default(),
            tools: ToolsConfig::default(),
            bootstrap: BootstrapConfig::default(),
        }
    }
}

impl Default for MessagingConfig {
    fn default() -> Self {
        Self {
            max_response_length: 2000,
            prefer_short_responses: true,
            markdown_dialect: "auto".to_string(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            conversation_memory_turns: 50,
            persona_memory_enabled: true,
            memory_extraction_enabled: true,
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            allowed_tools: vec![
                "KnowledgeSearch".to_string(),
                "WebSearch".to_string(),
                "DateTime".to_string(),
            ],
            max_tool_iterations: 3,
            enable_thinking: false,
        }
    }
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_default_matches_struct_default() {
        let toml_str = "";
        let config: BotConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.persona.name, "Y");
        assert!(!config.persona.enabled);
        assert_eq!(config.persona.messaging.max_response_length, 2000);
        assert_eq!(config.persona.tools.max_tool_iterations, 3);
    }

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
[persona]
name = "TestBot"
language = "en"
enabled = true

[persona.messaging]
max_response_length = 4000
prefer_short_responses = false
markdown_dialect = "discord"

[persona.memory]
conversation_memory_turns = 100
persona_memory_enabled = false
memory_extraction_enabled = false

[persona.tools]
allowed_tools = ["DateTime"]
max_tool_iterations = 5
enable_thinking = true

[persona.bootstrap]
enabled = false
"#;
        let config: BotConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.persona.name, "TestBot");
        assert!(config.persona.enabled);
        assert_eq!(config.persona.messaging.max_response_length, 4000);
        assert!(!config.persona.messaging.prefer_short_responses);
        assert_eq!(config.persona.memory.conversation_memory_turns, 100);
        assert!(!config.persona.memory.persona_memory_enabled);
        assert_eq!(config.persona.tools.allowed_tools, vec!["DateTime"]);
        assert_eq!(config.persona.tools.max_tool_iterations, 5);
        assert!(config.persona.tools.enable_thinking);
        assert!(!config.persona.bootstrap.enabled);
    }

    #[test]
    fn deserialize_partial_config_uses_defaults() {
        let toml_str = r#"
[persona]
name = "CustomBot"
enabled = true
"#;
        let config: BotConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.persona.name, "CustomBot");
        assert!(config.persona.enabled);
        // Defaults for unspecified sections.
        assert_eq!(config.persona.messaging.max_response_length, 2000);
        assert_eq!(config.persona.tools.max_tool_iterations, 3);
        assert!(config.persona.bootstrap.enabled);
    }
}
