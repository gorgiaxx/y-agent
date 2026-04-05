//! Bot context provider: injects persona sections into the context pipeline.
//!
//! Implements [`ContextProvider`] at priority 50 so persona content appears
//! before `BuildSystemPromptProvider` (priority 100) in the assembled prompt.

use async_trait::async_trait;

use y_context::pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipelineError, ContextProvider,
};

use super::persona::BotPersona;

/// Rough token estimate: ~4 chars per token for English text.
const CHARS_PER_TOKEN: u32 = 4;

/// Context provider that injects bot persona sections.
///
/// Created per-turn with a freshly loaded [`BotPersona`].
pub struct BotContextProvider {
    persona: BotPersona,
    /// Platform constraints text (e.g. "Max message length: 2000 characters").
    platform_constraints: String,
}

impl BotContextProvider {
    /// Create a new provider from a loaded persona.
    pub fn new(persona: BotPersona) -> Self {
        let max_len = persona.config.persona.messaging.max_response_length;
        let dialect = &persona.config.persona.messaging.markdown_dialect;
        let platform_constraints = format!(
            "## Platform Constraints\n\
             - Maximum message length: {max_len} characters. Split longer responses.\n\
             - Markdown dialect: {dialect}."
        );

        Self {
            persona,
            platform_constraints,
        }
    }

    /// Build the base prompt section that frames the persona files.
    fn base_prompt(&self) -> String {
        format!(
            "You are {name}, a personal assistant running inside y-agent.\n\
             The following persona files define who you are. Embody their tone and\n\
             guidelines. You may update these files to evolve your personality.",
            name = self.persona.name()
        )
    }

    /// Build messaging style guidelines from config.
    fn messaging_style(&self) -> String {
        let cfg = &self.persona.config.persona.messaging;
        let mut lines = Vec::new();
        lines.push("## Messaging Style".to_string());
        if cfg.prefer_short_responses {
            lines.push("- Prefer concise, conversational replies.".to_string());
        }
        lines.push(format!(
            "- Keep responses under {} characters when possible.",
            cfg.max_response_length
        ));
        lines.join("\n")
    }

    /// Estimate token count from a string.
    fn estimate_tokens(text: &str) -> u32 {
        let len = u32::try_from(text.len()).unwrap_or(u32::MAX);
        len / CHARS_PER_TOKEN
    }
}

#[async_trait]
impl ContextProvider for BotContextProvider {
    fn name(&self) -> &'static str {
        "BotContextProvider"
    }

    fn priority(&self) -> u32 {
        50
    }

    async fn provide(&self, ctx: &mut AssembledContext) -> Result<(), ContextPipelineError> {
        // 1. Base prompt (priority 10).
        let base = self.base_prompt();
        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: base.clone(),
            token_estimate: Self::estimate_tokens(&base),
            priority: 10,
        });

        // 2. SOUL.md (priority 20).
        if !self.persona.soul.is_empty() {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.persona.soul.clone(),
                token_estimate: Self::estimate_tokens(&self.persona.soul),
                priority: 20,
            });
        }

        // 3. IDENTITY.md (priority 25).
        if !self.persona.identity.is_empty() {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.persona.identity.clone(),
                token_estimate: Self::estimate_tokens(&self.persona.identity),
                priority: 25,
            });
        }

        // 4. USER.md (priority 30).
        if !self.persona.user.is_empty() {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.persona.user.clone(),
                token_estimate: Self::estimate_tokens(&self.persona.user),
                priority: 30,
            });
        }

        // 5. MEMORY.md (priority 35).
        if !self.persona.memory.is_empty() {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.persona.memory.clone(),
                token_estimate: Self::estimate_tokens(&self.persona.memory),
                priority: 35,
            });
        }

        // 6. BOOTSTRAP.md (priority 40, first-run only).
        if !self.persona.bootstrap.is_empty() {
            ctx.add(ContextItem {
                category: ContextCategory::SystemPrompt,
                content: self.persona.bootstrap.clone(),
                token_estimate: Self::estimate_tokens(&self.persona.bootstrap),
                priority: 40,
            });
        }

        // 7. Messaging style (priority 60).
        let style = self.messaging_style();
        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: style.clone(),
            token_estimate: Self::estimate_tokens(&style),
            priority: 60,
        });

        // 8. Platform constraints (priority 65).
        ctx.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: self.platform_constraints.clone(),
            token_estimate: Self::estimate_tokens(&self.platform_constraints),
            priority: 65,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_persona() -> BotPersona {
        BotPersona {
            config: super::super::config::BotConfig::default(),
            persona_dir: None,
            soul: "Be helpful.".to_string(),
            identity: "Name: Y".to_string(),
            user: "Name: Alice".to_string(),
            memory: "Likes Rust.".to_string(),
            bootstrap: String::new(),
        }
    }

    #[tokio::test]
    async fn provides_all_non_empty_sections() {
        let provider = BotContextProvider::new(test_persona());
        let mut ctx = AssembledContext::default();

        provider.provide(&mut ctx).await.unwrap();

        // base_prompt + soul + identity + user + memory + messaging_style + platform_constraints
        // = 7 items (bootstrap is empty so skipped).
        assert_eq!(ctx.items.len(), 7);
    }

    #[tokio::test]
    async fn skips_empty_sections() {
        let persona = BotPersona {
            config: super::super::config::BotConfig::default(),
            persona_dir: None,
            soul: String::new(),
            identity: String::new(),
            user: String::new(),
            memory: String::new(),
            bootstrap: String::new(),
        };
        let provider = BotContextProvider::new(persona);
        let mut ctx = AssembledContext::default();

        provider.provide(&mut ctx).await.unwrap();

        // Only base_prompt + messaging_style + platform_constraints = 3.
        assert_eq!(ctx.items.len(), 3);
    }

    #[tokio::test]
    async fn includes_bootstrap_when_present() {
        let mut persona = test_persona();
        persona.bootstrap = "First-run ritual.".to_string();
        let provider = BotContextProvider::new(persona);
        let mut ctx = AssembledContext::default();

        provider.provide(&mut ctx).await.unwrap();

        assert_eq!(ctx.items.len(), 8);
        let bootstrap_item = ctx.items.iter().find(|i| i.priority == 40).unwrap();
        assert_eq!(bootstrap_item.content, "First-run ritual.");
    }

    #[test]
    fn priority_is_50() {
        let provider = BotContextProvider::new(test_persona());
        assert_eq!(provider.priority(), 50);
    }

    #[test]
    fn name_is_correct() {
        let provider = BotContextProvider::new(test_persona());
        assert_eq!(provider.name(), "BotContextProvider");
    }
}
