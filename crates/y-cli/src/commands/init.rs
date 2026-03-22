//! `y-agent init` — interactive project initialization.
//!
//! Creates configuration files, detects environment dependencies,
//! guides the user through LLM provider selection, and initializes
//! the `SQLite` database with all embedded migrations.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::output;

// ---------------------------------------------------------------------------
// Clap arguments
// ---------------------------------------------------------------------------

/// Arguments for the `init` subcommand.
#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// LLM provider preset for non-interactive mode.
    #[arg(long, value_parser = ["openai", "anthropic", "deepseek", "deepseek-reasoner", "groq", "together", "ollama", "custom"])]
    pub provider: Option<String>,

    /// API key (non-interactive mode).
    #[arg(long)]
    pub api_key: Option<String>,

    /// Model name override (for custom provider).
    #[arg(long)]
    pub model: Option<String>,

    /// Base URL override (for custom provider).
    #[arg(long)]
    pub base_url: Option<String>,

    /// Skip interactive prompts; use defaults and flags.
    #[arg(long, default_value_t = false)]
    pub non_interactive: bool,

    /// Target directory for configuration files.
    /// Default: ~/.config/y-agent
    #[arg(long)]
    pub dir: Option<String>,

    /// Overwrite existing config files without asking.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

// ---------------------------------------------------------------------------
// Provider presets
// ---------------------------------------------------------------------------

/// A known LLM provider configuration preset.
#[derive(Debug, Clone)]
pub struct ProviderPreset {
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Short key used with `--provider` flag.
    pub key: &'static str,
    /// Provider ID for config file.
    pub id: &'static str,
    /// Provider type (openai / anthropic).
    pub provider_type: &'static str,
    /// Default model name.
    pub model: &'static str,
    /// Routing tags.
    pub tags: &'static [&'static str],
    /// Max concurrent requests.
    pub max_concurrency: usize,
    /// Context window in tokens.
    pub context_window: usize,
    /// Cost per 1k input tokens.
    pub cost_per_1k_input: f64,
    /// Cost per 1k output tokens.
    pub cost_per_1k_output: f64,
    /// Base URL override (None = provider default).
    pub base_url: Option<&'static str>,
    /// Whether an API key is required.
    pub requires_api_key: bool,
}

/// All built-in provider presets.
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        display_name: "OpenAI (GPT-4o)",
        key: "openai",
        id: "openai-main",
        provider_type: "openai",
        model: "gpt-4o",
        tags: &["reasoning", "general"],
        max_concurrency: 3,
        context_window: 128_000,
        cost_per_1k_input: 0.005,
        cost_per_1k_output: 0.015,
        base_url: None,
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "Anthropic (Claude 3.5 Sonnet)",
        key: "anthropic",
        id: "anthropic-main",
        provider_type: "anthropic",
        model: "claude-3-5-sonnet-20241022",
        tags: &["reasoning", "code"],
        max_concurrency: 3,
        context_window: 200_000,
        cost_per_1k_input: 0.003,
        cost_per_1k_output: 0.015,
        base_url: None,
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "DeepSeek (Chat)",
        key: "deepseek",
        id: "deepseek-main",
        provider_type: "openai",
        model: "deepseek-chat",
        tags: &["reasoning", "code", "general"],
        max_concurrency: 3,
        context_window: 65_536,
        cost_per_1k_input: 0.000_14,
        cost_per_1k_output: 0.000_28,
        base_url: Some("https://api.deepseek.com/v1"),
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "DeepSeek (Reasoner)",
        key: "deepseek-reasoner",
        id: "deepseek-reasoner",
        provider_type: "openai",
        model: "deepseek-reasoner",
        tags: &["reasoning", "deep-thinking"],
        max_concurrency: 2,
        context_window: 65_536,
        cost_per_1k_input: 0.000_55,
        cost_per_1k_output: 0.002_2,
        base_url: Some("https://api.deepseek.com/v1"),
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "Groq (Llama 3.1 70B)",
        key: "groq",
        id: "groq-main",
        provider_type: "openai",
        model: "llama-3.1-70b-versatile",
        tags: &["fast", "general"],
        max_concurrency: 3,
        context_window: 131_072,
        cost_per_1k_input: 0.000_59,
        cost_per_1k_output: 0.000_79,
        base_url: Some("https://api.groq.com/openai/v1"),
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "Together AI (Llama 3.1 70B)",
        key: "together",
        id: "together-main",
        provider_type: "openai",
        model: "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
        tags: &["general", "code"],
        max_concurrency: 3,
        context_window: 131_072,
        cost_per_1k_input: 0.000_88,
        cost_per_1k_output: 0.000_88,
        base_url: Some("https://api.together.xyz/v1"),
        requires_api_key: true,
    },
    ProviderPreset {
        display_name: "Ollama (Local \u{2014} no API key needed)",
        key: "ollama",
        id: "ollama-local",
        provider_type: "openai",
        model: "llama3.1",
        tags: &["local", "general"],
        max_concurrency: 1,
        context_window: 131_072,
        cost_per_1k_input: 0.0,
        cost_per_1k_output: 0.0,
        base_url: Some("http://localhost:11434/v1"),
        requires_api_key: false,
    },
];

/// Find a preset by its key.
pub fn find_preset(key: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.key == key)
}

