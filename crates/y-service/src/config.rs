//! Service configuration: the subset of config needed by the service layer.
//!
//! `ConfigLoader` and CLI-specific fields (`log_level`, `output_format`, `log_dir`)
//! stay in `y-cli`. This struct holds only the domain-relevant configuration
//! that `ServiceContainer` needs for construction.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::warn;

use y_guardrails::GuardrailConfig;
use y_hooks::HookConfig;
use y_provider::ProviderPoolConfig;
use y_runtime::RuntimeConfig;
use y_session::SessionConfig;
use y_storage::StorageConfig;
use y_tools::ToolRegistryConfig;

/// Configuration for constructing a [`ServiceContainer`](crate::ServiceContainer).
///
/// Contains all domain-relevant sub-configs. Presentation-specific fields
/// (log level, output format, log dir) are NOT included — they belong
/// in the presentation layer.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ServiceConfig {
    /// Provider pool configuration.
    pub providers: ProviderPoolConfig,

    /// Storage configuration (`SQLite` + `PostgreSQL`).
    pub storage: StorageConfig,

    /// Session lifecycle configuration.
    pub session: SessionConfig,

    /// Tool execution runtime configuration.
    pub runtime: RuntimeConfig,

    /// Hook system configuration.
    pub hooks: HookConfig,

    /// Tool registry configuration.
    pub tools: ToolRegistryConfig,

    /// Guardrail/safety configuration.
    pub guardrails: GuardrailConfig,
}

/// Config file basenames (without `.toml` extension) mapping to `ServiceConfig` fields.
const CONFIG_SECTIONS: &[&str] = &[
    "providers",
    "storage",
    "session",
    "runtime",
    "hooks",
    "tools",
    "guardrails",
];

impl ServiceConfig {
    /// Load a `ServiceConfig` by reading per-section TOML files from `config_dir`.
    ///
    /// Reads `providers.toml`, `storage.toml`, `session.toml`, `runtime.toml`,
    /// `hooks.toml`, `tools.toml`, `guardrails.toml` from the given directory.
    /// Missing files are silently skipped (defaults apply). After loading,
    /// resolves relative storage paths against `state_dir` if provided.
    pub fn load_from_directory(config_dir: &Path, state_dir: Option<&Path>) -> Self {
        let mut config = Self::default();

        if !config_dir.is_dir() {
            warn!(
                path = %config_dir.display(),
                "Config directory not found; using defaults"
            );
            return config;
        }

        for section in CONFIG_SECTIONS {
            let path = config_dir.join(format!("{section}.toml"));
            if !path.exists() {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to read config file; skipping"
                    );
                    continue;
                }
            };

