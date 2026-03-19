//! First-run initialization: seed config, prompts, skills, and agents.
//!
//! Shared by both `y-cli` (`y-agent init`) and `y-gui` (automatic on startup).
//! All seed operations are idempotent: existing files are never overwritten.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

// ---------------------------------------------------------------------------
// Embedded config templates (compile-time)
// ---------------------------------------------------------------------------

const EXAMPLE_Y_AGENT: &str = include_str!("../../../config/y-agent.example.toml");
const EXAMPLE_PROVIDERS: &str = include_str!("../../../config/providers.example.toml");
const EXAMPLE_STORAGE: &str = include_str!("../../../config/storage.example.toml");
const EXAMPLE_SESSION: &str = include_str!("../../../config/session.example.toml");
const EXAMPLE_RUNTIME: &str = include_str!("../../../config/runtime.example.toml");
const EXAMPLE_KNOWLEDGE: &str = include_str!("../../../config/knowledge.example.toml");
const EXAMPLE_HOOKS: &str = include_str!("../../../config/hooks.example.toml");
const EXAMPLE_TOOLS: &str = include_str!("../../../config/tools.example.toml");
const EXAMPLE_BROWSER: &str = include_str!("../../../config/browser.example.toml");
const EXAMPLE_GUARDRAILS: &str = include_str!("../../../config/guardrails.example.toml");

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
// Embedded builtin skills (compile-time)
// ---------------------------------------------------------------------------

struct BuiltinSkillFile {
    relative_path: &'static str,
    content: &'static str,
}

struct BuiltinSkill {
    name: &'static str,
    files: &'static [BuiltinSkillFile],
}

const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        name: "humanizer-zh",
        files: &[
            BuiltinSkillFile {
                relative_path: "skill.toml",
                content: include_str!("../../../builtin-skills/humanizer-zh/skill.toml"),
            },
            BuiltinSkillFile {
                relative_path: "root.md",
                content: include_str!("../../../builtin-skills/humanizer-zh/root.md"),
            },
            BuiltinSkillFile {
                relative_path: "details/tone-guidelines.md",
                content: include_str!("../../../builtin-skills/humanizer-zh/details/tone-guidelines.md"),
            },
        ],
    },
    BuiltinSkill {
        name: "code-review-rust",
        files: &[
            BuiltinSkillFile {
                relative_path: "skill.toml",
                content: include_str!("../../../builtin-skills/code-review-rust/skill.toml"),
            },
            BuiltinSkillFile {
                relative_path: "root.md",
                content: include_str!("../../../builtin-skills/code-review-rust/root.md"),
            },
            BuiltinSkillFile {
                relative_path: "details/error-handling-patterns.md",
                content: include_str!("../../../builtin-skills/code-review-rust/details/error-handling-patterns.md"),
            },
            BuiltinSkillFile {
                relative_path: "details/unsafe-review-checklist.md",
                content: include_str!("../../../builtin-skills/code-review-rust/details/unsafe-review-checklist.md"),
            },
        ],
    },
];

// ---------------------------------------------------------------------------
// Init report
// ---------------------------------------------------------------------------

