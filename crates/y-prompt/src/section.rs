//! `PromptSection`: structured, typed, prioritized, token-budgeted prompt unit.

use serde::{Deserialize, Serialize};

/// Unique identifier for a prompt section.
pub type SectionId = String;

/// Unique identifier for a prompt template.
pub type TemplateId = String;

/// Semantic category for grouping and budget allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionCategory {
    /// Who the agent is (name, role, persona).
    Identity,
    /// Dynamic environment (datetime, OS, workspace).
    Context,
    /// How the agent should act (guidelines, safety, mode-specific).
    Behavioral,
    /// Domain knowledge (persona expertise, custom instructions).
    Domain,
}

/// Source of section content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentSource {
    /// Content is inline in the section definition.
    Inline(String),
    /// Content lives in a file on disk.
    File(String),
    /// Content is stored in the `SectionStore` by key.
    Store(String),
}

/// Condition controlling section inclusion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionCondition {
    /// Always include.
    Always,
    /// Include when the agent is in the given mode.
    ModeIs(String),
    /// Include when the agent is NOT in the given mode.
    ModeNot(String),
    /// Include when a specific skill is active.
    HasSkill(String),
    /// Include when a specific tool is available.
    HasTool(String),
    /// Include when a configuration flag is set.
    ConfigFlag(String),
    /// All sub-conditions must be true.
    And(Vec<SectionCondition>),
    /// At least one sub-condition must be true.
    Or(Vec<SectionCondition>),
}

/// Runtime context for condition evaluation.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Current agent mode (e.g., "build", "plan", "explore", "general").
    pub agent_mode: String,
    /// Active skill IDs.
    pub active_skills: Vec<String>,
    /// Available tool names.
    pub available_tools: Vec<String>,
    /// Configuration flags (key-value).
    pub config_flags: std::collections::HashMap<String, bool>,
}

impl SectionCondition {
    /// Evaluate the condition against a prompt context.
    pub fn evaluate(&self, ctx: &PromptContext) -> bool {
        match self {
            Self::Always => true,
            Self::ModeIs(mode) => ctx.agent_mode == *mode,
            Self::ModeNot(mode) => ctx.agent_mode != *mode,
            Self::HasSkill(skill) => ctx.active_skills.contains(skill),
            Self::HasTool(tool) if tool == "*" => !ctx.available_tools.is_empty(),
            Self::HasTool(tool) => ctx.available_tools.contains(tool),
            Self::ConfigFlag(key) => ctx.config_flags.get(key).copied().unwrap_or(false),
            Self::And(conditions) => conditions.iter().all(|c| c.evaluate(ctx)),
            Self::Or(conditions) => conditions.iter().any(|c| c.evaluate(ctx)),
        }
    }
}

/// A single prompt section — the atomic unit of prompt content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSection {
    /// Unique identifier (e.g., "core.identity", "core.datetime").
    pub id: SectionId,
    /// Content source — inline, file, or store key.
    pub content_source: ContentSource,
    /// Maximum tokens this section may consume.
    pub token_budget: u32,
    /// Assembly priority — lower values appear earlier in the prompt.
    pub priority: i32,
    /// When to include this section; None = always include.
    pub condition: Option<SectionCondition>,
    /// Semantic category.
    pub category: SectionCategory,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_ctx() -> PromptContext {
        PromptContext {
            agent_mode: "build".into(),
            active_skills: vec!["code_review".into()],
            available_tools: vec!["file_read".into(), "file_write".into()],
            config_flags: {
                let mut m = std::collections::HashMap::new();
                m.insert("persona.enabled".into(), true);
                m
            },
        }
    }

    #[test]
    fn test_condition_always() {
        let ctx = build_ctx();
        assert!(SectionCondition::Always.evaluate(&ctx));
    }

    #[test]
    fn test_condition_mode_is() {
        let ctx = build_ctx();
        assert!(SectionCondition::ModeIs("build".into()).evaluate(&ctx));
        assert!(!SectionCondition::ModeIs("plan".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_mode_not() {
        let ctx = build_ctx();
        assert!(SectionCondition::ModeNot("plan".into()).evaluate(&ctx));
        assert!(!SectionCondition::ModeNot("build".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_has_skill() {
        let ctx = build_ctx();
        assert!(SectionCondition::HasSkill("code_review".into()).evaluate(&ctx));
        assert!(!SectionCondition::HasSkill("translation".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_has_tool() {
        let ctx = build_ctx();
        assert!(SectionCondition::HasTool("file_read".into()).evaluate(&ctx));
        assert!(!SectionCondition::HasTool("web_search".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_config_flag() {
        let ctx = build_ctx();
        assert!(SectionCondition::ConfigFlag("persona.enabled".into()).evaluate(&ctx));
        assert!(!SectionCondition::ConfigFlag("debug.enabled".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_and() {
        let ctx = build_ctx();
        let cond = SectionCondition::And(vec![
            SectionCondition::ModeIs("build".into()),
            SectionCondition::HasTool("file_read".into()),
        ]);
        assert!(cond.evaluate(&ctx));

        let cond_fail = SectionCondition::And(vec![
            SectionCondition::ModeIs("build".into()),
            SectionCondition::HasTool("web_search".into()),
        ]);
        assert!(!cond_fail.evaluate(&ctx));
    }

    #[test]
    fn test_condition_or() {
        let ctx = build_ctx();
        let cond = SectionCondition::Or(vec![
            SectionCondition::ModeIs("plan".into()),
            SectionCondition::HasTool("file_read".into()),
        ]);
        assert!(cond.evaluate(&ctx));

        let cond_fail = SectionCondition::Or(vec![
            SectionCondition::ModeIs("plan".into()),
            SectionCondition::HasTool("web_search".into()),
        ]);
        assert!(!cond_fail.evaluate(&ctx));
    }

    #[test]
    fn test_condition_has_tool_wildcard_true() {
        let ctx = build_ctx(); // has file_read, file_write
        assert!(SectionCondition::HasTool("*".into()).evaluate(&ctx));
    }

    #[test]
    fn test_condition_has_tool_wildcard_false() {
        let ctx = PromptContext {
            available_tools: vec![],
            ..build_ctx()
        };
        assert!(!SectionCondition::HasTool("*".into()).evaluate(&ctx));
    }

    #[test]
    fn test_section_serialization() {
        let section = PromptSection {
            id: "core.identity".into(),
            content_source: ContentSource::Inline("You are an AI assistant.".into()),
            token_budget: 200,
            priority: 100,
            condition: Some(SectionCondition::Always),
            category: SectionCategory::Identity,
        };
        let json = serde_json::to_string(&section).unwrap();
        let deserialized: PromptSection = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "core.identity");
        assert_eq!(deserialized.priority, 100);
    }
}
