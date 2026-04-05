//! Bot persona: loads identity, personality, and memory from `config/persona/`.
//!
//! Reads the Markdown persona files (`SOUL.md`, `IDENTITY.md`, `USER.md`,
//! `MEMORY.md`, `BOOTSTRAP.md`) and the structural `persona.toml`. Files are
//! re-read on each turn because the bot may self-modify them at runtime.

use std::path::{Path, PathBuf};

use tracing::warn;

use super::config::BotConfig;

/// Well-known file names in the `config/persona/` directory.
const PERSONA_TOML: &str = "persona.toml";
const SOUL_MD: &str = "SOUL.md";
const IDENTITY_MD: &str = "IDENTITY.md";
const USER_MD: &str = "USER.md";
const MEMORY_MD: &str = "MEMORY.md";
const BOOTSTRAP_MD: &str = "BOOTSTRAP.md";

/// Loaded bot persona: structural config + Markdown file contents.
///
/// Created via [`BotPersona::load`] for file-based personas or
/// [`BotPersona::default_embedded`] for fallback when the directory is absent.
#[derive(Debug, Clone)]
pub struct BotPersona {
    /// Structural configuration from `persona.toml`.
    pub config: BotConfig,
    /// Root directory path (for auditing / diagnostics).
    pub persona_dir: Option<PathBuf>,
    /// Content of `SOUL.md`.
    pub soul: String,
    /// Content of `IDENTITY.md`.
    pub identity: String,
    /// Content of `USER.md`.
    pub user: String,
    /// Content of `MEMORY.md`.
    pub memory: String,
    /// Content of `BOOTSTRAP.md` (empty when file is absent or deleted).
    pub bootstrap: String,
}

impl BotPersona {
    /// Load a persona from the given directory.
    ///
    /// - Reads `persona.toml` for structural config; falls back to defaults on
    ///   parse failure.
    /// - Reads each Markdown file; missing files produce empty strings (not errors).
    /// - `BOOTSTRAP.md` is only read when `persona.bootstrap.enabled` is true.
    pub fn load(persona_dir: &Path) -> Self {
        let config = Self::load_config(persona_dir);

        let soul = read_file_or_empty(&persona_dir.join(SOUL_MD), SOUL_MD);
        let identity = read_file_or_empty(&persona_dir.join(IDENTITY_MD), IDENTITY_MD);
        let user = read_file_or_empty(&persona_dir.join(USER_MD), USER_MD);
        let memory = read_file_or_empty(&persona_dir.join(MEMORY_MD), MEMORY_MD);

        let bootstrap = if config.persona.bootstrap.enabled {
            read_file_or_empty(&persona_dir.join(BOOTSTRAP_MD), BOOTSTRAP_MD)
        } else {
            String::new()
        };

        Self {
            config,
            persona_dir: Some(persona_dir.to_path_buf()),
            soul,
            identity,
            user,
            memory,
            bootstrap,
        }
    }

    /// Default embedded persona for when `config/persona/` is absent.
    ///
    /// Uses inline defaults so the bot can function without any persona files.
    pub fn default_embedded() -> Self {
        Self {
            config: BotConfig::default(),
            persona_dir: None,
            soul: String::new(),
            identity: String::new(),
            user: String::new(),
            memory: String::new(),
            bootstrap: String::new(),
        }
    }

    /// Whether persona-driven prompting is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.persona.enabled
    }

    /// The bot's display name from config.
    pub fn name(&self) -> &str {
        &self.config.persona.name
    }

    /// Load and parse `persona.toml`, falling back to defaults on any error.
    fn load_config(persona_dir: &Path) -> BotConfig {
        let path = persona_dir.join(PERSONA_TOML);
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse persona.toml; using defaults"
                    );
                    BotConfig::default()
                }
            },
            Err(_) => {
                // File not found is not an error -- just use defaults.
                BotConfig::default()
            }
        }
    }
}

/// Read a file to string; return empty on any I/O error (missing file is normal).
fn read_file_or_empty(path: &Path, label: &str) -> String {
    match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "Failed to read persona file {label}"
            );
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_empty_dir_returns_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let persona = BotPersona::load(dir.path());
        assert!(!persona.is_enabled());
        assert_eq!(persona.name(), "Y");
        assert!(persona.soul.is_empty());
        assert!(persona.identity.is_empty());
    }

    #[test]
    fn load_reads_markdown_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("SOUL.md"), "Be helpful.").unwrap();
        std::fs::write(dir.path().join("IDENTITY.md"), "Name: Bot").unwrap();
        std::fs::write(dir.path().join("USER.md"), "Name: Alice").unwrap();
        std::fs::write(dir.path().join("MEMORY.md"), "Likes Rust.").unwrap();

        let persona = BotPersona::load(dir.path());
        assert_eq!(persona.soul, "Be helpful.");
        assert_eq!(persona.identity, "Name: Bot");
        assert_eq!(persona.user, "Name: Alice");
        assert_eq!(persona.memory, "Likes Rust.");
    }

    #[test]
    fn load_reads_persona_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("persona.toml"),
            r#"
[persona]
name = "TestBot"
enabled = true

[persona.tools]
max_tool_iterations = 5
"#,
        )
        .unwrap();

        let persona = BotPersona::load(dir.path());
        assert!(persona.is_enabled());
        assert_eq!(persona.name(), "TestBot");
        assert_eq!(persona.config.persona.tools.max_tool_iterations, 5);
    }

    #[test]
    fn load_skips_bootstrap_when_disabled() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("persona.toml"),
            "[persona.bootstrap]\nenabled = false\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("BOOTSTRAP.md"),
            "This should not be loaded.",
        )
        .unwrap();

        let persona = BotPersona::load(dir.path());
        assert!(persona.bootstrap.is_empty());
    }

    #[test]
    fn load_reads_bootstrap_when_enabled() {
        let dir = tempfile::TempDir::new().unwrap();
        // Default bootstrap.enabled is true.
        std::fs::write(dir.path().join("BOOTSTRAP.md"), "Welcome ritual.").unwrap();

        let persona = BotPersona::load(dir.path());
        assert_eq!(persona.bootstrap, "Welcome ritual.");
    }

    #[test]
    fn default_embedded_returns_disabled_persona() {
        let persona = BotPersona::default_embedded();
        assert!(!persona.is_enabled());
        assert!(persona.persona_dir.is_none());
    }

    #[test]
    fn malformed_toml_falls_back_to_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("persona.toml"), "invalid {{} toml").unwrap();

        let persona = BotPersona::load(dir.path());
        assert!(!persona.is_enabled());
        assert_eq!(persona.name(), "Y");
    }
}
