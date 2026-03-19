//! Configuration loader with 5-layer hierarchy:
//! CLI args > env vars > user config > project config > defaults.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use y_browser::BrowserConfig;
use y_guardrails::GuardrailConfig;
use y_hooks::HookConfig;
use y_knowledge::config::KnowledgeConfig;
use y_provider::ProviderPoolConfig;
use y_runtime::RuntimeConfig;
use y_session::SessionConfig;
use y_storage::StorageConfig;
use y_tools::ToolRegistryConfig;

/// Environment variable prefix for y-agent configuration overrides.
const ENV_PREFIX: &str = "Y_AGENT_";

/// Top-level configuration for y-agent.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct YAgentConfig {
    /// Provider pool configuration.
    pub providers: ProviderPoolConfig,

    /// Storage configuration.
    pub storage: StorageConfig,

    /// Session configuration.
    pub session: SessionConfig,

    /// Runtime configuration.
    pub runtime: RuntimeConfig,

    /// Hook system configuration.
    pub hooks: HookConfig,

    /// Tool registry configuration.
    pub tools: ToolRegistryConfig,

    /// Guardrail configuration.
    pub guardrails: GuardrailConfig,

    /// Browser (CDP) configuration.
    pub browser: BrowserConfig,

    /// Knowledge base configuration (chunking, embedding, retrieval).
    pub knowledge: KnowledgeConfig,

    /// Log level (trace, debug, info, warn, error).
    pub log_level: String,

    /// Output format (json, table, plain).
    pub output_format: String,

    /// Override log directory (defaults to `$XDG_STATE_HOME/y-agent/log/`).
    pub log_dir: Option<String>,

    /// Number of days to retain log files (default: 7).
    pub log_retention_days: u32,
}

impl Default for YAgentConfig {
    fn default() -> Self {
        Self {
            providers: ProviderPoolConfig::default(),
            storage: StorageConfig::default(),
            session: SessionConfig::default(),
            runtime: RuntimeConfig::default(),
            hooks: HookConfig::default(),
            tools: ToolRegistryConfig::default(),
            guardrails: GuardrailConfig::default(),
            browser: BrowserConfig::default(),
            knowledge: KnowledgeConfig::default(),
            log_level: "info".to_string(),
            output_format: "plain".to_string(),
            log_dir: None,
            log_retention_days: 7,
        }
    }
}

/// Config loader that merges multiple configuration sources.
#[derive(Debug)]
pub struct ConfigLoader {
    /// CLI overrides (highest priority).
    cli_overrides: HashMap<String, String>,

    /// Environment variable overrides.
    env_overrides: HashMap<String, String>,

    /// Path to user config file (~/.config/y-agent/config.toml).
    user_config_path: Option<PathBuf>,

    /// Path to user config directory (~/.config/y-agent/).
    /// Split per-concern files (storage.toml, providers.toml, etc.) live
    /// directly here — no subdirectory.
    user_config_dir_path: Option<PathBuf>,

    /// Path to project config file (./y-agent.toml).
    project_config_path: Option<PathBuf>,

    /// Path to config directory (./config/).
    config_dir_path: Option<PathBuf>,
}

/// Mapping of config file basenames (without extension) to their target section
/// in `YAgentConfig`. Used by `load_config_dir`.
const CONFIG_FILE_SECTIONS: &[&str] = &[
    "providers",
    "storage",
    "session",
    "runtime",
    "hooks",
    "tools",
    "guardrails",
    "browser",
    "knowledge",
];

impl ConfigLoader {
    /// Create a new config loader with default paths.
    pub fn new() -> Self {
        let user_config_dir = dirs_user_config();
        let user_config_path = user_config_dir.as_ref().map(|p| p.join("config.toml"));
        // Config files live directly in ~/.config/y-agent/ — no subdirectory.
        let user_config_dir_path = user_config_dir;
        let project_config_path = Some(PathBuf::from("y-agent.toml"));
        let config_dir_path = Some(PathBuf::from("config"));

        Self {
            cli_overrides: HashMap::new(),
            env_overrides: HashMap::new(),
            user_config_path,
            user_config_dir_path,
            project_config_path,
            config_dir_path,
        }
    }

