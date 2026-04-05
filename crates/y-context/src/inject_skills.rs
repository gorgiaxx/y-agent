//! `InjectSkills` pipeline stage (priority 400).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Dynamically injects active skill descriptions into the context pipeline.
//! At `provide()` time, reads `PromptContext.active_skills` and loads content
//! from a `FilesystemSkillStore`. Each skill root document is kept under
//! 2,000 tokens per design principle 2.4 (Token Efficiency).
//!
//! ## Template Variable Expansion
//!
//! Skills that contain companion files (scripts, data, templates) need path
//! context at injection time. When a skill's `root_content` contains `{{}}`
//! template markers OR its installed directory has companion files beyond
//! the standard layout, the following variables are expanded:
//!
//! | Variable | Description |
//! |---|---|
//! | `{{WORKSPACE}}` | Current session workspace path (CWD fallback) |
//! | `{{SKILL_PATH}}` | Absolute path to this skill's installed directory |
//! | `{{PYTHON_PATH}}` | Path to the runtime's `uv` binary |
//! | `{{PYTHON_VENV}}` | Path to the Python virtual environment directory |
//! | `{{BUN_PATH}}` | Path to the runtime's `bun` binary |

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use y_prompt::PromptContext;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};
use crate::system_prompt::VenvPromptInfo;

/// Maximum tokens per individual skill description (design principle 2.4).
const MAX_TOKENS_PER_SKILL: u32 = 2_000;

/// Files and directories that are part of the standard skill layout.
/// Anything beyond these in the skill directory is considered a companion file.
const STANDARD_SKILL_FILES: &[&str] = &["skill.toml", "root.md", "lineage.toml", "details"];

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------------
// Template variable support
// ---------------------------------------------------------------------------

/// Runtime template variables for skill content expansion.
///
/// Built once per `provide()` call from the current `PromptContext` and
/// `VenvPromptInfo`, then applied to each skill that needs template expansion.
#[derive(Debug, Clone, Default)]
pub struct SkillTemplateVars {
    /// Key-value pairs for template expansion (e.g. `("{{WORKSPACE}}", "/path")`).
    vars: Vec<(String, String)>,
}

impl SkillTemplateVars {
    /// Build template variables from the current runtime context.
    ///
    /// `workspace` is the session's workspace directory (falls back to CWD).
    /// `venv_info` provides Python/Bun runtime paths.
    pub fn from_context(workspace: &str, venv_info: &VenvPromptInfo) -> Self {
        let mut vars = Vec::with_capacity(5);

        vars.push(("{{WORKSPACE}}".to_string(), workspace.to_string()));

        // Python runtime paths.
        if let Some(ref py) = venv_info.python {
            vars.push(("{{PYTHON_PATH}}".to_string(), py.uv_path.clone()));
            vars.push(("{{PYTHON_VENV}}".to_string(), py.venv_dir.clone()));
        } else {
            vars.push(("{{PYTHON_PATH}}".to_string(), String::new()));
            vars.push(("{{PYTHON_VENV}}".to_string(), String::new()));
        }

        // Bun runtime path.
        if let Some(ref bun) = venv_info.bun {
            vars.push(("{{BUN_PATH}}".to_string(), bun.bun_path.clone()));
        } else {
            vars.push(("{{BUN_PATH}}".to_string(), String::new()));
        }

        Self { vars }
    }

    /// Expand template variables in the given content.
    ///
    /// `skill_path` is the absolute path to the skill's installed directory,
    /// added as `{{SKILL_PATH}}` per-skill.
    pub fn expand(&self, content: &str, skill_path: &str) -> String {
        let mut result = content.to_string();

        // Per-skill variable.
        result = result.replace("{{SKILL_PATH}}", skill_path);

        // Shared runtime variables.
        for (key, val) in &self.vars {
            result = result.replace(key, val);
        }

        result
    }

    /// Check whether the content contains any `{{` template markers.
    pub fn content_has_templates(content: &str) -> bool {
        content.contains("{{")
    }
}

/// Check whether a skill directory contains companion files beyond the
/// standard layout (`skill.toml`, `root.md`, `lineage.toml`, `details/`).
///
/// Returns `true` if any non-standard files or directories are found,
/// indicating the skill ships scripts, data, or template files that may
/// reference runtime paths.
pub fn has_companion_files(skill_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(skill_dir) else {
        return false;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files/dirs.
        if name_str.starts_with('.') {
            continue;
        }

        if !STANDARD_SKILL_FILES.contains(&name_str.as_ref()) {
            return true;
        }
    }

    false
}

