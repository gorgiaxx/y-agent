//! Built-in prompt sections and default template.
//!
//! Provides factory functions for the 9 built-in prompt sections defined in
//! `prompt-design.md` and a default `PromptTemplate` referencing them.

use std::collections::HashMap;

use crate::section::{ContentSource, PromptSection, SectionCategory, SectionCondition};
use crate::store::SectionStore;
use crate::template::{ModeOverlay, PromptTemplate, SectionRef};

/// Create a `SectionStore` populated with the 9 built-in prompt sections.
///
/// Sections use `ContentSource::Inline` with default content.
/// Dynamic sections (`core.datetime`, `core.environment`) use placeholder
/// content that the `BuildSystemPromptProvider` replaces at assembly time.
pub fn builtin_section_store() -> SectionStore {
    let mut store = SectionStore::new();

    store.register(PromptSection {
        id: "core.identity".into(),
        content_source: ContentSource::Inline(
            "You are y-agent, an AI assistant built for software engineering tasks. \
             You are direct, precise, and helpful. You write clean, correct code and \
             explain your reasoning clearly."
                .into(),
        ),
        token_budget: 200,
        priority: 100,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Identity,
    });

    store.register(PromptSection {
        id: "core.datetime".into(),
        content_source: ContentSource::Inline("{{datetime}}".into()),
        token_budget: 50,
        priority: 150,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Context,
    });

    store.register(PromptSection {
        id: "core.environment".into(),
        content_source: ContentSource::Inline("{{environment}}".into()),
        token_budget: 300,
        priority: 200,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Context,
    });

    store.register(PromptSection {
        id: "core.guidelines".into(),
        content_source: ContentSource::Inline(
            "Guidelines:\n\
             - Read and understand existing code before making changes.\n\
             - Follow existing patterns and conventions in the codebase.\n\
             - Keep changes minimal and focused on the task at hand.\n\
             - Prefer editing existing files over creating new ones.\n\
             - Write clear, descriptive commit messages.\n\
             - Do not introduce security vulnerabilities."
                .into(),
        ),
        token_budget: 500,
        priority: 300,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Behavioral,
    });

    store.register(PromptSection {
        id: "core.safety".into(),
        content_source: ContentSource::Inline(
            "Safety rules:\n\
             - Never execute destructive commands without explicit user confirmation.\n\
             - Do not access, modify, or transmit sensitive data (API keys, credentials, personal information).\n\
             - Refuse requests that could cause harm to systems or data.\n\
             - When uncertain, ask the user for clarification before proceeding."
                .into(),
        ),
        token_budget: 300,
        priority: 400,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Behavioral,
    });

    store.register(PromptSection {
        id: "core.tool_protocol".into(),
        content_source: ContentSource::Inline(
            "## Tool Usage Protocol\n\
             \n\
             Use tools ONLY when the user's request requires an action you cannot \
             accomplish with plain text (e.g., reading files, running commands, searching \
             code, modifying files). For greetings, general conversation, questions you \
             can answer from context, or simple explanations, respond directly in text \
             without calling any tool.\n\
             \n\
             When you do need a tool, output a <tool_call> block with <name> and <arguments> tags:\n\
             \n\
             <tool_call>\n\
             <name>tool_name</name>\n\
             <arguments>{\"param1\": \"value1\"}</arguments>\n\
             </tool_call>\n\
             \n\
             You may include multiple <tool_call> blocks in a single response. \
             Each will be executed in order.\n\
             \n\
             After each tool call, you will receive the result in a <tool_result> block:\n\
             \n\
             <tool_result name=\"tool_name\" success=\"true\">\n\
             {\"result_key\": \"result_value\"}\n\
             </tool_result>\n\
             \n\
             ## Core Tools (always available)\n\
             \n\
             You can call these tools directly without searching:\n\
             \n\
             | Tool | Description | Required Args |\n\
             |------|-------------|---------------|\n\
             | file_read | Read file contents | {\"path\": \"<filepath>\"} |\n\
             | file_write | Write content to a file (creates dirs) | {\"path\": \"<filepath>\", \"content\": \"<text>\"} |\n\
             | file_list | List directory contents | {\"path\": \"<dirpath>\"} |\n\
             | file_search | Search for text pattern in files | {\"pattern\": \"<text>\", \"path\": \"<dirpath>\"} |\n\
             | shell_exec | Execute a shell command | {\"command\": \"<cmd>\"} |\n\
             \n\
             IMPORTANT: Use ONLY these exact tool names. Do NOT invent tool names \
             like 'ls', 'cat', 'grep', or 'mkdir'. For shell operations not covered above, \
             use shell_exec.\n\
             \n\
             ## Extended Tools\n\
             \n\
             For capabilities beyond the core tools, use tool_search to discover additional tools:\n\
             - Do not guess tool names for extended tools -- search first, then call.\n\
             - You may include regular text before and after tool calls."
                .into(),
        ),
        token_budget: 600,
        priority: 450,
        condition: Some(SectionCondition::Always),
        category: SectionCategory::Behavioral,
    });

    store.register(PromptSection {
        id: "core.tool_behavior".into(),
        content_source: ContentSource::Inline(
            "Tool usage:\n\
             - Use the appropriate tool for each task.\n\
             - Validate tool parameters before invocation.\n\
             - Handle tool errors gracefully and report failures clearly.\n\
             - Prefer read-only operations when gathering information."
                .into(),
        ),
        token_budget: 300,
        priority: 500,
        condition: Some(SectionCondition::HasTool("*".into())),
        category: SectionCategory::Behavioral,
    });

    store.register(PromptSection {
        id: "core.persona".into(),
        content_source: ContentSource::Inline(String::new()),
        token_budget: 500,
        priority: 250,
        condition: Some(SectionCondition::ConfigFlag("persona.enabled".into())),
        category: SectionCategory::Domain,
    });

    store.register(PromptSection {
        id: "core.planning".into(),
        content_source: ContentSource::Inline(
            "You are in planning mode. Focus on:\n\
             - Analyzing requirements and constraints before proposing solutions.\n\
             - Breaking down complex tasks into clear, ordered steps.\n\
             - Identifying risks, dependencies, and alternatives.\n\
             - Presenting a structured plan for user approval before implementation."
                .into(),
        ),
        token_budget: 300,
        priority: 350,
        condition: Some(SectionCondition::ModeIs("plan".into())),
        category: SectionCategory::Behavioral,
    });

    store.register(PromptSection {
        id: "core.exploration".into(),
        content_source: ContentSource::Inline(
            "You are in exploration mode. Focus on:\n\
             - Searching broadly across the codebase to understand structure.\n\
             - Reading files and tracing code paths to answer questions.\n\
             - Summarizing findings concisely.\n\
             - Do not make changes; only observe and report."
                .into(),
        ),
        token_budget: 200,
        priority: 350,
        condition: Some(SectionCondition::ModeIs("explore".into())),
        category: SectionCategory::Behavioral,
    });

    store
}

