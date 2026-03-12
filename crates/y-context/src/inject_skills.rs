//! `InjectSkills` pipeline stage (priority 400).
//!
//! Design reference: context-session-design.md §Pipeline Stages
//!
//! Injects active skill descriptions into the context pipeline.
//! Each skill root document is kept under 2,000 tokens per design
//! principle 2.4 (Token Efficiency).

use async_trait::async_trait;

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

/// `InjectSkills` — injects available skill descriptions into context.
///
/// Runs at priority 400 (`INJECT_SKILLS`).
pub struct InjectSkills {
    /// Active skills to inject.
    skills: Vec<SkillSummary>,
}

impl InjectSkills {
    /// Create a new `InjectSkills` provider.
    pub fn new(skills: Vec<SkillSummary>) -> Self {
        Self { skills }
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
        let provider = InjectSkills::new(vec![
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
        let provider = InjectSkills::new(vec![]);
        let mut ctx = AssembledContext::default();
        provider.provide(&mut ctx).await.unwrap();
        assert!(ctx.items.is_empty());
    }

    /// Skill descriptions exceeding 2,000 tokens are truncated.
    #[tokio::test]
    async fn test_skill_token_limit() {
        let long_desc = "x".repeat(40_000); // ~10,000 tokens
        let provider = InjectSkills::new(vec![SkillSummary {
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
}
