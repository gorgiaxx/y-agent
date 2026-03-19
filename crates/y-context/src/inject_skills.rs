//! `InjectSkills` pipeline stage (priority 400).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Dynamically injects active skill descriptions into the context pipeline.
//! At `provide()` time, reads `PromptContext.active_skills` and loads content
//! from a `FilesystemSkillStore`. Each skill root document is kept under
//! 2,000 tokens per design principle 2.4 (Token Efficiency).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use y_prompt::PromptContext;

use crate::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

/// Maximum tokens per individual skill description (design principle 2.4).
const MAX_TOKENS_PER_SKILL: u32 = 2_000;

/// Simple token estimation (4 chars per token).
fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

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

/// `InjectSkills` — dynamically injects active skill descriptions into context.
///
/// Runs at priority 400 (`INJECT_SKILLS`).
///
/// At `provide()` time, reads `PromptContext.active_skills` and loads each
/// skill's manifest from the on-disk skill store to get its `root_content`.
pub struct InjectSkills {
    /// Shared prompt context to read `active_skills` from.
    prompt_context: Arc<RwLock<PromptContext>>,
    /// Path to the skills store directory (e.g. `~/.config/y-agent/skills/`).
    skills_dir: PathBuf,
}

impl InjectSkills {
    /// Create a new dynamic `InjectSkills` provider.
    pub fn new(prompt_context: Arc<RwLock<PromptContext>>, skills_dir: PathBuf) -> Self {
        Self {
            prompt_context,
            skills_dir,
        }
    }

    /// Create a provider from a static list of skill summaries (for tests).
    pub fn from_summaries(skills: Vec<SkillSummary>) -> InjectSkillsStatic {
        InjectSkillsStatic { skills }
    }

    /// Format a ContextItem from skill name, description and content.
    fn format_skill_item(name: &str, description: &str, root_content: &str) -> ContextItem {
        let formatted = format!(
            "### Skill: {}\n{}\n\n{}",
            name, description, root_content
        );

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
        let active_skills = {
            let prompt_ctx = self.prompt_context.read().await;
            prompt_ctx.active_skills.clone()
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

        let mut injected = 0;
        for skill_name in &active_skills {
            match store.load_skill(skill_name) {
                Ok(manifest) => {
                    let item = Self::format_skill_item(
                        &manifest.name,
                        &manifest.description,
                        &manifest.root_content,
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

/// Static version of `InjectSkills` — takes a fixed list of skill summaries.
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
        let provider = InjectSkills::new(prompt_context, PathBuf::from("/nonexistent"));
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }
}