/// Determine whether template expansion should be applied to a skill.
///
/// Returns `true` when either:
/// 1. The skill's `root_content` contains `{{` template markers, OR
/// 2. The skill directory has companion files (scripts, data, templates)
fn needs_template_expansion(root_content: &str, skill_dir: &Path) -> bool {
    SkillTemplateVars::content_has_templates(root_content) || has_companion_files(skill_dir)
}

// ---------------------------------------------------------------------------
// SkillSummary (for static injection)
// ---------------------------------------------------------------------------

/// Summary of a skill available to the agent.
#[derive(Debug, Clone)]
pub struct SkillSummary {
    /// Skill name / identifier.
    pub name: String,
    /// Short description of what this skill does.
    pub description: String,
    /// Trigger conditions or when this skill is applicable.
    pub triggers: Vec<String>,
}

// ---------------------------------------------------------------------------
// InjectSkills (dynamic, filesystem-backed)
// ---------------------------------------------------------------------------

/// `InjectSkills` -- dynamically injects active skill descriptions into context.
///
/// Runs at priority 400 (`INJECT_SKILLS`).
///
/// At `provide()` time, reads `PromptContext.active_skills` and loads each
/// skill's manifest from the on-disk skill store to get its `root_content`.
/// When a skill contains companion files or uses `{{}}` template syntax,
/// runtime template variables are expanded before injection.
pub struct InjectSkills {
    /// Shared prompt context to read `active_skills` from.
    prompt_context: Arc<RwLock<PromptContext>>,
    /// Path to the skills store directory (e.g. `~/.config/y-agent/skills/`).
    skills_dir: PathBuf,
    /// Virtual environment info for template variable expansion.
    venv_info: VenvPromptInfo,
}

impl InjectSkills {
    /// Create a new dynamic `InjectSkills` provider.
    pub fn new(
        prompt_context: Arc<RwLock<PromptContext>>,
        skills_dir: PathBuf,
        venv_info: VenvPromptInfo,
    ) -> Self {
        Self {
            prompt_context,
            skills_dir,
            venv_info,
        }
    }

    /// Create a provider from a static list of skill summaries (for tests).
    pub fn from_summaries(skills: Vec<SkillSummary>) -> InjectSkillsStatic {
        InjectSkillsStatic { skills }
    }

    /// Format a `ContextItem` from skill name, description and content.
    fn format_skill_item(name: &str, description: &str, root_content: &str) -> ContextItem {
        let formatted = format!("### Skill: {name}\n{description}\n\n{root_content}");

        let mut tokens = estimate_tokens(&formatted);

        // Enforce per-skill token limit (design principle 2.4).
        let content = if tokens > MAX_TOKENS_PER_SKILL {
            let max_chars = (MAX_TOKENS_PER_SKILL as usize) * 4;
            let truncated = if formatted.len() > max_chars {
                format!("{}... [truncated]", &formatted[..max_chars])
            } else {
                formatted
            };
            tokens = estimate_tokens(&truncated);
            truncated
        } else {
            formatted
        };

        ContextItem {
            category: ContextCategory::Skills,
            content,
            token_estimate: tokens,
            priority: 400,
        }
    }

    /// Resolve the workspace path: prefer `PromptContext.working_directory`,
    /// fall back to the process CWD.
    fn resolve_workspace(prompt_ctx: &PromptContext) -> String {
        prompt_ctx
            .working_directory
            .as_deref()
            .filter(|s| !s.is_empty())
            .map_or_else(
                || {
                    std::env::current_dir()
                        .map_or_else(|_| ".".to_string(), |p| p.display().to_string())
                },
                String::from,
            )
    }
}