/// Generate TOML for a single `[[providers]]` entry.
pub fn preset_to_toml(preset: &ProviderPreset, api_key: &str) -> String {
    let mut lines = vec![
        "[[providers]]".to_string(),
        format!("id = {:?}", preset.id),
        format!("provider_type = {:?}", preset.provider_type),
        format!("model = {:?}", preset.model),
        format!(
            "tags = [{}]",
            preset
                .tags
                .iter()
                .map(|t| format!("{t:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!("max_concurrency = {}", preset.max_concurrency),
        format!("context_window = {}", preset.context_window),
        format!("cost_per_1k_input = {}", preset.cost_per_1k_input),
        format!("cost_per_1k_output = {}", preset.cost_per_1k_output),
    ];

    if preset.requires_api_key && !api_key.is_empty() {
        lines.push(format!("api_key = {api_key:?}"));
    }

    if let Some(url) = preset.base_url {
        lines.push(format!("base_url = {url:?}"));
    }

    lines.join("\n")
}

/// Generate a custom provider TOML entry.
pub fn custom_provider_to_toml(model: &str, base_url: &str, api_key: &str) -> String {
    let mut lines = vec![
        "[[providers]]".to_string(),
        "id = \"custom-main\"".to_string(),
        "provider_type = \"openai\"".to_string(),
        format!("model = {model:?}"),
        "tags = [\"general\"]".to_string(),
        "max_concurrency = 3".to_string(),
        "context_window = 128000".to_string(),
        "cost_per_1k_input = 0.0".to_string(),
        "cost_per_1k_output = 0.0".to_string(),
    ];

    if !api_key.is_empty() {
        lines.push(format!("api_key = {api_key:?}"));
    }

    if !base_url.is_empty() {
        lines.push(format!("base_url = {base_url:?}"));
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Prompter trait (for testability)
// ---------------------------------------------------------------------------

/// Abstraction over interactive terminal prompts.
pub trait Prompter {
    /// Ask a yes/no question.
    fn confirm(&self, message: &str, default: bool) -> Result<bool>;

    /// Ask the user to select from a list. Returns the chosen index.
    fn select(&self, message: &str, items: &[&str], default: usize) -> Result<usize>;

    /// Ask the user for text input with an optional default.
    fn input(&self, message: &str, default: Option<&str>) -> Result<String>;
}

/// Interactive prompter wrapping `dialoguer`.
pub struct InteractivePrompter;

impl Prompter for InteractivePrompter {
    fn confirm(&self, message: &str, default: bool) -> Result<bool> {
        dialoguer::Confirm::new()
            .with_prompt(message)
            .default(default)
            .interact()
            .context("interactive confirm failed")
    }

    fn select(&self, message: &str, items: &[&str], default: usize) -> Result<usize> {
        dialoguer::Select::new()
            .with_prompt(message)
            .items(items)
            .default(default)
            .interact()
            .context("interactive select failed")
    }

    fn input(&self, message: &str, default: Option<&str>) -> Result<String> {
        let mut input = dialoguer::Input::<String>::new().with_prompt(message);
        if let Some(d) = default {
            input = input.default(d.to_string());
        }
        input.interact_text().context("interactive input failed")
    }
}

/// Non-interactive prompter: always returns defaults.
pub struct NonInteractivePrompter;

impl Prompter for NonInteractivePrompter {
    fn confirm(&self, _message: &str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn select(&self, _message: &str, _items: &[&str], default: usize) -> Result<usize> {
        Ok(default)
    }

    fn input(&self, _message: &str, default: Option<&str>) -> Result<String> {
        Ok(default.unwrap_or("").to_string())
    }
}

// ---------------------------------------------------------------------------
// Dependency detection
// ---------------------------------------------------------------------------

/// Status of a single environment dependency.
#[derive(Debug, Clone)]
pub struct DependencyStatus {
    pub name: &'static str,
    pub required: bool,
    pub found: bool,
    pub detail: String,
}

/// Run a command and capture the first line of stdout.
fn run_version_command(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let text = if stdout.trim().is_empty() {
                stderr.to_string()
            } else {
                stdout.to_string()
            };
            text.lines().next().map(|l| l.trim().to_string())
        })
}

/// Check if a TCP port is reachable on localhost.
fn check_tcp_port(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(2),
    )
    .is_ok()
}

/// Detect all environment dependencies.
pub fn check_dependencies() -> Vec<DependencyStatus> {
    vec![
        {
            let version = run_version_command("rustc", &["--version"]);
            DependencyStatus {
                name: "rustc",
                required: true,
                found: version.is_some(),
                detail: version.unwrap_or_else(|| "not found".to_string()),
            }
        },
        {
            let version = run_version_command("cargo", &["--version"]);
            DependencyStatus {
                name: "cargo",
                required: true,
                found: version.is_some(),
                detail: version.unwrap_or_else(|| "not found".to_string()),
            }
        },
        {
            let version = run_version_command("sqlite3", &["--version"]);
            DependencyStatus {
                name: "sqlite3",
                required: false,
                found: version.is_some(),
                detail: version.unwrap_or_else(|| "not found (optional)".to_string()),
            }
        },
        {
            let version = run_version_command("docker", &["--version"]);
            DependencyStatus {
                name: "docker",
                required: false,
                found: version.is_some(),
                detail: version.unwrap_or_else(|| "not found (optional)".to_string()),
            }
        },
        {
            let version = run_version_command("docker", &["compose", "version"]);
            DependencyStatus {
                name: "docker compose",
                required: false,
                found: version.is_some(),
                detail: version.unwrap_or_else(|| "not found (optional)".to_string()),
            }
        },
        {
            let found = check_tcp_port(5432);
            DependencyStatus {
                name: "PostgreSQL",
                required: false,
                found,
                detail: if found {
                    "port 5432 reachable".to_string()
                } else {
                    "not found (optional \u{2014} for diagnostics)".to_string()
                },
            }
        },
        {
            let found = check_tcp_port(6333);
            DependencyStatus {
                name: "Qdrant",
                required: false,
                found,
                detail: if found {
                    "port 6333 reachable".to_string()
                } else {
                    "not found (optional \u{2014} for vector search)".to_string()
                },
            }
        },
        {
            let version = run_version_command("sqlx", &["--version"]);
            DependencyStatus {
                name: "sqlx-cli",
                required: false,
                found: version.is_some(),
                detail: version
                    .unwrap_or_else(|| "not found (optional \u{2014} for migrations)".to_string()),
            }
        },
    ]
}

/// Format dependency statuses for display.
pub fn format_dependencies(deps: &[DependencyStatus]) -> String {
    use crate::output::{format_table, TableRow};

    let headers = &["Dependency", "Status", "Detail"];
    let rows: Vec<TableRow> = deps
        .iter()
        .map(|d| {
            let status = if d.found {
                "found".to_string()
            } else if d.required {
                "MISSING".to_string()
            } else {
                "not found".to_string()
            };
            TableRow {
                cells: vec![d.name.to_string(), status, d.detail.clone()],
            }
        })
        .collect();

    format_table(headers, &rows)
}

// ---------------------------------------------------------------------------
// Configuration generation
// ---------------------------------------------------------------------------

// Embed all example config files at compile time.
const EXAMPLE_Y_AGENT: &str = include_str!("../../../../config/y-agent.example.toml");
const EXAMPLE_PROVIDERS: &str = include_str!("../../../../config/providers.example.toml");
const EXAMPLE_STORAGE: &str = include_str!("../../../../config/storage.example.toml");
const EXAMPLE_SESSION: &str = include_str!("../../../../config/session.example.toml");
const EXAMPLE_RUNTIME: &str = include_str!("../../../../config/runtime.example.toml");
const EXAMPLE_KNOWLEDGE: &str = include_str!("../../../../config/knowledge.example.toml");
const EXAMPLE_HOOKS: &str = include_str!("../../../../config/hooks.example.toml");
const EXAMPLE_TOOLS: &str = include_str!("../../../../config/tools.example.toml");
const EXAMPLE_BROWSER: &str = include_str!("../../../../config/browser.example.toml");
const EXAMPLE_GUARDRAILS: &str = include_str!("../../../../config/guardrails.example.toml");

/// Mapping of config file names to their embedded content.
const CONFIG_TEMPLATES: &[(&str, &str)] = &[
    ("y-agent.toml", EXAMPLE_Y_AGENT),
    ("providers.toml", EXAMPLE_PROVIDERS),
    ("storage.toml", EXAMPLE_STORAGE),
    ("session.toml", EXAMPLE_SESSION),
    ("runtime.toml", EXAMPLE_RUNTIME),
    ("knowledge.toml", EXAMPLE_KNOWLEDGE),
    ("hooks.toml", EXAMPLE_HOOKS),
    ("tools.toml", EXAMPLE_TOOLS),
    ("browser.toml", EXAMPLE_BROWSER),
    ("guardrails.toml", EXAMPLE_GUARDRAILS),
];

// ---------------------------------------------------------------------------
// Skills seeding (copy from source directory at install time)
// ---------------------------------------------------------------------------

/// Detect the skills source directory shipped alongside the binary.
///
/// Looks for `<exe_dir>/../skills/` (release install layout) and
/// `<exe_dir>/../../skills/` (development layout). Returns `None`
/// if neither location exists.
pub fn detect_skills_source() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // Zip archive layout (standalone): <unzipped_dir>/y-agent -> <unzipped_dir>/skills/
    let candidate = exe_dir.join("skills");
    if candidate.is_dir() {
        return Some(candidate);
    }

    // Release layout (Unix-style): <prefix>/bin/y-agent -> <prefix>/skills/
    let candidate = exe_dir.join("../skills");
    if candidate.is_dir() {
        return Some(candidate);
    }

    // Development / workspace layout: target/release/y-agent -> ../../skills/
    let candidate = exe_dir.join("../../skills");
    if candidate.is_dir() {
        return Some(candidate);
    }

    None
}

/// Seed skills from a source directory into the user's skills store.
///
/// Thin wrapper around [`y_service::init::copy_skills_from_source`].
/// Only installs skills that don't already exist (won't overwrite user modifications).
/// Returns the list of skill names that were seeded.
pub fn seed_skills_from_source(source_dir: &Path, data_dir: &Path) -> Result<Vec<String>> {
    y_service::init::copy_skills_from_source(source_dir, data_dir)
}

/// Seed built-in prompt files into the user's config directory.
///
/// Creates `<config_dir>/prompts/` and writes each prompt file only if it
/// does not already exist (preserving user modifications).
/// Returns the list of prompt filenames that were seeded.
pub fn seed_builtin_prompts(config_dir: &Path) -> Result<Vec<String>> {
    let prompts_dir = config_dir.join("prompts");
    std::fs::create_dir_all(&prompts_dir)
        .with_context(|| format!("creating prompts directory: {}", prompts_dir.display()))?;

    let mut seeded = Vec::new();

    for &(filename, content) in y_service::BUILTIN_PROMPT_FILES {
        let dest = prompts_dir.join(filename);

        // Skip if already exists (don't overwrite user modifications).
        if dest.exists() {
            continue;
        }

        std::fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;

        seeded.push(filename.to_string());
    }

    Ok(seeded)
}

/// Seed built-in agent definitions into the user's config directory.
///
/// Creates `<config_dir>/agents/` and writes each agent TOML file only if it
/// does not already exist (preserving user modifications).
/// Returns the list of agent IDs that were seeded.
pub fn seed_builtin_agents(config_dir: &Path) -> Result<Vec<String>> {
    use y_agent::agent::registry::AgentRegistry;

    let agents_dir = config_dir.join("agents");
    std::fs::create_dir_all(&agents_dir)
        .with_context(|| format!("creating agents directory: {}", agents_dir.display()))?;

    let mut seeded = Vec::new();

    for (name, content) in AgentRegistry::builtin_toml_sources() {
        let dest = agents_dir.join(format!("{name}.toml"));

        // Skip if already exists (don't overwrite user modifications).
        if dest.exists() {
            continue;
        }

        std::fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;

        seeded.push(name.to_string());
    }

    Ok(seeded)
}

/// Ensure required directories exist.
///
/// - `base` is the config directory (`~/.config/y-agent/`) — config files live
///   directly here, no subdirectory.
/// - `data_dir` is the state data directory (`~/.local/state/y-agent/data/`).
pub fn ensure_directories(base: &Path, data_dir: &Path) -> Result<Vec<PathBuf>> {
    let dirs = [
        base.to_path_buf(),
        data_dir.to_path_buf(),
        data_dir.join("transcripts"),
    ];

    for dir in &dirs {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory: {}", dir.display()))?;
    }

    Ok(dirs.to_vec())
}

/// Write a file if it doesn't exist, or if force/confirm allows overwrite.
/// Returns true if the file was written.
pub fn write_if_allowed(
    path: &Path,
    content: &str,
    force: bool,
    prompter: &dyn Prompter,
) -> Result<bool> {
    if path.exists() && !force {
        let msg = format!("{} already exists. Overwrite?", path.display());
        if !prompter.confirm(&msg, false)? {
            return Ok(false);
        }
    }

    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Copy all example config templates directly into the base config directory.
pub fn copy_example_configs(
    base: &Path,
    force: bool,
    prompter: &dyn Prompter,
) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();

    for (name, content) in CONFIG_TEMPLATES {
        let dest = base.join(name);
        if write_if_allowed(&dest, content, force, prompter)? {
            created.push(dest);
        }
    }

    Ok(created)
}

/// Generate providers.toml with the user's selected providers.
pub fn generate_providers_config(base: &Path, provider_toml_blocks: &[String]) -> Result<PathBuf> {
    let path = base.join("providers.toml");

    let header = "\
# ==============================================================================
# y-agent \u{2014} LLM Provider Pool Configuration
# ==============================================================================

# Freeze duration before retrying a failed provider (seconds).
default_freeze_duration_secs = 60

# Maximum freeze duration cap for exponential backoff (seconds).
max_freeze_duration_secs = 3600

# Health check interval for frozen providers (seconds).
health_check_interval_secs = 30

# ==============================================================================
# Provider Definitions
# ==============================================================================
";

    let mut content = header.to_string();
    for block in provider_toml_blocks {
        content.push('\n');
        content.push_str(block);
        content.push('\n');
    }

    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Interactive provider selection
// ---------------------------------------------------------------------------

/// Result of the provider selection flow.
#[derive(Debug)]
pub struct ProviderSelection {
    /// TOML blocks for each selected provider.
    pub toml_blocks: Vec<String>,
    /// Human-readable descriptions for summary.
    pub descriptions: Vec<String>,
}

/// Run the interactive provider selection flow.
pub fn select_providers(args: &InitArgs, prompter: &dyn Prompter) -> Result<ProviderSelection> {
    let mut toml_blocks = Vec::new();
    let mut descriptions = Vec::new();

    // If --provider flag is set, use it directly.
    if let Some(ref provider_key) = args.provider {
        if provider_key == "custom" {
            let model = args.model.as_deref().unwrap_or("gpt-4o");
            let base_url = args.base_url.as_deref().unwrap_or("");
            let api_key = args.api_key.as_deref().unwrap_or("");

            toml_blocks.push(custom_provider_to_toml(model, base_url, api_key));
            descriptions.push(format!("Custom ({model})"));
        } else if let Some(preset) = find_preset(provider_key) {
            let api_key = args.api_key.as_deref().unwrap_or("");
            toml_blocks.push(preset_to_toml(preset, api_key));
            descriptions.push(preset.display_name.to_string());
        }

        return Ok(ProviderSelection {
            toml_blocks,
            descriptions,
        });
    }

    // Interactive selection loop.
    loop {
        // Build display list: presets + custom + skip.
        let mut display_names: Vec<&str> =
            PROVIDER_PRESETS.iter().map(|p| p.display_name).collect();
        display_names.push("Custom (OpenAI-compatible endpoint)");
        display_names.push("Skip (Configure later)");

        let choice = prompter.select("Select LLM provider", &display_names, 0)?;

        match choice.cmp(&PROVIDER_PRESETS.len()) {
            std::cmp::Ordering::Less => {
                let preset = &PROVIDER_PRESETS[choice];

                let api_key = if preset.requires_api_key {
                    prompter.input(&format!("Enter API key for {}", preset.display_name), None)?
                } else {
                    String::new()
                };

                toml_blocks.push(preset_to_toml(preset, &api_key));
                descriptions.push(preset.display_name.to_string());
            }
            std::cmp::Ordering::Equal => {
                // Custom provider.
                let model = prompter.input("Model name", Some("gpt-4o"))?;
                let base_url = prompter.input("Base URL", Some("https://api.example.com/v1"))?;
                let api_key = prompter.input("API key (empty if none)", Some(""))?;

                toml_blocks.push(custom_provider_to_toml(&model, &base_url, &api_key));
                descriptions.push(format!("Custom ({model})"));
            }
            std::cmp::Ordering::Greater => {
                // Skip.
                break;
            }
        }

        if !prompter.confirm("Add another provider?", false)? {
            break;
        }
    }

    Ok(ProviderSelection {
        toml_blocks,
        descriptions,
    })
}

// ---------------------------------------------------------------------------
// Database initialization
// ---------------------------------------------------------------------------

/// Build a minimal `StorageConfig` for use during `init`.
///
/// - `config_base` is `~/.config/y-agent/` where config files live.
/// - `data_dir` is `~/.local/state/y-agent/data/` where the database lives.
///
/// Only one connection is needed since init just runs migrations.
fn build_init_storage_config(_config_base: &Path, data_dir: &Path) -> y_service::StorageConfig {
    y_service::StorageConfig {
        db_path: data_dir.join("y-agent.db").to_string_lossy().to_string(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5000,
        transcript_dir: data_dir.join("transcripts"),
    }
}

/// Create the `SQLite` database and run all embedded migrations.
///
/// - `config_base` is `~/.config/y-agent/` where config files live.
/// - `data_dir` is `~/.local/state/y-agent/data/` where the database lives.
///
/// Returns the path to the created database file.
pub async fn initialize_database(config_base: &Path, data_dir: &Path) -> Result<PathBuf> {
    let config = build_init_storage_config(config_base, data_dir);
    let db_path = PathBuf::from(&config.db_path);

    let pool = y_service::create_pool(&config)
        .await
        .context("failed to create SQLite database")?;

    y_service::migration::run_embedded_migrations(&pool)
        .await
        .context("failed to run database migrations")?;

    pool.close().await;

    Ok(db_path)
}

/// Determine whether database initialization should proceed.
///
/// If the database file already exists and `force` is false, the user is
/// prompted. Migrations are idempotent, so the default answer is yes.
pub fn should_initialize_db(db_path: &Path, force: bool, prompter: &dyn Prompter) -> Result<bool> {
    if !db_path.exists() || force {
        return Ok(true);
    }
    prompter.confirm("Database already exists. Re-run migrations?", true)
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Get the default configuration directory.
///
/// Always uses `~/.config/y-agent` regardless of platform, matching the
/// convention used by `ConfigLoader::dirs_user_config()`.
fn default_config_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(
            || PathBuf::from("."),
            |h| PathBuf::from(h).join(".config").join("y-agent"),
        )
}

/// Get the default state data directory.
///
/// Uses `$XDG_STATE_HOME/y-agent/data/` (defaults to `~/.local/state/y-agent/data/`).
/// This follows XDG conventions: state data that persists across restarts but
/// is not configuration.
pub fn default_state_data_dir() -> PathBuf {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        });
    state_home.map_or_else(|| PathBuf::from("data"), |s| s.join("y-agent").join("data"))
}

/// Run the `init` subcommand.
pub async fn run(args: &InitArgs) -> Result<()> {
    println!();
    println!(
        "  y-agent v{} \u{2014} Project Initialization",
        env!("CARGO_PKG_VERSION")
    );
    println!("  =========================================");
    println!();

    // --- Step 0: Determine target directory ---
    let prompter: Box<dyn Prompter> = if args.non_interactive {
        Box::new(NonInteractivePrompter)
    } else {
        Box::new(InteractivePrompter)
    };

    let base = if let Some(ref dir) = args.dir {
        // Explicit --dir flag.
        PathBuf::from(dir)
    } else {
        // Interactive: ask user to confirm or change.
        let default_dir = default_config_dir();
        let default_str = default_dir.to_string_lossy().to_string();
        let chosen = prompter.input("Configuration directory", Some(&default_str))?;
        PathBuf::from(chosen)
    };

    // Determine the state data directory.
    let data_dir = default_state_data_dir();

    println!("  Config: {}", base.display());
    println!("  Data:   {}\n", data_dir.display());

    // --- Step 1: Dependency detection ---
    println!("  Checking environment dependencies...");
    println!();

    let deps = check_dependencies();
    let table = format_dependencies(&deps);
    for line in table.lines() {
        println!("  {line}");
    }
    println!();

    // Check for missing required dependencies.
    let missing_required: Vec<_> = deps.iter().filter(|d| d.required && !d.found).collect();
    if !missing_required.is_empty() {
        for dep in &missing_required {
            output::print_error(&format!("Required dependency missing: {}", dep.name));
        }
        output::print_warning("Some required dependencies are missing. Continuing anyway...");
        println!();
    }

    // --- Step 2: Provider selection ---
    let selection = select_providers(args, prompter.as_ref())?;

    if selection.toml_blocks.is_empty() {
        output::print_warning(
            "No providers selected. You can configure them later in providers.toml",
        );
    }

    println!();
    println!("  Creating project files...");
    println!();

    // --- Step 3: Create directories ---
    let created_dirs = ensure_directories(&base, &data_dir)?;
    for dir in &created_dirs {
        output::print_success(&format!("Created {}/", dir.display()));
    }

    // --- Step 4: Copy example configs ---
    let created_files = copy_example_configs(&base, args.force, prompter.as_ref())?;
    for file in &created_files {
        output::print_success(&format!("Created {}", file.display()));
    }

    // --- Step 5: Patch providers.toml ---
    if !selection.toml_blocks.is_empty() {
        let providers_path = generate_providers_config(&base, &selection.toml_blocks)?;
        let provider_names = selection.descriptions.join(", ");
        output::print_success(&format!(
            "Created {} ({provider_names})",
            providers_path.display()
        ));
    }

    // --- Step 6: Initialize SQLite database ---
    let db_path = data_dir.join("y-agent.db");
    if should_initialize_db(&db_path, args.force, prompter.as_ref())? {
        match initialize_database(&base, &data_dir).await {
            Ok(path) => {
                output::print_success(&format!("Database initialized: {}", path.display()));
            }
            Err(e) => {
                output::print_error(&format!("Database initialization failed: {e}"));
                output::print_warning("You can retry later with: y-agent init --force");
            }
        }
    } else {
        output::print_info("Skipped database initialization (existing database preserved)");
    }

    // --- Step 6c: Seed skills from source directory ---
    if let Some(skills_source) = detect_skills_source() {
        match seed_skills_from_source(&skills_source, &data_dir) {
            Ok(seeded) => {
                if seeded.is_empty() {
                    output::print_info("Skills already installed");
                } else {
                    output::print_success(&format!(
                        "Seeded {} skill(s): {}",
                        seeded.len(),
                        seeded.join(", ")
                    ));
                }
            }
            Err(e) => {
                output::print_warning(&format!("Skills seeding failed: {e}"));
                output::print_info("You can manually copy skills later");
            }
        }
    } else {
        output::print_info("No skills source directory found; skipping skill seeding");
    }

    // --- Step 6d: Seed built-in prompts ---
    match seed_builtin_prompts(&base) {
        Ok(seeded) => {
            if seeded.is_empty() {
                output::print_info("Built-in prompts already installed");
            } else {
                output::print_success(&format!(
                    "Seeded {} built-in prompt(s) to {}/prompts/",
                    seeded.len(),
                    base.display()
                ));
            }
        }
        Err(e) => {
            output::print_warning(&format!("Built-in prompts seeding failed: {e}"));
            output::print_info("You can manually copy prompt files later");
        }
    }

    // --- Step 6e: Seed built-in agent definitions ---
    match seed_builtin_agents(&base) {
        Ok(seeded) => {
            if seeded.is_empty() {
                output::print_info("Built-in agent definitions already installed");
            } else {
                output::print_success(&format!(
                    "Seeded {} built-in agent(s) to {}/agents/",
                    seeded.len(),
                    base.display()
                ));
            }
        }
        Err(e) => {
            output::print_warning(&format!("Built-in agent seeding failed: {e}"));
            output::print_info("You can manually copy agent files later");
        }
    }

    // --- Step 7: Summary ---
    println!();
    println!("  Next steps:");

    println!("  1. Review config:     {}", base.display());
    println!("  2. Validate config:   y-agent config validate");
    println!("  3. Start chatting:    y-agent chat");
    println!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Mock prompter with scripted responses.
    struct MockPrompter {
        confirm_responses: RefCell<Vec<bool>>,
        select_responses: RefCell<Vec<usize>>,
        input_responses: RefCell<Vec<String>>,
    }

    impl MockPrompter {
        fn new(confirms: Vec<bool>, selects: Vec<usize>, inputs: Vec<String>) -> Self {
            Self {
                confirm_responses: RefCell::new(confirms),
                select_responses: RefCell::new(selects),
                input_responses: RefCell::new(inputs),
            }
        }
    }

    impl Prompter for MockPrompter {
        fn confirm(&self, _message: &str, default: bool) -> Result<bool> {
            let mut responses = self.confirm_responses.borrow_mut();
            if responses.is_empty() {
                Ok(default)
            } else {
                Ok(responses.remove(0))
            }
        }

        fn select(&self, _message: &str, _items: &[&str], default: usize) -> Result<usize> {
            let mut responses = self.select_responses.borrow_mut();
            if responses.is_empty() {
                Ok(default)
            } else {
                Ok(responses.remove(0))
            }
        }

        fn input(&self, _message: &str, default: Option<&str>) -> Result<String> {
            let mut responses = self.input_responses.borrow_mut();
            if responses.is_empty() {
                Ok(default.unwrap_or("").to_string())
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    // T-INIT-001: clap parses `y-agent init` correctly.
    #[test]
    fn test_parse_init_defaults() {
        let args = InitArgs {
            provider: None,
            api_key: None,
            model: None,
            base_url: None,
            non_interactive: false,
            dir: None,
            force: false,
        };
        assert!(!args.non_interactive);
        assert!(!args.force);
        assert!(args.dir.is_none());
    }

    // T-INIT-002: --provider flag restricts to valid values.
    #[test]
    fn test_find_preset_openai() {
        let preset = find_preset("openai").expect("openai should exist");
        assert_eq!(preset.provider_type, "openai");
        assert_eq!(preset.model, "gpt-4o");
    }

    // T-INIT-003: all presets have valid fields.
    #[test]
    fn test_all_presets_valid() {
        for preset in PROVIDER_PRESETS {
            assert!(
                !preset.key.is_empty(),
                "key empty for {}",
                preset.display_name
            );
            assert!(
                !preset.id.is_empty(),
                "id empty for {}",
                preset.display_name
            );
            assert!(
                !preset.provider_type.is_empty(),
                "provider_type empty for {}",
                preset.display_name
            );
            assert!(
                !preset.model.is_empty(),
                "model empty for {}",
                preset.display_name
            );
            assert!(
                preset.context_window > 0,
                "context_window = 0 for {}",
                preset.display_name
            );
        }
    }

    // T-INIT-004: preset_to_toml generates parseable TOML.
    #[test]
    fn test_preset_to_toml_roundtrip() {
        let preset = find_preset("openai").unwrap();
        let toml_str = preset_to_toml(preset, "sk-test-key-123");

        // Verify structure.
        assert!(toml_str.contains("[[providers]]"));
        assert!(toml_str.contains("id = \"openai-main\""));
        assert!(toml_str.contains("provider_type = \"openai\""));
        assert!(toml_str.contains("model = \"gpt-4o\""));
        assert!(toml_str.contains("api_key = \"sk-test-key-123\""));

        // Verify it parses as part of a ProviderPoolConfig.
        let pool_toml = format!("default_freeze_duration_secs = 60\n\n{toml_str}\n");
        let parsed: y_service::ProviderPoolConfig =
            toml::from_str(&pool_toml).expect("should parse");
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].id, "openai-main");
    }

    // T-INIT-005: custom_provider_to_toml generates valid TOML.
    #[test]
    fn test_custom_provider_to_toml() {
        let toml_str =
            custom_provider_to_toml("my-model", "https://api.example.com/v1", "sk-my-key");
        assert!(toml_str.contains("model = \"my-model\""));
        assert!(toml_str.contains("base_url = \"https://api.example.com/v1\""));
        assert!(toml_str.contains("api_key = \"sk-my-key\""));
    }

    // T-INIT-006: MockPrompter returns scripted responses.
    #[test]
    fn test_mock_prompter() {
        let mock = MockPrompter::new(vec![true, false], vec![2], vec!["hello".to_string()]);

        assert!(mock.confirm("q?", false).unwrap());
        assert!(!mock.confirm("q?", true).unwrap());
        // exhausted: returns default
        assert!(mock.confirm("q?", true).unwrap());

        assert_eq!(mock.select("q?", &["a", "b", "c"], 0).unwrap(), 2);
        assert_eq!(mock.input("q?", Some("default")).unwrap(), "hello");
    }

    // T-INIT-007: NonInteractivePrompter returns defaults.
    #[test]
    fn test_non_interactive_prompter() {
        let p = NonInteractivePrompter;
        assert!(p.confirm("q?", true).unwrap());
        assert!(!p.confirm("q?", false).unwrap());
        assert_eq!(p.select("q?", &["a", "b"], 1).unwrap(), 1);
        assert_eq!(p.input("q?", Some("def")).unwrap(), "def");
        assert_eq!(p.input("q?", None).unwrap(), "");
    }

    // T-INIT-008: ensure_directories creates expected dirs.
    #[test]
    fn test_ensure_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("state").join("data");
        let dirs = ensure_directories(&config_dir, &data_dir).unwrap();

        assert_eq!(dirs.len(), 3);
        assert!(config_dir.is_dir());
        assert!(data_dir.is_dir());
        assert!(data_dir.join("transcripts").is_dir());
    }

    // T-INIT-009: copy_example_configs creates all 10 files directly in base.
    #[test]
    fn test_copy_example_configs() {
        let tmp = tempfile::tempdir().unwrap();

        let prompter = NonInteractivePrompter;
        let created = copy_example_configs(tmp.path(), false, &prompter).unwrap();

        assert_eq!(created.len(), CONFIG_TEMPLATES.len());
        for (name, _) in CONFIG_TEMPLATES {
            assert!(
                tmp.path().join(name).exists(),
                "{name} should exist directly in base"
            );
        }
    }

    // T-INIT-010: existing files are not overwritten without force.
    #[test]
    fn test_skip_existing_no_force() {
        let tmp = tempfile::tempdir().unwrap();

        // Pre-create one file directly in base.
        let existing = tmp.path().join("y-agent.toml");
        std::fs::write(&existing, "existing content").unwrap();

        // Mock says "no" to overwrite.
        let prompter = MockPrompter::new(vec![false], vec![], vec![]);
        let created = copy_example_configs(tmp.path(), false, &prompter).unwrap();

        // The pre-existing file was skipped.
        assert_eq!(created.len(), CONFIG_TEMPLATES.len() - 1);
        let content = std::fs::read_to_string(&existing).unwrap();
        assert_eq!(content, "existing content");
    }

    // T-INIT-011: --force overwrites existing files.
    #[test]
    fn test_overwrite_with_force() {
        let tmp = tempfile::tempdir().unwrap();

        let existing = tmp.path().join("y-agent.toml");
        std::fs::write(&existing, "old content").unwrap();

        let prompter = NonInteractivePrompter;
        let created = copy_example_configs(tmp.path(), true, &prompter).unwrap();

        assert_eq!(created.len(), CONFIG_TEMPLATES.len());
        let content = std::fs::read_to_string(&existing).unwrap();
        assert_ne!(content, "old content");
    }

    // T-INIT-012: generate_providers_config writes correct TOML.
    #[test]
    fn test_generate_providers_config() {
        let tmp = tempfile::tempdir().unwrap();

        let preset = find_preset("anthropic").unwrap();
        let block = preset_to_toml(preset, "sk-ant-test-key");

        let path = generate_providers_config(tmp.path(), &[block]).unwrap();
        assert!(path.exists());
        assert_eq!(
            path,
            tmp.path().join("providers.toml"),
            "providers.toml should be directly in base"
        );

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("anthropic-main"));
        assert!(content.contains("claude-3-5-sonnet"));
        assert!(content.contains("default_freeze_duration_secs"));

        // Verify it parses.
        let parsed: y_service::ProviderPoolConfig = toml::from_str(&content).expect("should parse");
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].provider_type, "anthropic");
    }

    // T-INIT-014: select_providers with --provider flag.
    #[test]
    fn test_select_providers_non_interactive() {
        let args = InitArgs {
            provider: Some("openai".to_string()),
            api_key: Some("sk-test-key".to_string()),
            model: None,
            base_url: None,
            non_interactive: true,
            dir: Some(".".to_string()),
            force: false,
        };

        let prompter = NonInteractivePrompter;
        let sel = select_providers(&args, &prompter).unwrap();

        assert_eq!(sel.toml_blocks.len(), 1);
        assert!(sel.toml_blocks[0].contains("openai-main"));
        assert!(sel.toml_blocks[0].contains("api_key = \"sk-test-key\""));
        assert_eq!(sel.descriptions, vec!["OpenAI (GPT-4o)"]);
    }

    // T-INIT-015: interactive provider selection via MockPrompter.
    #[test]
    fn test_select_providers_interactive() {
        let args = InitArgs {
            provider: None,
            api_key: None,
            model: None,
            base_url: None,
            non_interactive: false,
            dir: Some(".".to_string()),
            force: false,
        };

        // Select index 1 (Anthropic), provide key, decline adding more.
        let prompter = MockPrompter::new(
            vec![false],                            // "Add another provider?" -> no
            vec![1],                                // Select Anthropic
            vec!["sk-ant-my-test-key".to_string()], // API key
        );

        let sel = select_providers(&args, &prompter).unwrap();

        assert_eq!(sel.toml_blocks.len(), 1);
        assert!(sel.toml_blocks[0].contains("anthropic-main"));
        assert!(sel.toml_blocks[0].contains("api_key = \"sk-ant-my-test-key\""));
        assert_eq!(sel.descriptions, vec!["Anthropic (Claude 3.5 Sonnet)"]);
    }

    // T-INIT-026: "Skip" option in interactive provider selection.
    #[test]
    fn test_select_providers_skip() {
        let args = InitArgs {
            provider: None,
            api_key: None,
            model: None,
            base_url: None,
            non_interactive: false,
            dir: Some(".".to_string()),
            force: false,
        };

        // Select the "Skip" option (last index).
        // display_names: PROVIDER_PRESETS (length depends on config, but Skip is always last)
        let skip_index = PROVIDER_PRESETS.len() + 1;

        let prompter = MockPrompter::new(
            vec![],           // No "Add another provider?" prompt
            vec![skip_index], // Select Skip
            vec![],           // No API key prompt
        );

        let sel = select_providers(&args, &prompter).unwrap();

        assert_eq!(sel.toml_blocks.len(), 0);
        assert_eq!(sel.descriptions.len(), 0);
    }

    // T-INIT-016: dependency checker returns expected structure.
    #[test]
    fn test_check_dependencies_structure() {
        let deps = check_dependencies();
        assert_eq!(deps.len(), 8);

        // At least rustc + cargo should be found (we're running Rust tests).
        let rustc = deps.iter().find(|d| d.name == "rustc").unwrap();
        assert!(rustc.required);
        assert!(rustc.found);

        let cargo = deps.iter().find(|d| d.name == "cargo").unwrap();
        assert!(cargo.required);
        assert!(cargo.found);
    }

    // T-INIT-017: format_dependencies produces readable table.
    #[test]
    fn test_format_dependencies() {
        let deps = vec![
            DependencyStatus {
                name: "rustc",
                required: true,
                found: true,
                detail: "rustc 1.76.0".to_string(),
            },
            DependencyStatus {
                name: "docker",
                required: false,
                found: false,
                detail: "not found (optional)".to_string(),
            },
        ];

        let table = format_dependencies(&deps);
        assert!(table.contains("rustc"));
        assert!(table.contains("found"));
        assert!(table.contains("docker"));
        assert!(table.contains("not found"));
    }

    // T-INIT-018: Ollama preset has no API key requirement.
    #[test]
    fn test_ollama_preset_no_api_key() {
        let preset = find_preset("ollama").unwrap();
        assert!(!preset.requires_api_key);

        let toml_str = preset_to_toml(preset, "");
        assert!(!toml_str.contains("api_key"));
        assert!(toml_str.contains("http://localhost:11434/v1"));
    }

    // T-INIT-019: build_init_storage_config produces correct paths and settings.
    #[test]
    fn test_build_init_storage_config() {
        let config_base = PathBuf::from("/tmp/test-config");
        let data_dir = PathBuf::from("/tmp/test-state/data");
        let config = build_init_storage_config(&config_base, &data_dir);

        assert_eq!(
            config.db_path, "/tmp/test-state/data/y-agent.db",
            "db_path should be under data dir"
        );
        assert_eq!(config.pool_size, 1);
        assert!(config.wal_enabled);
        assert_eq!(config.busy_timeout_ms, 5000);
        assert_eq!(
            config.transcript_dir,
            PathBuf::from("/tmp/test-state/data/transcripts")
        );
    }

    // T-INIT-020: initialize_database creates a real SQLite file on disk.
    #[tokio::test]
    async fn test_initialize_database_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("state").join("data");
        ensure_directories(&config_dir, &data_dir).unwrap();

        let db_path = initialize_database(&config_dir, &data_dir).await.unwrap();

        assert!(db_path.exists(), "database file should exist on disk");
        assert!(
            db_path.to_string_lossy().ends_with("y-agent.db"),
            "should be named y-agent.db"
        );
    }

    // T-INIT-021: after init, all 6 migration tables exist in the DB.
    #[tokio::test]
    async fn test_initialize_database_runs_all_migrations() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("state").join("data");
        ensure_directories(&config_dir, &data_dir).unwrap();

        initialize_database(&config_dir, &data_dir).await.unwrap();

        // Re-open the database to verify tables.
        let config = build_init_storage_config(&config_dir, &data_dir);
        let pool = y_service::create_pool(&config).await.unwrap();

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        let table_names: Vec<&str> = tables.iter().map(|t| t.0.as_str()).collect();

        let expected = [
            "session_metadata",
            "orchestrator_checkpoints",
            "orchestrator_workflows",
            "file_journal_entries",
            "tool_dynamic_definitions",
            "tool_activation_log",
            "agent_definitions",
            "schedule_definitions",
            "schedule_executions",
            "stm_experience_store",
        ];

        for expected_table in &expected {
            assert!(
                table_names.contains(expected_table),
                "table {expected_table} should exist, got: {table_names:?}"
            );
        }

        pool.close().await;
    }

    // T-INIT-022: initialize_database is idempotent (second call succeeds).
    #[tokio::test]
    async fn test_initialize_database_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("state").join("data");
        ensure_directories(&config_dir, &data_dir).unwrap();

        initialize_database(&config_dir, &data_dir).await.unwrap();
        // Second call should succeed without error.
        initialize_database(&config_dir, &data_dir)
            .await
            .expect("second init should be idempotent");
    }

    // T-INIT-023: existing DB skipped when prompter says no.
    #[test]
    fn test_should_initialize_db_skip_on_decline() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("y-agent.db");
        // Pre-create the file.
        std::fs::write(&db_path, "fake db").unwrap();

        let prompter = MockPrompter::new(vec![false], vec![], vec![]);
        let result = should_initialize_db(&db_path, false, &prompter).unwrap();
        assert!(!result, "should skip when user declines");
    }

    // T-INIT-024: existing DB migrated when --force is set.
    #[test]
    fn test_should_initialize_db_force_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("y-agent.db");
        // Pre-create the file.
        std::fs::write(&db_path, "fake db").unwrap();

        let prompter = NonInteractivePrompter;
        let result = should_initialize_db(&db_path, true, &prompter).unwrap();
        assert!(result, "should proceed when force is set");
    }

    // T-INIT-025: should_initialize_db returns true for new database.
    #[test]
    fn test_should_initialize_db_new_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("nonexistent.db");

        let prompter = NonInteractivePrompter;
        let result = should_initialize_db(&db_path, false, &prompter).unwrap();
        assert!(result, "should proceed for non-existent database");
    }
}
