//! Service configuration: the subset of config needed by the service layer.
//!
//! `ConfigLoader` and CLI-specific fields (`log_level`, `output_format`, `log_dir`)
//! stay in `y-cli`. This struct holds only the domain-relevant configuration
//! that `ServiceContainer` needs for construction.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::warn;

use y_browser::BrowserConfig;
use y_context::PruningConfig;
#[cfg(feature = "langfuse")]
use y_diagnostics::langfuse::LangfuseConfig;
use y_guardrails::GuardrailConfig;
use y_hooks::HookConfig;
use y_knowledge::config::KnowledgeConfig;
use y_provider::ProviderPoolConfig;
use y_runtime::RuntimeConfig;
use y_session::SessionConfig;
use y_storage::StorageConfig;
use y_tools::ToolRegistryConfig;

/// Combined struct for deserializing `session.toml` which contains both
/// session configuration and a nested `[pruning]` section.
#[derive(Debug, Clone, Deserialize)]
struct SessionFileConfig {
    #[serde(flatten)]
    session: SessionConfig,
    #[serde(default)]
    pruning: PruningConfig,
}

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

    /// Storage configuration (`SQLite`).
    pub storage: StorageConfig,

    /// Session lifecycle configuration.
    pub session: SessionConfig,

    /// Tool execution runtime configuration.
    pub runtime: RuntimeConfig,

    /// Hook system configuration.
    pub hooks: HookConfig,

    /// Tool registry configuration.
    pub tools: ToolRegistryConfig,

    /// Guardrail/security configuration.
    pub guardrails: GuardrailConfig,

    /// Browser (CDP) configuration.
    pub browser: BrowserConfig,

    /// Knowledge base configuration (chunking, embedding, retrieval).
    pub knowledge: KnowledgeConfig,

    /// Context pruning configuration (strategies, thresholds).
    /// Loaded from the `[pruning]` section in `session.toml`.
    pub pruning: PruningConfig,

    /// Path to the user prompts override directory (`~/.config/y-agent/prompts/`).
    /// When set, prompt files here take priority over compiled-in defaults.
    #[serde(skip)]
    pub prompts_dir: Option<PathBuf>,

    /// Path to the skills store directory (`~/.config/y-agent/skills/`).
    /// When set, `InjectSkills` reads skill content from here.
    #[serde(skip)]
    pub skills_dir: Option<PathBuf>,

    /// Path to the bot persona directory (`~/.config/y-agent/persona/`).
    /// When set and containing `persona.toml` with `enabled = true`,
    /// `BotService` uses persona-aware prompting.
    #[serde(skip)]
    pub persona_dir: Option<PathBuf>,

    /// Langfuse export configuration.
    #[cfg(feature = "langfuse")]
    #[serde(default)]
    pub langfuse: LangfuseConfig,
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
    "browser",
    "knowledge",
    "langfuse",
];

impl ServiceConfig {
    /// Load a `ServiceConfig` by reading per-section TOML files from `config_dir`.
    ///
    /// Reads `providers.toml`, `storage.toml`, `session.toml`, `runtime.toml`,
    /// `hooks.toml`, `tools.toml`, `guardrails.toml` from the given directory.
    /// Missing files are silently skipped (defaults apply). After loading,
    /// resolves relative storage paths against `state_dir` if provided.
    pub fn load_from_directory(config_dir: &Path, state_dir: Option<&Path>) -> Self {
        Self::load_from_directory_with_diagnostics(config_dir, state_dir).0
    }