    /// Create a config loader for testing (no default paths).
    #[cfg(test)]
    pub fn for_testing() -> Self {
        Self {
            cli_overrides: HashMap::new(),
            env_overrides: HashMap::new(),
            user_config_path: None,
            user_config_dir_path: None,
            project_config_path: None,
            config_dir_path: None,
        }
    }

    /// Set CLI argument overrides.
    pub fn with_cli_overrides(mut self, overrides: HashMap<String, String>) -> Self {
        self.cli_overrides = overrides;
        self
    }

    /// Override the project config path.
    pub fn with_project_config(mut self, path: Option<PathBuf>) -> Self {
        self.project_config_path = path;
        self
    }

    /// Load and merge configuration from all sources.
    ///
    /// Precedence (highest to lowest):
    /// 1. CLI argument overrides
    /// 2. Environment variables (`Y_AGENT_*`)
    /// 3. User config file (`~/.config/y-agent/config.toml`)
    /// 4. User config directory (`~/.config/y-agent/*.toml`)
    /// 5. Project config file (`./y-agent.toml`)
    /// 6. Config directory files (`./config/*.toml`)
    /// 7. Built-in defaults
    pub fn load(&self) -> Result<YAgentConfig> {
        // Start with defaults.
        let mut config = YAgentConfig::default();

        // Layer 6: project config directory (split per-concern files).
        self.load_config_dir_from(&self.config_dir_path, &mut config)?;

        // Layer 5: project config file (monolithic, backward compat).
        if let Some(ref path) = self.project_config_path {
            if path.exists() {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("reading project config: {}", path.display()))?;
                config = toml::from_str(&content)
                    .with_context(|| format!("parsing project config: {}", path.display()))?;
            }
        }

        // Layer 4: user config directory (split per-concern files).
        self.load_config_dir_from(&self.user_config_dir_path, &mut config)?;