/// Create the default `PromptTemplate` referencing the built-in sections.
///
/// Includes mode overlays for `plan` and `explore` modes as defined in
/// `prompt-design.md`.
pub fn default_template() -> PromptTemplate {
    let sections = vec![
        section_ref("core.identity"),
        section_ref("core.datetime"),
        section_ref("core.environment"),
        section_ref("core.persona"),
        section_ref("core.guidelines"),
        section_ref("core.safety"),
        section_ref("core.tool_protocol"),
        section_ref("core.tool_behavior"),
        section_ref("core.planning"),
        section_ref("core.exploration"),
    ];

    let mut mode_overlays = HashMap::new();

    mode_overlays.insert(
        "plan".into(),
        ModeOverlay {
            exclude: vec!["core.tool_behavior".into()],
            include: vec!["core.planning".into()],
            ..Default::default()
        },
    );

    mode_overlays.insert(
        "explore".into(),
        ModeOverlay {
            exclude: vec!["core.safety".into()],
            include: vec!["core.exploration".into()],
            token_budget_override: Some(2000),
            ..Default::default()
        },
    );

    PromptTemplate {
        id: "default".into(),
        parent: None,
        sections,
        mode_overlays,
        total_token_budget: 4000,
    }
}

fn section_ref(id: &str) -> SectionRef {
    SectionRef {
        section_id: id.into(),
        priority_override: None,
        condition_override: None,
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_store_has_10_sections() {
        let store = builtin_section_store();
        assert_eq!(store.len(), 10);
    }

    #[test]
    fn test_builtin_sections_have_inline_content() {
        let store = builtin_section_store();
        let ids = [
            "core.identity",
            "core.datetime",
            "core.environment",
            "core.guidelines",
            "core.safety",
            "core.tool_protocol",
            "core.tool_behavior",
            "core.persona",
            "core.planning",
            "core.exploration",
        ];
        for id in &ids {
            let content = store.load_content(id);
            assert!(content.is_ok(), "section {id} should have loadable content");
        }
    }

    #[test]
    fn test_default_template_general_mode() {
        let template = default_template();
        let sections = template.effective_sections("general");
        // general mode: all 10 sections in template, no overlay excludes any
        // (conditions are evaluated later by the provider, not by effective_sections)
        assert_eq!(sections.len(), 10);
    }

    #[test]
    fn test_default_template_plan_mode() {
        let template = default_template();
        let sections = template.effective_sections("plan");
        let ids: Vec<&str> = sections.iter().map(|s| s.section_id.as_str()).collect();
        // plan mode excludes core.tool_behavior
        assert!(!ids.contains(&"core.tool_behavior"));
        // plan mode includes core.planning (already in template, not excluded)
        assert!(ids.contains(&"core.planning"));
    }

    #[test]
    fn test_default_template_explore_mode() {
        let template = default_template();
        let sections = template.effective_sections("explore");
        let ids: Vec<&str> = sections.iter().map(|s| s.section_id.as_str()).collect();
        assert!(!ids.contains(&"core.safety"));
        assert!(ids.contains(&"core.exploration"));
        assert_eq!(template.effective_budget("explore"), 2000);
    }

    #[test]
    fn test_default_template_budget() {
        let template = default_template();
        assert_eq!(template.effective_budget("general"), 4000);
        assert_eq!(template.effective_budget("build"), 4000);
    }
}