    /// Like [`Self::load_from_directory`] but also returns non-fatal parse
    /// diagnostics: section files that failed to parse, individual
    /// `[[providers]]` blocks that were skipped, and capability values that
    /// were dropped.
    ///
    /// Release builds without a `tracing_subscriber` installed have no
    /// other channel for these warnings — the calling presentation layer
    /// (e.g. the Tauri GUI) is expected to persist them to a user-visible
    /// location so config typos can be diagnosed without rebuilding the app.
    pub fn load_from_directory_with_diagnostics(
        config_dir: &Path,
        state_dir: Option<&Path>,
    ) -> (Self, Vec<String>) {
        let mut config = Self::default();
        let mut errors: Vec<String> = Vec::new();

        // Drain anything sitting in the provider thread-local from a previous
        // call. This prevents stale diagnostics from bleeding in if a caller
        // never picks them up.
        let _ = y_provider::drain_config_load_errors();

        // Set prompts override directory to <config_dir>/prompts/.
        let prompts_dir = config_dir.join("prompts");
        if prompts_dir.is_dir() {
            config.prompts_dir = Some(prompts_dir);
        }

        // Set skills store directory to <config_dir>/skills/.
        let skills_dir = config_dir.join("skills");
        config.skills_dir = Some(skills_dir);

        // Set persona directory to <config_dir>/persona/.
        let persona_dir = config_dir.join("persona");
        config.persona_dir = Some(persona_dir);

        if !config_dir.is_dir() {
            warn!(
                path = %config_dir.display(),
                "Config directory not found; using defaults"
            );
            errors.push(format!(
                "config directory not found: {} -- using defaults",
                config_dir.display()
            ));
            return (config, errors);
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
                    errors.push(format!("{}: failed to read: {e}", path.display()));
                    continue;
                }
            };

            // Helper: parse one section, log + record on failure.
            macro_rules! parse_section {
                ($field:expr, $ty:ty) => {
                    match toml::from_str::<$ty>(&content) {
                        Ok(v) => $field = v,
                        Err(e) => {
                            warn!(file = format!("{section}.toml").as_str(), error = %e, "Parse error");
                            errors.push(format!("{section}.toml: parse error: {e}"));
                        }
                    }
                };
            }

            match *section {
                "providers" => parse_section!(config.providers, ProviderPoolConfig),
                "storage" => parse_section!(config.storage, StorageConfig),
                "session" => match toml::from_str::<SessionFileConfig>(&content) {
                    Ok(v) => {
                        config.session = v.session;
                        config.pruning = v.pruning;
                    }
                    Err(e) => {
                        warn!(file = "session.toml", error = %e, "Parse error");
                        errors.push(format!("session.toml: parse error: {e}"));
                    }
                },
                "runtime" => parse_section!(config.runtime, RuntimeConfig),
                "hooks" => parse_section!(config.hooks, HookConfig),
                "tools" => parse_section!(config.tools, ToolRegistryConfig),
                "guardrails" => parse_section!(config.guardrails, GuardrailConfig),
                "browser" => parse_section!(config.browser, BrowserConfig),
                "knowledge" => parse_section!(config.knowledge, KnowledgeConfig),
                #[cfg(feature = "langfuse")]
                "langfuse" => parse_section!(config.langfuse, LangfuseConfig),
                _ => {}
            }
        }

        // Pick up provider-level non-fatal diagnostics (per-block skips and
        // dropped capabilities) recorded by lenient deserializers in
        // y-provider during the per-section parses above.
        errors.extend(y_provider::drain_config_load_errors());

        if let Some(base_dir) = state_dir {
            config.resolve_storage_paths(base_dir);
        }

        (config, errors)
    }

    /// Resolve relative `db_path` and `transcript_dir` against a base directory.
    /// Also expands `~` to the user's home directory.
    ///
    /// Leaves absolute paths and the `:memory:` sentinel unchanged.
    pub fn resolve_storage_paths(&mut self, base_dir: &Path) {
        if self.storage.db_path != ":memory:" {
            let db_path = expand_tilde(&self.storage.db_path);
            let db = PathBuf::from(&db_path);
            if db.is_relative() {
                self.storage.db_path = base_dir.join(&db).to_string_lossy().to_string();
            } else {
                self.storage.db_path = db_path;
            }
        }

        let transcript_str = expand_tilde(&self.storage.transcript_dir.to_string_lossy());
        let transcript_dir = PathBuf::from(&transcript_str);
        if transcript_dir.is_relative() {
            self.storage.transcript_dir = base_dir.join(&transcript_dir);
        } else {
            self.storage.transcript_dir = transcript_dir;
        }

        // Resolve runtime paths
        self.runtime.ssh.private_key_path = self
            .runtime
            .ssh
            .private_key_path
            .as_deref()
            .map(expand_tilde);
        self.runtime.ssh.known_hosts_path = self
            .runtime
            .ssh
            .known_hosts_path
            .as_deref()
            .map(expand_tilde);
        self.runtime.python_venv.working_dir = expand_tilde(&self.runtime.python_venv.working_dir);
        self.runtime.bun_venv.working_dir = expand_tilde(&self.runtime.bun_venv.working_dir);
    }
}