#[async_trait]
impl ContextProvider for InjectSkills {
    fn name(&self) -> &'static str {
        "inject_skills"
    }

    fn priority(&self) -> u32 {
        400
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        let (active_skills, workspace) = {
            let prompt_ctx = self.prompt_context.read().await;
            let skills = prompt_ctx.active_skills.clone();
            let ws = Self::resolve_workspace(&prompt_ctx);
            (skills, ws)
        };

        if active_skills.is_empty() {
            return Ok(());
        }

        // Try to load skills from the filesystem skill store.
        if !self.skills_dir.exists() {
            tracing::warn!(
                skills_dir = %self.skills_dir.display(),
                "skills directory not found; skipping skill injection"
            );
            return Ok(());
        }

        let store = match y_skills::FilesystemSkillStore::new(&self.skills_dir) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to open skill store; skipping skill injection");
                return Ok(());
            }
        };

        // Build template vars once for all skills in this turn.
        let template_vars = SkillTemplateVars::from_context(&workspace, &self.venv_info);

        let mut injected = 0;
        for skill_name in &active_skills {
            match store.load_skill(skill_name) {
                Ok(manifest) => {
                    let skill_dir = self.skills_dir.join(&manifest.name);
                    let skill_path = skill_dir.display().to_string();

                    // Apply template expansion if the skill needs it.
                    let root_content =
                        if needs_template_expansion(&manifest.root_content, &skill_dir) {
                            tracing::debug!(
                                skill = %skill_name,
                                "applying template variable expansion"
                            );
                            template_vars.expand(&manifest.root_content, &skill_path)
                        } else {
                            manifest.root_content.clone()
                        };

                    let item = Self::format_skill_item(
                        &manifest.name,
                        &manifest.description,
                        &root_content,
                    );
                    ctx.add(item);
                    injected += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        skill = %skill_name,
                        error = %e,
                        "failed to load skill manifest; skipping"
                    );
                }
            }
        }

        if injected > 0 {
            tracing::debug!(skills = injected, "skill context injected");
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Static variant (for tests and non-filesystem use cases)
// ---------------------------------------------------------------------------

/// Static version of `InjectSkills` -- takes a fixed list of skill summaries.
/// Used primarily for testing.
pub struct InjectSkillsStatic {
    skills: Vec<SkillSummary>,
}

#[async_trait]
impl ContextProvider for InjectSkillsStatic {
    fn name(&self) -> &'static str {
        "inject_skills"
    }

    fn priority(&self) -> u32 {
        400
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        if self.skills.is_empty() {
            return Ok(());
        }

        for skill in &self.skills {
            let triggers = if skill.triggers.is_empty() {
                String::new()
            } else {
                format!("\nTriggers: {}", skill.triggers.join(", "))
            };

            let formatted = format!(
                "### Skill: {}\n{}{}",
                skill.name, skill.description, triggers
            );

            let mut tokens = estimate_tokens(&formatted);

            let content = if tokens > MAX_TOKENS_PER_SKILL {
                let max_chars = (MAX_TOKENS_PER_SKILL as usize) * 4;
                let truncated = if formatted.len() > max_chars {
                    format!("{}... [truncated]", &formatted[..max_chars])
                } else {
                    formatted
                };
                tokens = estimate_tokens(&truncated);
                truncated
            } else {
                formatted
            };

            ctx.add(ContextItem {
                category: ContextCategory::Skills,
                content,
                token_estimate: tokens,
                priority: 400,
            });
        }

        tracing::debug!(skills = self.skills.len(), "skill context injected");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P1-05: Provider name and priority; injects skill descriptions.
    #[tokio::test]
    async fn test_provider_name_priority_and_inject() {
        let provider = InjectSkills::from_summaries(vec![
            SkillSummary {
                name: "code_review".into(),
                description: "Reviews code for best practices.".into(),
                triggers: vec!["review".into(), "check code".into()],
            },
            SkillSummary {
                name: "refactor".into(),
                description: "Refactors code to improve structure.".into(),
                triggers: vec![],
            },
        ]);

        assert_eq!(provider.name(), "inject_skills");
        assert_eq!(provider.priority(), 400);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 2);
        assert!(ctx
            .items
            .iter()
            .all(|i| i.category == ContextCategory::Skills));
        assert!(ctx.items[0].content.contains("code_review"));
        assert!(ctx.items[0].content.contains("Triggers:"));
        assert!(ctx.items[1].content.contains("refactor"));
        // No triggers for refactor skill.
        assert!(!ctx.items[1].content.contains("Triggers:"));
    }

    /// Empty skills produce no items.
    #[tokio::test]
    async fn test_empty_skills() {
        let provider = InjectSkills::from_summaries(vec![]);
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    /// Skill descriptions exceeding 2,000 tokens are truncated.
    #[tokio::test]
    async fn test_skill_token_limit() {
        let long_desc = "x".repeat(40_000); // ~10,000 tokens
        let provider = InjectSkills::from_summaries(vec![SkillSummary {
            name: "verbose_skill".into(),
            description: long_desc,
            triggers: vec![],
        }]);

        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 1);
        assert!(ctx.items[0].token_estimate <= MAX_TOKENS_PER_SKILL + 10); // allow overhead
        assert!(ctx.items[0].content.contains("[truncated]"));
    }

    /// Dynamic provider with no active skills produces no items.
    #[tokio::test]
    async fn test_dynamic_no_active_skills() {
        let prompt_context = Arc::new(RwLock::new(PromptContext::default()));
        let provider = InjectSkills::new(
            prompt_context,
            PathBuf::from("/nonexistent"),
            VenvPromptInfo::default(),
        );
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    // -----------------------------------------------------------------------
    // Template variable expansion tests
    // -----------------------------------------------------------------------

    /// Template variables in skill content are expanded.
    #[test]
    fn test_template_expansion_with_venv_info() {
        use crate::system_prompt::{BunVenvPromptInfo, PythonVenvPromptInfo};

        let venv_info = VenvPromptInfo {
            python: Some(PythonVenvPromptInfo {
                uv_path: "/usr/local/bin/uv".into(),
                python_version: "3.12".into(),
                venv_dir: "/home/user/.venv".into(),
                working_dir: "/tmp".into(),
            }),
            bun: Some(BunVenvPromptInfo {
                bun_path: "/usr/local/bin/bun".into(),
                bun_version: "1.1".into(),
                working_dir: "/tmp".into(),
            }),
        };

        let vars = SkillTemplateVars::from_context("/home/user/project", &venv_info);

        let content = "Run {{PYTHON_PATH}} run {{SKILL_PATH}}/scripts/convert.py \
                        in {{WORKSPACE}} or use {{BUN_PATH}} for JS. \
                        Venv at {{PYTHON_VENV}}.";

        let result = vars.expand(content, "/opt/skills/xlsx-processor");

        assert!(result.contains("/usr/local/bin/uv"));
        assert!(result.contains("/opt/skills/xlsx-processor/scripts/convert.py"));
        assert!(result.contains("/home/user/project"));
        assert!(result.contains("/usr/local/bin/bun"));
        assert!(result.contains("/home/user/.venv"));
        assert!(!result.contains("{{"));
    }

    /// When venv info is unavailable, template variables are replaced with empty strings.
    #[test]
    fn test_template_expansion_partial_vars() {
        let venv_info = VenvPromptInfo::default(); // no Python, no Bun
        let vars = SkillTemplateVars::from_context("/workspace", &venv_info);

        let content = "python: {{PYTHON_PATH}}, bun: {{BUN_PATH}}, ws: {{WORKSPACE}}";
        let result = vars.expand(content, "/skills/test");

        assert_eq!(result, "python: , bun: , ws: /workspace");
    }

    /// Content without `{{` markers is detected as not needing templates.
    #[test]
    fn test_content_has_templates_detection() {
        assert!(SkillTemplateVars::content_has_templates(
            "Run {{PYTHON_PATH}} script"
        ));
        assert!(!SkillTemplateVars::content_has_templates(
            "Simple instructions without templates"
        ));
    }

    /// Companion file detection identifies non-standard files.
    #[test]
    fn test_has_companion_files_detection() {
        let dir = tempfile::tempdir().unwrap();

        // Standard layout only -- no companions.
        std::fs::write(dir.path().join("skill.toml"), "").unwrap();
        std::fs::write(dir.path().join("root.md"), "").unwrap();
        std::fs::write(dir.path().join("lineage.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("details")).unwrap();
        assert!(!has_companion_files(dir.path()));

        // Add a script -- now has companions.
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts/convert.py"), "").unwrap();
        assert!(has_companion_files(dir.path()));
    }

    /// Companion files in the top level are detected.
    #[test]
    fn test_has_companion_files_top_level() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.toml"), "").unwrap();
        std::fs::write(dir.path().join("root.md"), "").unwrap();

        // Add a non-standard file (e.g. LICENSE.txt).
        std::fs::write(dir.path().join("LICENSE.txt"), "MIT").unwrap();
        assert!(has_companion_files(dir.path()));
    }

    /// Hidden files are ignored by companion detection.
    #[test]
    fn test_has_companion_files_ignores_hidden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.toml"), "").unwrap();
        std::fs::write(dir.path().join("root.md"), "").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "").unwrap();
        assert!(!has_companion_files(dir.path()));
    }

    /// Non-existent directory returns false for companion detection.
    #[test]
    fn test_has_companion_files_nonexistent() {
        assert!(!has_companion_files(Path::new("/nonexistent/skill")));
    }

    /// `needs_template_expansion` triggers on template markers in content.
    #[test]
    fn test_needs_expansion_with_template_markers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.toml"), "").unwrap();
        std::fs::write(dir.path().join("root.md"), "").unwrap();

        // Content has templates, no companion files.
        assert!(needs_template_expansion(
            "Use {{PYTHON_PATH}} to run",
            dir.path()
        ));

        // No templates, no companions.
        assert!(!needs_template_expansion("Simple instructions", dir.path()));
    }

    /// `needs_template_expansion` triggers on companion files even without markers.
    #[test]
    fn test_needs_expansion_with_companion_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skill.toml"), "").unwrap();
        std::fs::write(dir.path().join("root.md"), "").unwrap();
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::write(dir.path().join("scripts/run.py"), "").unwrap();

        // No template markers in content, but has companion files.
        assert!(needs_template_expansion("Run the script", dir.path()));
    }
}