/// Summary of what was seeded during initialization.
#[derive(Debug, Default)]
pub struct InitReport {
    /// Config files that were created.
    pub configs_created: Vec<String>,
    /// Prompt files that were seeded.
    pub prompts_seeded: Vec<String>,
    /// Skills that were seeded.
    pub skills_seeded: Vec<String>,
    /// Agent definitions that were seeded.
    pub agents_seeded: Vec<String>,
    /// Whether any work was done at all.
    pub was_first_run: bool,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Ensure the y-agent environment is initialized.
///
/// Creates directories, seeds config files, prompts, skills, and agents
/// if they don't already exist. All operations are idempotent — existing
/// files are never overwritten.
///
/// - `config_dir`: e.g. `~/.config/y-agent/`
/// - `data_dir`: e.g. `~/.local/state/y-agent/`
pub fn ensure_initialized(config_dir: &Path, data_dir: &Path) -> Result<InitReport> {
    let mut report = InitReport::default();

    // -- Ensure directories exist ---------------------------------------------
    ensure_directories(config_dir, data_dir)?;

    // -- Seed config files ----------------------------------------------------
    report.configs_created = seed_config_files(config_dir)?;

    // -- Seed prompt files ----------------------------------------------------
    report.prompts_seeded = seed_builtin_prompts(config_dir)?;

    // -- Seed builtin skills --------------------------------------------------
    report.skills_seeded = seed_builtin_skills(data_dir)?;

    // -- Seed agent definitions -----------------------------------------------
    report.agents_seeded = seed_builtin_agents(config_dir)?;

    report.was_first_run = !report.configs_created.is_empty()
        || !report.prompts_seeded.is_empty()
        || !report.skills_seeded.is_empty()
        || !report.agents_seeded.is_empty();

    if report.was_first_run {
        info!(
            configs = report.configs_created.len(),
            prompts = report.prompts_seeded.len(),
            skills = report.skills_seeded.len(),
            agents = report.agents_seeded.len(),
            "First-run initialization completed"
        );
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Ensure required directories exist.
fn ensure_directories(config_dir: &Path, data_dir: &Path) -> Result<()> {
    for dir in &[
        config_dir.to_path_buf(),
        data_dir.to_path_buf(),
        data_dir.join("transcripts"),
    ] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory: {}", dir.display()))?;
    }
    Ok(())
}

/// Seed config template files into the config directory.
/// Only writes files that don't already exist.
fn seed_config_files(config_dir: &Path) -> Result<Vec<String>> {
    let mut created = Vec::new();

    for (name, content) in CONFIG_TEMPLATES {
        let dest = config_dir.join(name);
        if !dest.exists() {
            std::fs::write(&dest, content)
                .with_context(|| format!("writing {}", dest.display()))?;
            created.push((*name).to_string());
        }
    }

    Ok(created)
}

/// Seed builtin prompt files into `<config_dir>/prompts/`.
fn seed_builtin_prompts(config_dir: &Path) -> Result<Vec<String>> {
    let prompts_dir = config_dir.join("prompts");
    std::fs::create_dir_all(&prompts_dir)
        .with_context(|| format!("creating prompts directory: {}", prompts_dir.display()))?;

    let mut seeded = Vec::new();

    for &(filename, content) in y_prompt::BUILTIN_PROMPT_FILES {
        let dest = prompts_dir.join(filename);
        if !dest.exists() {
            std::fs::write(&dest, content)
                .with_context(|| format!("writing {}", dest.display()))?;
            seeded.push(filename.to_string());
        }
    }

    Ok(seeded)
}

/// Seed builtin skills into `<data_dir>/skills/`.
fn seed_builtin_skills(data_dir: &Path) -> Result<Vec<String>> {
    let skills_dir = data_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)
        .with_context(|| format!("creating skills directory: {}", skills_dir.display()))?;

    let mut seeded = Vec::new();

    for skill in BUILTIN_SKILLS {
        let skill_dir = skills_dir.join(skill.name);
        if skill_dir.exists() {
            continue;
        }

        for file in skill.files {
            let file_path = skill_dir.join(file.relative_path);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating dir: {}", parent.display()))?;
            }
            std::fs::write(&file_path, file.content)
                .with_context(|| format!("writing {}", file_path.display()))?;
        }

        // Create empty lineage.toml (required by standard).
        std::fs::write(
            skill_dir.join("lineage.toml"),
            "# Transformation lineage (builtin skill)\n",
        )
        .with_context(|| format!("writing lineage.toml for {}", skill.name))?;

        seeded.push(skill.name.to_string());
    }

    Ok(seeded)
}

/// Seed builtin agent definitions into `<config_dir>/agents/`.
fn seed_builtin_agents(config_dir: &Path) -> Result<Vec<String>> {
    use y_agent::agent::registry::AgentRegistry;

    let agents_dir = config_dir.join("agents");
    std::fs::create_dir_all(&agents_dir)
        .with_context(|| format!("creating agents directory: {}", agents_dir.display()))?;

    let mut seeded = Vec::new();

    for (name, content) in AgentRegistry::builtin_toml_sources() {
        let dest = agents_dir.join(format!("{name}.toml"));
        if !dest.exists() {
            std::fs::write(&dest, content)
                .with_context(|| format!("writing {}", dest.display()))?;
            seeded.push(name.to_string());
        }
    }

    Ok(seeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_initialized_creates_all_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");

        let report = ensure_initialized(&config_dir, &data_dir).unwrap();

        assert!(report.was_first_run);
        assert!(!report.configs_created.is_empty());
        assert!(!report.prompts_seeded.is_empty());
        assert!(!report.skills_seeded.is_empty());

        // Verify files exist
        assert!(config_dir.join("y-agent.toml").exists());
        assert!(config_dir.join("providers.toml").exists());
        assert!(config_dir.join("prompts").is_dir());
        assert!(data_dir.join("skills").is_dir());
    }

    #[test]
    fn ensure_initialized_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");

        let report1 = ensure_initialized(&config_dir, &data_dir).unwrap();
        assert!(report1.was_first_run);

        let report2 = ensure_initialized(&config_dir, &data_dir).unwrap();
        assert!(!report2.was_first_run);
        assert!(report2.configs_created.is_empty());
        assert!(report2.prompts_seeded.is_empty());
        assert!(report2.skills_seeded.is_empty());
    }
}