/// Expand `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    fn resolve_home() -> String {
        fn non_empty(name: &str) -> Option<std::ffi::OsString> {
            std::env::var_os(name).filter(|v| !v.is_empty())
        }
        let primary = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let fallback = if cfg!(windows) { "HOME" } else { "USERPROFILE" };
        non_empty(primary)
            .or_else(|| non_empty(fallback))
            .map_or_else(
                || ".".to_string(),
                |os| PathBuf::from(os).to_string_lossy().into_owned(),
            )
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        format!("{}/{stripped}", resolve_home())
    } else if path == "~" {
        resolve_home()
    } else {
        path.to_string()
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
    fn diagnostics_surface_per_provider_skip_and_unknown_capability() {
        // Models the exact failure mode that emptied a user's GUI model
        // dropdown in production: a single capability typo in one
        // [[providers]] block (`"vison"` instead of `"vision"`). Before the
        // lenient deserializers the whole file failed to parse and the
        // provider pool was silently empty. Now the typo's entry is dropped,
        // siblings survive, and a human-readable diagnostic is returned for
        // the GUI to surface.
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("providers.toml"),
            r#"
[[providers]]
id = "good"
provider_type = "openai-compat"
model = "m"
api_key = "k"
capabilities = ["text", "vison"]

[[providers]]
# Missing required `id`.
provider_type = "openai-compat"
model = "m"
api_key = "k"
"#,
        )
        .unwrap();

        let (config, errors) =
            ServiceConfig::load_from_directory_with_diagnostics(dir.path(), None);

        // The well-formed provider must survive both the bad-capability
        // entry and the malformed sibling block.
        assert_eq!(config.providers.providers.len(), 1);
        assert_eq!(config.providers.providers[0].id, "good");
        assert_eq!(
            config.providers.providers[0].capabilities,
            vec![y_core::provider::ProviderCapability::Text],
        );

        // Both kinds of diagnostic must be surfaced for the GUI.
        assert!(
            errors.iter().any(|e| e.contains("vison")),
            "expected a diagnostic for the dropped capability: {errors:?}",
        );
        assert!(
            errors.iter().any(|e| e.contains("skipping provider")),
            "expected a diagnostic for the skipped provider block: {errors:?}",
        );
    }

    #[test]
    fn diagnostics_are_drained_between_calls() {
        // Each call to load_from_directory_with_diagnostics must start
        // from a clean slate so stale errors from a prior load don't bleed
        // into a fresh report.
        let dir1 = TempDir::new().unwrap();
        std::fs::write(
            dir1.path().join("providers.toml"),
            r#"
[[providers]]
id = "x"
provider_type = "openai-compat"
model = "m"
api_key = "k"
capabilities = ["nonsense"]
"#,
        )
        .unwrap();
        let (_, errors1) = ServiceConfig::load_from_directory_with_diagnostics(dir1.path(), None);
        assert!(!errors1.is_empty(), "first load should report the typo");

        let dir2 = TempDir::new().unwrap();
        let (_, errors2) = ServiceConfig::load_from_directory_with_diagnostics(dir2.path(), None);
        // Empty directory: only the "config directory not found" path could
        // fire, but dir2 exists. So the only surviving error category would
        // be leakage from dir1, which we explicitly forbid.
        assert!(
            errors2.iter().all(|e| !e.contains("nonsense")),
            "second load must not leak diagnostics from the first: {errors2:?}",
        );
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