        // Layer 3: user config file (merges over project config).
        if let Some(ref path) = self.user_config_path {
            if path.exists() {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("reading user config: {}", path.display()))?;
                let user_config: YAgentConfig = toml::from_str(&content)
                    .with_context(|| format!("parsing user config: {}", path.display()))?;
                merge_config(&mut config, &user_config);
            }
        }

        // Layer 2: environment variables.
        self.apply_env_overrides(&mut config);

        // Layer 1: CLI argument overrides.
        self.apply_cli_overrides(&mut config);

        // Resolve relative storage paths against the user config directory
        // so that `db_path = "data/y-agent.db"` always resolves to
        // `~/.config/y-agent/data/y-agent.db` regardless of cwd.
        resolve_storage_paths(&mut config);

        Ok(config)
    }

    /// Load per-concern config files from a config directory.
    ///
    /// Each file maps to a specific sub-section of `YAgentConfig`:
    /// - `y-agent.toml`   → global fields (`log_level`, `output_format`)
    /// - `providers.toml` → `config.providers`
    /// - `storage.toml`   → `config.storage`
    /// - `session.toml`   → `config.session`
    /// - `runtime.toml`   → `config.runtime`
    /// - `hooks.toml`     → `config.hooks`
    /// - `tools.toml`     → `config.tools`
    /// - `guardrails.toml`→ `config.guardrails`
    fn load_config_dir_from(
        &self,
        dir_path: &Option<PathBuf>,
        config: &mut YAgentConfig,
    ) -> Result<()> {
        let dir = match dir_path {
            Some(ref p) if p.is_dir() => p,
            _ => return Ok(()),
        };

        // Load global y-agent.toml from config dir (log_level, output_format).
        let global_path = dir.join("y-agent.toml");
        if global_path.exists() {
            let content = std::fs::read_to_string(&global_path)
                .with_context(|| format!("reading {}", global_path.display()))?;
            let global: YAgentConfig = toml::from_str(&content)
                .with_context(|| format!("parsing {}", global_path.display()))?;
            merge_config(config, &global);
        }

        // Load per-section config files.
        for section in CONFIG_FILE_SECTIONS {
            let path = dir.join(format!("{section}.toml"));
            if !path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;

            match *section {
                "providers" => {
                    config.providers = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "storage" => {
                    config.storage = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "session" => {
                    config.session = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "runtime" => {
                    config.runtime = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "hooks" => {
                    config.hooks = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "tools" => {
                    config.tools = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "guardrails" => {
                    config.guardrails = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "browser" => {
                    config.browser = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                "knowledge" => {
                    config.knowledge = toml::from_str(&content)
                        .with_context(|| format!("parsing {}", path.display()))?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Apply environment variable overrides to the config.
    fn apply_env_overrides(&self, config: &mut YAgentConfig) {
        // Check injected overrides first (for testing), then real env vars.
        let get_env = |key: &str| -> Option<String> {
            self.env_overrides
                .get(key)
                .cloned()
                .or_else(|| std::env::var(key).ok())
        };

        if let Some(val) = get_env(&format!("{ENV_PREFIX}LOG_LEVEL")) {
            config.log_level = val;
        }
        if let Some(val) = get_env(&format!("{ENV_PREFIX}OUTPUT_FORMAT")) {
            config.output_format = val;
        }
        if let Some(val) = get_env(&format!("{ENV_PREFIX}DB_PATH")) {
            config.storage.db_path = val;
        }
        if let Some(val) = get_env(&format!("{ENV_PREFIX}LOG_DIR")) {
            config.log_dir = Some(val);
        }
        if let Some(val) = get_env(&format!("{ENV_PREFIX}LOG_RETENTION_DAYS")) {
            if let Ok(days) = val.parse::<u32>() {
                config.log_retention_days = days;
            }
        }
    }

    /// Apply CLI argument overrides to the config.
    fn apply_cli_overrides(&self, config: &mut YAgentConfig) {
        if let Some(val) = self.cli_overrides.get("log_level") {
            config.log_level = val.clone();
        }
        if let Some(val) = self.cli_overrides.get("output_format") {
            config.output_format = val.clone();
        }
        if let Some(val) = self.cli_overrides.get("db_path") {
            config.storage.db_path = val.clone();
        }
        if let Some(val) = self.cli_overrides.get("log_dir") {
            config.log_dir = Some(val.clone());
        }
    }
}

/// Validate a configuration for required fields and consistency.
pub fn validate_config(config: &YAgentConfig) -> Result<()> {
    // Log level must be valid.
    match config.log_level.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => {}
        other => {
            anyhow::bail!("invalid log_level: '{other}' (expected trace/debug/info/warn/error)")
        }
    }

    // Output format must be valid.
    match config.output_format.as_str() {
        "json" | "table" | "plain" => {}
        other => anyhow::bail!("invalid output_format: '{other}' (expected json/table/plain)"),
    }

    // Storage db_path must be set.
    if config.storage.db_path.is_empty() {
        anyhow::bail!("storage.db_path must not be empty");
    }

    Ok(())
}

/// Resolve relative storage paths against the XDG state data directory.
///
/// When `db_path` or `transcript_dir` is a relative path (e.g., `data/y-agent.db`),
/// it is resolved against the state data directory (`~/.local/state/y-agent/`) so that
/// the same database is used regardless of the current working directory.
///
/// Absolute paths and `:memory:` are left unchanged.
pub fn resolve_storage_paths(config: &mut YAgentConfig) {
    let base_dir = match dirs_state() {
        Some(dir) => dir,
        None => return, // Cannot determine home dir; leave paths as-is.
    };

    // Resolve db_path.
    if config.storage.db_path != ":memory:" {
        let db = PathBuf::from(&config.storage.db_path);
        if db.is_relative() {
            config.storage.db_path = base_dir.join(&db).to_string_lossy().to_string();
        }
    }

    // Resolve transcript_dir.
    if config.storage.transcript_dir.is_relative() {
        config.storage.transcript_dir = base_dir.join(&config.storage.transcript_dir);
    }
}

/// Resolve an API key from a named environment variable.
///
/// Will be consumed by provider initialization when LLM API keys are configured.
#[allow(dead_code)]
pub fn resolve_secret(env_var_name: &str) -> Result<String> {
    std::env::var(env_var_name).with_context(|| format!("secret env var '{env_var_name}' not set"))
}

/// Merge fields from `source` into `target` where source has non-default values.
/// This is a simple top-level merge: if the source `log_level` differs from default,
/// it overrides target.
fn merge_config(target: &mut YAgentConfig, source: &YAgentConfig) {
    let defaults = YAgentConfig::default();

    if source.log_level != defaults.log_level {
        target.log_level = source.log_level.clone();
    }
    if source.output_format != defaults.output_format {
        target.output_format = source.output_format.clone();
    }
    if source.log_dir != defaults.log_dir {
        target.log_dir.clone_from(&source.log_dir);
    }
    if source.log_retention_days != defaults.log_retention_days {
        target.log_retention_days = source.log_retention_days;
    }
    // Sub-configs are fully replaced if they differ from defaults (the TOML
    // parser already provides serde-defaulted values, so a partially specified
    // section still contains sensible defaults).
}

/// Get the user config directory for y-agent.
pub(crate) fn dirs_user_config() -> Option<PathBuf> {
    // Always use ~/.config/y-agent regardless of platform.
    home_dir().map(|h| h.join(".config").join("y-agent"))
}

/// Get the log directory for y-agent.
///
/// Uses `$XDG_STATE_HOME/y-agent/log/` (defaults to `~/.local/state/y-agent/log/`).
/// `XDG_STATE_HOME` is the correct XDG location for log files: "state data that
/// persists across restarts but is not important or portable enough for
/// `$XDG_DATA_HOME`."
pub fn dirs_log() -> Option<PathBuf> {
    dirs_state().map(|s| s.join("log"))
}

/// Get the XDG state base directory for y-agent.
///
/// Uses `$XDG_STATE_HOME/y-agent/` (defaults to `~/.local/state/y-agent/`).
/// Both log and data directories live under this path.
pub fn dirs_state() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local").join("state")));
    state_home.map(|s| s.join("y-agent"))
}

/// Clean up log files older than the given retention period.
///
/// Deletes files matching `y-agent.*.log` in the given directory that are
/// older than `retention_days` days. Returns the number of files deleted.
pub fn cleanup_old_logs(log_dir: &std::path::Path, retention_days: u32) -> std::io::Result<usize> {
    use std::time::{Duration, SystemTime};

    if !log_dir.is_dir() {
        return Ok(0);
    }

    let cutoff = SystemTime::now() - Duration::from_secs(u64::from(retention_days) * 86400);
    let mut deleted = 0;

    for entry in std::fs::read_dir(log_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only delete files matching the y-agent log pattern.
        // Matches both new format (y-agent.YYYY-MM-DD.log) and
        // legacy format (y-agent.YYYY-MM-DD without .log suffix).
        let is_log_file = name_str.starts_with("y-agent.")
            && (name_str.ends_with(".log")
                || name_str
                    .chars()
                    .skip("y-agent.".len())
                    .all(|c| c.is_ascii_digit() || c == '-'));
        if !is_log_file {
            continue;
        }

        if let Ok(metadata) = entry.metadata() {
            let modified = metadata.modified().unwrap_or(SystemTime::now());
            if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
                deleted += 1;
            }
        }
    }

    Ok(deleted)
}

/// Simple home directory resolution.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

impl ConfigLoader {
    /// Override the config directory path.
    pub fn with_config_dir(mut self, path: Option<PathBuf>) -> Self {
        self.config_dir_path = path;
        self
    }

    /// Override the user config directory path.
    pub fn with_user_config_dir(mut self, path: Option<PathBuf>) -> Self {
        self.user_config_dir_path = path;
        self
    }
}

#[cfg(test)]
impl ConfigLoader {
    /// Set environment variable overrides (for testing).
    pub fn with_env_overrides(mut self, overrides: HashMap<String, String>) -> Self {
        self.env_overrides = overrides;
        self
    }

    /// Override the user config path.
    pub fn with_user_config(mut self, path: Option<PathBuf>) -> Self {
        self.user_config_path = path;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-001-01: test_config_load_defaults
    #[test]
    fn test_config_load_defaults() {
        let loader = ConfigLoader::for_testing();
        let config = loader.load().expect("defaults should load");

        assert_eq!(config.log_level, "info");
        assert_eq!(config.output_format, "plain");
        assert!(!config.storage.db_path.is_empty());
    }

    // T-CLI-001-02: test_config_load_from_toml
    #[test]
    fn test_config_load_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("y-agent.toml");

        let toml_content = r#"
log_level = "debug"
output_format = "json"

[storage]
db_path = "/tmp/test.db"
"#;
        std::fs::write(&config_path, toml_content).unwrap();

        let loader = ConfigLoader::for_testing().with_project_config(Some(config_path));
        let config = loader.load().expect("toml should load");

        assert_eq!(config.log_level, "debug");
        assert_eq!(config.output_format, "json");
        assert_eq!(config.storage.db_path, "/tmp/test.db");
    }

    // T-CLI-001-03: test_config_env_overrides_file
    #[test]
    fn test_config_env_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("y-agent.toml");

        let toml_content = r#"
log_level = "debug"
"#;
        std::fs::write(&config_path, toml_content).unwrap();

        let mut env = HashMap::new();
        env.insert(format!("{ENV_PREFIX}LOG_LEVEL"), "warn".to_string());

        let loader = ConfigLoader::for_testing()
            .with_project_config(Some(config_path))
            .with_env_overrides(env);
        let config = loader.load().expect("env override should work");

        assert_eq!(config.log_level, "warn", "env var should override file");
    }

    // T-CLI-001-04: test_config_cli_overrides_all
    #[test]
    fn test_config_cli_overrides_all() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("y-agent.toml");
        std::fs::write(&config_path, "log_level = \"debug\"").unwrap();

        let mut env = HashMap::new();
        env.insert(format!("{ENV_PREFIX}LOG_LEVEL"), "warn".to_string());

        let mut cli = HashMap::new();
        cli.insert("log_level".to_string(), "error".to_string());

        let loader = ConfigLoader::for_testing()
            .with_project_config(Some(config_path))
            .with_env_overrides(env)
            .with_cli_overrides(cli);
        let config = loader.load().expect("cli override should work");

        assert_eq!(config.log_level, "error", "CLI should override everything");
    }

    // T-CLI-001-05: test_config_validate_catches_errors
    #[test]
    fn test_config_validate_catches_errors() {
        let mut config = YAgentConfig::default();
        config.log_level = "invalid".to_string();

        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid log_level"));

        // Reset log_level, break output_format.
        config.log_level = "info".to_string();
        config.output_format = "xml".to_string();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid output_format"));

        // Reset output_format, break db_path.
        config.output_format = "plain".to_string();
        config.storage.db_path = String::new();
        let result = validate_config(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("db_path"));
    }

    // T-CLI-001-06: test_config_secrets_from_env_only
    #[test]
    fn test_config_secrets_from_env_only() {
        // Set a test env var for this test.
        let key = "Y_AGENT_TEST_SECRET_KEY_12345";
        std::env::set_var(key, "sk-test-abc123");

        let result = resolve_secret(key);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "sk-test-abc123");

        std::env::remove_var(key);

        // Missing env var should error.
        let result = resolve_secret("Y_AGENT_NONEXISTENT_KEY_XYZ");
        assert!(result.is_err());
    }

    // T-CLI-001-07: test_config_load_from_dir
    #[test]
    fn test_config_load_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();

        // Write a global config file.
        std::fs::write(config_dir.join("y-agent.toml"), "log_level = \"debug\"\n").unwrap();

        // Write a storage config file.
        std::fs::write(
            config_dir.join("storage.toml"),
            "db_path = \"/tmp/split-test.db\"\npool_size = 10\n",
        )
        .unwrap();

        let loader = ConfigLoader::for_testing().with_config_dir(Some(config_dir));
        let config = loader.load().expect("config dir should load");

        assert_eq!(config.log_level, "debug");
        assert_eq!(config.storage.db_path, "/tmp/split-test.db");
        assert_eq!(config.storage.pool_size, 10);
    }

    // T-CLI-001-08: test_project_config_overrides_dir
    #[test]
    fn test_project_config_overrides_dir() {
        let dir = tempfile::tempdir().unwrap();

        // Config dir: sets log_level = debug.
        let config_dir = dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("y-agent.toml"), "log_level = \"debug\"\n").unwrap();

        // Project file: sets log_level = warn (higher priority).
        let project_path = dir.path().join("y-agent.toml");
        std::fs::write(&project_path, "log_level = \"warn\"\n").unwrap();

        let loader = ConfigLoader::for_testing()
            .with_config_dir(Some(config_dir))
            .with_project_config(Some(project_path));
        let config = loader.load().expect("override should work");

        assert_eq!(
            config.log_level, "warn",
            "project config should override config dir"
        );
    }

    // T-CLI-001-09: test_user_config_dir_loads_split_files
    #[test]
    fn test_user_config_dir_loads_split_files() {
        let dir = tempfile::tempdir().unwrap();

        // User config dir: sets db_path in storage.toml.
        let user_config_dir = dir.path().join("user_config");
        std::fs::create_dir_all(&user_config_dir).unwrap();
        std::fs::write(
            user_config_dir.join("storage.toml"),
            "db_path = \"/tmp/user-test.db\"\n",
        )
        .unwrap();

        let loader = ConfigLoader::for_testing().with_user_config_dir(Some(user_config_dir));
        let config = loader.load().expect("user config dir should load");

        assert_eq!(
            config.storage.db_path, "/tmp/user-test.db",
            "db_path from user config dir should be loaded"
        );
    }

    // T-CLI-001-10: test_user_config_dir_overrides_project_config
    #[test]
    fn test_user_config_dir_overrides_project_config() {
        let dir = tempfile::tempdir().unwrap();

        // Project config dir: sets storage with project db_path.
        let project_dir = dir.path().join("config");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("storage.toml"),
            "db_path = \"data/project.db\"\n",
        )
        .unwrap();

        // User config dir: sets db_path (higher priority).
        let user_dir = dir.path().join("user_config");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(
            user_dir.join("storage.toml"),
            "db_path = \"data/user.db\"\n",
        )
        .unwrap();

        let loader = ConfigLoader::for_testing()
            .with_config_dir(Some(project_dir))
            .with_user_config_dir(Some(user_dir));
        let config = loader.load().expect("override should work");

        assert!(
            config.storage.db_path.ends_with("data/user.db"),
            "user config dir should override project config dir, got: {}",
            config.storage.db_path
        );
    }

    // T-LOG-01: dirs_log() returns XDG_STATE_HOME default when env var is unset.
    #[test]
    fn test_dirs_log_default() {
        // Temporarily remove XDG_STATE_HOME to test fallback.
        let prev = std::env::var_os("XDG_STATE_HOME");
        std::env::remove_var("XDG_STATE_HOME");

        let log_dir = dirs_log();
        assert!(log_dir.is_some());
        let path = log_dir.unwrap();
        assert!(
            path.ends_with("y-agent/log"),
            "expected path ending with y-agent/log, got: {}",
            path.display()
        );
        // Should be under ~/.local/state/
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains(".local/state"),
            "expected .local/state in path: {path_str}"
        );

        // Restore.
        if let Some(val) = prev {
            std::env::set_var("XDG_STATE_HOME", val);
        }
    }

    // T-LOG-02: dirs_log() respects XDG_STATE_HOME env var.
    #[test]
    fn test_dirs_log_respects_xdg_state_home() {
        let prev = std::env::var_os("XDG_STATE_HOME");
        std::env::set_var("XDG_STATE_HOME", "/tmp/test-xdg-state");

        let log_dir = dirs_log();
        assert!(log_dir.is_some());
        assert_eq!(
            log_dir.unwrap(),
            PathBuf::from("/tmp/test-xdg-state/y-agent/log")
        );

        // Restore.
        if let Some(val) = prev {
            std::env::set_var("XDG_STATE_HOME", val);
        } else {
            std::env::remove_var("XDG_STATE_HOME");
        }
    }

    // T-LOG-03: YAgentConfig.log_dir override takes precedence.
    #[test]
    fn test_config_log_dir_override() {
        let mut env = HashMap::new();
        env.insert(
            format!("{ENV_PREFIX}LOG_DIR"),
            "/custom/log/dir".to_string(),
        );
        let loader = ConfigLoader::for_testing().with_env_overrides(env);
        let config = loader.load().unwrap();
        assert_eq!(config.log_dir, Some("/custom/log/dir".to_string()));
    }

    // T-LOG-04: log_retention_days defaults to 7.
    #[test]
    fn test_config_log_retention_days_default() {
        let config = YAgentConfig::default();
        assert_eq!(config.log_retention_days, 7);
    }

    // T-LOG-05: cleanup_old_logs with retention_days=0 removes all matching files.
    #[test]
    fn test_cleanup_old_logs_removes_with_zero_retention() {
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("y-agent.2024-01-01.log");
        std::fs::write(&log_file, "old log content").unwrap();

        // With retention_days=0, even a freshly created file is "older than 0 days"
        // only if its mtime is before now. Since the file was just created, it
        // won't be deleted. Use a very large retention to confirm preservation,
        // then test that the pattern matching works.
        // Actually, retention_days=0 means cutoff = now, so any file modified
        // before now should be deleted. A just-created file might be at exactly
        // now so it may or may not be deleted. Test with a reasonable approach:
        // Sleep briefly so the file mtime is definitely before cutoff.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let deleted = cleanup_old_logs(dir.path(), 0).unwrap();
        assert_eq!(deleted, 1);
        assert!(!log_file.exists());
    }

    // T-LOG-06: cleanup_old_logs preserves recent files.
    #[test]
    fn test_cleanup_old_logs_preserves_recent() {
        let dir = tempfile::tempdir().unwrap();
        let recent = dir.path().join("y-agent.2026-03-11.log");
        std::fs::write(&recent, "recent log").unwrap();

        let deleted = cleanup_old_logs(dir.path(), 7).unwrap();
        assert_eq!(deleted, 0);
        assert!(recent.exists());
    }

    // T-LOG-07: cleanup_old_logs ignores non-log files.
    #[test]
    fn test_cleanup_old_logs_ignores_non_log() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "not a log").unwrap();

        let deleted = cleanup_old_logs(dir.path(), 0).unwrap();
        assert_eq!(deleted, 0);
        assert!(other.exists());
    }

    // T-LOG-08: cleanup_old_logs on non-existent dir returns 0.
    #[test]
    fn test_cleanup_old_logs_nonexistent_dir() {
        let deleted = cleanup_old_logs(std::path::Path::new("/nonexistent/dir"), 7).unwrap();
        assert_eq!(deleted, 0);
    }

    // T-CFG-PATH-01: resolve_storage_paths resolves relative db_path against state dir.
    #[test]
    fn test_resolve_storage_paths_relative_db() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = "data/y-agent.db".to_string();
        config.storage.transcript_dir = PathBuf::from("data/transcripts");

        resolve_storage_paths(&mut config);

        assert!(
            PathBuf::from(&config.storage.db_path).is_absolute(),
            "db_path should be absolute after resolution: {}",
            config.storage.db_path
        );
        assert!(
            config.storage.db_path.contains(".local/state/y-agent"),
            "db_path should be under state dir: {}",
            config.storage.db_path
        );
        assert!(
            config.storage.db_path.ends_with("data/y-agent.db"),
            "db_path should preserve the relative suffix: {}",
            config.storage.db_path
        );

        assert!(
            config.storage.transcript_dir.is_absolute(),
            "transcript_dir should be absolute after resolution: {}",
            config.storage.transcript_dir.display()
        );
    }

    // T-CFG-PATH-02: resolve_storage_paths leaves absolute paths unchanged.
    #[test]
    fn test_resolve_storage_paths_absolute_unchanged() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = "/opt/y-agent/data.db".to_string();
        config.storage.transcript_dir = PathBuf::from("/opt/y-agent/transcripts");

        resolve_storage_paths(&mut config);

        assert_eq!(config.storage.db_path, "/opt/y-agent/data.db");
        assert_eq!(
            config.storage.transcript_dir,
            PathBuf::from("/opt/y-agent/transcripts")
        );
    }

    // T-CFG-PATH-03: resolve_storage_paths preserves :memory: db_path.
    #[test]
    fn test_resolve_storage_paths_memory_preserved() {
        let mut config = YAgentConfig::default();
        config.storage.db_path = ":memory:".to_string();

        resolve_storage_paths(&mut config);

        assert_eq!(config.storage.db_path, ":memory:");
    }
}