            match *section {
                "providers" => match toml::from_str(&content) {
                    Ok(v) => config.providers = v,
                    Err(e) => warn!(file = "providers.toml", error = %e, "Parse error"),
                },
                "storage" => match toml::from_str(&content) {
                    Ok(v) => config.storage = v,
                    Err(e) => warn!(file = "storage.toml", error = %e, "Parse error"),
                },
                "session" => match toml::from_str(&content) {
                    Ok(v) => config.session = v,
                    Err(e) => warn!(file = "session.toml", error = %e, "Parse error"),
                },
                "runtime" => match toml::from_str(&content) {
                    Ok(v) => config.runtime = v,
                    Err(e) => warn!(file = "runtime.toml", error = %e, "Parse error"),
                },
                "hooks" => match toml::from_str(&content) {
                    Ok(v) => config.hooks = v,
                    Err(e) => warn!(file = "hooks.toml", error = %e, "Parse error"),
                },
                "tools" => match toml::from_str(&content) {
                    Ok(v) => config.tools = v,
                    Err(e) => warn!(file = "tools.toml", error = %e, "Parse error"),
                },
                "guardrails" => match toml::from_str(&content) {
                    Ok(v) => config.guardrails = v,
                    Err(e) => warn!(file = "guardrails.toml", error = %e, "Parse error"),
                },
                _ => {}
            }
        }

        if let Some(base_dir) = state_dir {
            config.resolve_storage_paths(base_dir);
        }

        config
    }

    /// Resolve relative `db_path` and `transcript_dir` against a base directory.
    ///
    /// Leaves absolute paths and the `:memory:` sentinel unchanged.
    pub fn resolve_storage_paths(&mut self, base_dir: &Path) {
        if self.storage.db_path != ":memory:" {
            let db = PathBuf::from(&self.storage.db_path);
            if db.is_relative() {
                self.storage.db_path = base_dir.join(&db).to_string_lossy().to_string();
            }
        }

        if self.storage.transcript_dir.is_relative() {
            self.storage.transcript_dir = base_dir.join(&self.storage.transcript_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_from_empty_directory_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let config = ServiceConfig::load_from_directory(dir.path(), None);
        // Should match ServiceConfig::default() in all fields.
        assert_eq!(config.storage.db_path, "data/y-agent.db");
        assert!(config.providers.providers.is_empty());
    }

    #[test]
    fn load_from_nonexistent_directory_returns_defaults() {
        let config = ServiceConfig::load_from_directory(Path::new("/nonexistent/path"), None);
        assert_eq!(config.storage.db_path, "data/y-agent.db");
    }

    #[test]
    fn load_valid_providers_toml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("providers.toml"),
            r#"
[[providers]]
id = "test-provider"
provider_type = "openai"
model = "gpt-4"
api_key_env = "OPENAI_API_KEY"
"#,
        )
        .unwrap();

        let config = ServiceConfig::load_from_directory(dir.path(), None);
        assert_eq!(config.providers.providers.len(), 1);
        assert_eq!(config.providers.providers[0].id, "test-provider");
    }

    #[test]
    fn malformed_toml_uses_default_for_that_section() {
        let dir = TempDir::new().unwrap();
        // Write valid storage config.
        std::fs::write(
            dir.path().join("storage.toml"),
            r#"db_path = "/tmp/test.db""#,
        )
        .unwrap();
        // Write invalid providers config.
        std::fs::write(
            dir.path().join("providers.toml"),
            "this is not valid toml {{{}}}",
        )
        .unwrap();

        let config = ServiceConfig::load_from_directory(dir.path(), None);
        // Storage should be loaded correctly.
        assert_eq!(config.storage.db_path, "/tmp/test.db");
        // Providers should fall back to default (empty).
        assert!(config.providers.providers.is_empty());
    }

    #[test]
    fn resolve_storage_paths_resolves_relative() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = "data/y-agent.db".to_string();
        config.storage.transcript_dir = PathBuf::from("data/transcripts");

        config.resolve_storage_paths(Path::new("/home/user/.local/state/y-agent"));

        assert_eq!(
            config.storage.db_path,
            "/home/user/.local/state/y-agent/data/y-agent.db"
        );
        assert_eq!(
            config.storage.transcript_dir,
            PathBuf::from("/home/user/.local/state/y-agent/data/transcripts")
        );
    }

    #[test]
    fn resolve_storage_paths_leaves_absolute_unchanged() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = "/absolute/path/db.sqlite".to_string();
        config.storage.transcript_dir = PathBuf::from("/absolute/transcripts");

        config.resolve_storage_paths(Path::new("/home/user/.local/state/y-agent"));

        assert_eq!(config.storage.db_path, "/absolute/path/db.sqlite");
        assert_eq!(
            config.storage.transcript_dir,
            PathBuf::from("/absolute/transcripts")
        );
    }

    #[test]
    fn resolve_storage_paths_leaves_memory_unchanged() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = ":memory:".to_string();

        config.resolve_storage_paths(Path::new("/home/user/.local/state/y-agent"));

        assert_eq!(config.storage.db_path, ":memory:");
    }
}

