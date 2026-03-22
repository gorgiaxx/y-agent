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
// Init report
// ---------------------------------------------------------------------------

/// Summary of what was seeded during initialization.
#[derive(Debug, Default)]
pub struct InitReport {
    /// Config files that were created.
    pub configs_created: Vec<String>,
    /// Prompt files that were seeded.
    pub prompts_seeded: Vec<String>,
    /// Skills that were copied from the source directory.
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
/// if they don't already exist. All operations are idempotent -- existing
/// files are never overwritten.
///
/// - `config_dir`: e.g. `~/.config/y-agent/`
/// - `data_dir`: e.g. `~/.local/state/y-agent/`
/// - `skills_source_dir`: optional path to a directory containing skill
///   subdirectories to copy into `<data_dir>/skills/`. When `None`, skill
///   seeding is skipped.
pub fn ensure_initialized(
    config_dir: &Path,
    data_dir: &Path,
    skills_source_dir: Option<&Path>,
) -> Result<InitReport> {
    let mut report = InitReport::default();

    // -- Ensure directories exist ---------------------------------------------
    ensure_directories(config_dir, data_dir)?;

    // -- Seed config files ----------------------------------------------------
    report.configs_created = seed_config_files(config_dir)?;

    // -- Seed prompt files ----------------------------------------------------
    report.prompts_seeded = seed_builtin_prompts(config_dir)?;

    // -- Copy skills from source directory ------------------------------------
    if let Some(source) = skills_source_dir {
        report.skills_seeded = copy_skills_from_source(source, data_dir)?;
    }

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

/// Copy skill directories from a source directory into `<data_dir>/skills/`.
///
/// Each immediate subdirectory in `source_dir` is treated as a skill.
/// Skills that already exist in the destination are skipped (idempotent).
/// Returns the list of skill names that were copied.
pub fn copy_skills_from_source(source_dir: &Path, data_dir: &Path) -> Result<Vec<String>> {
    let skills_dir = data_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)
        .with_context(|| format!("creating skills directory: {}", skills_dir.display()))?;

    if !source_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut seeded = Vec::new();

    let entries = std::fs::read_dir(source_dir)
        .with_context(|| format!("reading skills source: {}", source_dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Only process directories (each is a skill).
        if !path.is_dir() {
            continue;
        }

        let skill_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };

        let dest_path = skills_dir.join(&skill_name);

        // Skip if already exists (don't overwrite user modifications).
        if dest_path.exists() {
            continue;
        }

        copy_dir_recursive(&path, &dest_path).with_context(|| {
            format!("copying skill '{}' to {}", skill_name, dest_path.display())
        })?;

        seeded.push(skill_name);
    }

    Ok(seeded)
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("creating directory: {}", dst.display()))?;

    for entry in
        std::fs::read_dir(src).with_context(|| format!("reading directory: {}", src.display()))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).with_context(|| {
                format!("copying {} -> {}", src_path.display(), dst_path.display())
            })?;
        }
    }

    Ok(())
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

    /// Create a temporary skills source directory with a sample skill.
    fn create_sample_skills_source(base: &Path) -> std::path::PathBuf {
        let skills_src = base.join("skills-source");
        let skill_dir = skills_src.join("test-skill");
        std::fs::create_dir_all(skill_dir.join("details")).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            "[meta]\nname = \"test-skill\"\n",
        )
        .unwrap();
        std::fs::write(skill_dir.join("root.md"), "# Test Skill\n").unwrap();
        std::fs::write(skill_dir.join("details/guide.md"), "## Guide\n").unwrap();
        skills_src
    }

    #[test]
    fn ensure_initialized_creates_all_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        let skills_src = create_sample_skills_source(tmp.path());

        let report = ensure_initialized(&config_dir, &data_dir, Some(&skills_src)).unwrap();

        assert!(report.was_first_run);
        assert!(!report.configs_created.is_empty());
        assert!(!report.prompts_seeded.is_empty());
        assert!(!report.skills_seeded.is_empty());

        // Verify files exist
        assert!(config_dir.join("y-agent.toml").exists());
        assert!(config_dir.join("providers.toml").exists());
        assert!(config_dir.join("prompts").is_dir());
        assert!(data_dir.join("skills").is_dir());
        assert!(data_dir.join("skills/test-skill/skill.toml").exists());
        assert!(data_dir.join("skills/test-skill/details/guide.md").exists());
    }

    #[test]
    fn ensure_initialized_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");
        let skills_src = create_sample_skills_source(tmp.path());

        let report1 = ensure_initialized(&config_dir, &data_dir, Some(&skills_src)).unwrap();
        assert!(report1.was_first_run);

        let report2 = ensure_initialized(&config_dir, &data_dir, Some(&skills_src)).unwrap();
        assert!(!report2.was_first_run);
        assert!(report2.configs_created.is_empty());
        assert!(report2.prompts_seeded.is_empty());
        assert!(report2.skills_seeded.is_empty());
    }

    #[test]
    fn ensure_initialized_without_skills_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join("config");
        let data_dir = tmp.path().join("data");

        let report = ensure_initialized(&config_dir, &data_dir, None).unwrap();

        assert!(report.was_first_run);
        assert!(report.skills_seeded.is_empty());
        // Other seeding still works.
        assert!(!report.configs_created.is_empty());
    }

    #[test]
    fn copy_skills_from_source_skips_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let skills_src = create_sample_skills_source(tmp.path());

        // First copy succeeds.
        let seeded = copy_skills_from_source(&skills_src, &data_dir).unwrap();
        assert_eq!(seeded, vec!["test-skill"]);

        // Second copy skips existing.
        let seeded2 = copy_skills_from_source(&skills_src, &data_dir).unwrap();
        assert!(seeded2.is_empty());
    }

    #[test]
    fn copy_skills_nonexistent_source_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let fake_src = tmp.path().join("nonexistent-skills");

        let seeded = copy_skills_from_source(&fake_src, &data_dir).unwrap();
        assert!(seeded.is_empty());
    }
}
