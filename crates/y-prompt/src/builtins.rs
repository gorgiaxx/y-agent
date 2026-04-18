//! Built-in prompt sections and default template.
//!
//! Provides factory functions for the built-in prompt sections defined in
//! `prompt-design.md` and a default `PromptTemplate` referencing them.
//!
//! Prompt content is loaded from `config/prompts/*.txt` at compile time via
//! `include_str!`. At runtime, users can override any prompt by placing a
//! `.txt` file with the same name in their XDG config prompts directory
//! (`~/.config/y-agent/prompts/`).

use std::collections::HashMap;
use std::path::Path;

use y_core::runtime::RuntimeBackend;

use crate::section::{ContentSource, PromptSection, SectionCategory, SectionCondition};
use crate::store::SectionStore;
use crate::template::{ModeOverlay, PromptTemplate, SectionRef};

// ---------------------------------------------------------------------------
// Compile-time prompt content (embedded from config/prompts/*.txt)
// ---------------------------------------------------------------------------

const PROMPT_IDENTITY: &str = include_str!("../../../config/prompts/core_identity.txt");
const PROMPT_DATETIME: &str = include_str!("../../../config/prompts/core_datetime.txt");
const PROMPT_ENVIRONMENT: &str = include_str!("../../../config/prompts/core_environment.txt");
const PROMPT_GUIDELINES: &str = include_str!("../../../config/prompts/core_guidelines.txt");
const PROMPT_SECURITY: &str = include_str!("../../../config/prompts/core_security.txt");
pub const PROMPT_TOOL_PROTOCOL: &str =
    include_str!("../../../config/prompts/core_tool_protocol.txt");
pub const PROMPT_TOOL_PROTOCOL_REMOTE: &str =
    include_str!("../../../config/prompts/core_tool_protocol_remote.txt");

const PROMPT_PERSONA: &str = include_str!("../../../config/prompts/core_persona.txt");
const PROMPT_PLANNING: &str = include_str!("../../../config/prompts/core_planning.txt");
const PROMPT_EXPLORATION: &str = include_str!("../../../config/prompts/core_exploration.txt");
const PROMPT_ORCHESTRATION: &str = include_str!("../../../config/prompts/core_orchestration.txt");
const PROMPT_PLAN_MODE_ACTIVE: &str = include_str!("../../../config/prompts/plan_mode_hint.txt");
const PROMPT_MCP_HINT: &str = include_str!("../../../config/prompts/mcp_hint.txt");

/// Mapping from section ID to (compiled default content, override filename,
/// `token_budget`, priority, condition, category).
const BUILTIN_SECTIONS: &[(&str, &str, &str, u32, i32, SectionCategoryTag, ConditionTag)] = &[
    (
        "core.plan_mode_active",
        PROMPT_PLAN_MODE_ACTIVE,
        "plan_mode_hint.txt",
        200,
        50,
        SectionCategoryTag::Behavioral,
        ConditionTag::PlanModeActive,
    ),
    (
        "core.identity",
        PROMPT_IDENTITY,
        "core_identity.txt",
        300,
        100,
        SectionCategoryTag::Identity,
        ConditionTag::Always,
    ),
    (
        "core.datetime",
        PROMPT_DATETIME,
        "core_datetime.txt",
        50,
        150,
        SectionCategoryTag::Context,
        ConditionTag::Always,
    ),
    (
        "core.environment",
        PROMPT_ENVIRONMENT,
        "core_environment.txt",
        300,
        200,
        SectionCategoryTag::Context,
        ConditionTag::Always,
    ),
    (
        "core.guidelines",
        PROMPT_GUIDELINES,
        "core_guidelines.txt",
        500,
        300,
        SectionCategoryTag::Behavioral,
        ConditionTag::Always,
    ),
    (
        "core.security",
        PROMPT_SECURITY,
        "core_security.txt",
        300,
        400,
        SectionCategoryTag::Behavioral,
        ConditionTag::Always,
    ),
    (
        "core.tool_protocol",
        PROMPT_TOOL_PROTOCOL,
        "core_tool_protocol.txt",
        400,
        450,
        SectionCategoryTag::Behavioral,
        ConditionTag::Always,
    ),
    (
        "core.persona",
        PROMPT_PERSONA,
        "core_persona.txt",
        500,
        250,
        SectionCategoryTag::Domain,
        ConditionTag::PersonaEnabled,
    ),
    (
        "core.planning",
        PROMPT_PLANNING,
        "core_planning.txt",
        300,
        350,
        SectionCategoryTag::Behavioral,
        ConditionTag::ModePlan,
    ),
    (
        "core.exploration",
        PROMPT_EXPLORATION,
        "core_exploration.txt",
        200,
        350,
        SectionCategoryTag::Behavioral,
        ConditionTag::ModeExplore,
    ),
    (
        "core.orchestration",
        PROMPT_ORCHESTRATION,
        "core_orchestration.txt",
        400,
        425,
        SectionCategoryTag::Behavioral,
        ConditionTag::OrchestrationEnabled,
    ),
    (
        "core.mcp_hint",
        PROMPT_MCP_HINT,
        "mcp_hint.txt",
        200,
        460,
        SectionCategoryTag::Behavioral,
        ConditionTag::McpEnabled,
    ),
];

/// Internal tag for compact table — maps to `SectionCategory`.
#[derive(Clone, Copy)]
enum SectionCategoryTag {
    Identity,
    Context,
    Behavioral,
    Domain,
}

impl SectionCategoryTag {
    fn into_category(self) -> SectionCategory {
        match self {
            Self::Identity => SectionCategory::Identity,
            Self::Context => SectionCategory::Context,
            Self::Behavioral => SectionCategory::Behavioral,
            Self::Domain => SectionCategory::Domain,
        }
    }
}

/// Internal tag for compact table — maps to `Option<SectionCondition>`.
#[derive(Clone, Copy)]
enum ConditionTag {
    Always,
    PersonaEnabled,
    ModePlan,
    ModeExplore,
    /// Include `core.orchestration` only when workflow/schedule tools are active.
    ///
    /// Set by `sync_dynamic_tool_defs` in `agent_service.rs` when `ToolSearch`
    /// activates workflow or schedule tools (~400 tokens saved otherwise).
    OrchestrationEnabled,
    /// Include `core.plan_mode_active` only when the agent has entered plan mode.
    PlanModeActive,
    /// Include `core.mcp_hint` only when MCP tools are available.
    McpEnabled,
}

impl ConditionTag {
    fn into_condition(self) -> SectionCondition {
        match self {
            Self::Always => SectionCondition::Always,
            Self::PersonaEnabled => SectionCondition::ConfigFlag("persona.enabled".into()),
            Self::ModePlan => SectionCondition::ModeIs("plan".into()),
            Self::ModeExplore => SectionCondition::ModeIs("explore".into()),
            Self::OrchestrationEnabled => {
                SectionCondition::ConfigFlag("orchestration.enabled".into())
            }
            Self::PlanModeActive => SectionCondition::ConfigFlag("plan_mode.active".into()),
            Self::McpEnabled => SectionCondition::ConfigFlag("mcp.enabled".into()),
        }
    }
}

/// Return the compiled tool-protocol prompt text for the given runtime backend.
///
/// - `Native` -> dedicated-file-tool guidance (`FileRead`, `FileEdit`, ...)
/// - `Docker` / `Ssh` -> ShellExec-based guidance for remote targets
pub fn tool_protocol_for(backend: &RuntimeBackend) -> &'static str {
    match backend {
        RuntimeBackend::Native => PROMPT_TOOL_PROTOCOL,
        RuntimeBackend::Docker | RuntimeBackend::Ssh => PROMPT_TOOL_PROTOCOL_REMOTE,
    }
}

/// Create a `SectionStore` populated with the built-in prompt sections.
///
/// Uses compiled-in default content and assumes a **native** runtime.
/// For override / runtime-variant support, use
/// [`builtin_section_store_with_overrides`].
pub fn builtin_section_store() -> SectionStore {
    builtin_section_store_with_overrides(None, &RuntimeBackend::Native)
}

/// Create a `SectionStore` with built-in sections, optionally loading
/// user overrides from `prompts_dir`.
///
/// The `runtime_backend` controls which variant of `core.tool_protocol` is
/// loaded:
/// - `Native`  -> `core_tool_protocol.txt`  (dedicated file tools)
/// - `Docker` / `Ssh` -> `core_tool_protocol_remote.txt` (ShellExec-based)
///
/// For each section, if `prompts_dir` is `Some` and a corresponding `.txt`
/// file exists there, the file content is used instead of the compiled default.
/// This allows users to customise prompts by editing files in their XDG config
/// directory (`~/.config/y-agent/prompts/`).
pub fn builtin_section_store_with_overrides(
    prompts_dir: Option<&Path>,
    runtime_backend: &RuntimeBackend,
) -> SectionStore {
    let mut store = SectionStore::new();

    // Determine runtime-variant defaults for core.tool_protocol.
    let (tp_content, tp_filename) = match runtime_backend {
        RuntimeBackend::Native => (PROMPT_TOOL_PROTOCOL, "core_tool_protocol.txt"),
        RuntimeBackend::Docker | RuntimeBackend::Ssh => {
            (PROMPT_TOOL_PROTOCOL_REMOTE, "core_tool_protocol_remote.txt")
        }
    };

    for &(id, default_content, filename, token_budget, priority, cat_tag, cond_tag) in
        BUILTIN_SECTIONS
    {
        // Swap default content and override filename for the tool protocol
        // section based on the active runtime backend.
        let (effective_content, effective_filename) = if id == "core.tool_protocol" {
            (tp_content, tp_filename)
        } else {
            (default_content, filename)
        };

        // Try to load override from user's prompts directory.
        let content = prompts_dir
            .map(|dir| dir.join(effective_filename))
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .unwrap_or_else(|| effective_content.to_string());

        store.register(PromptSection {
            id: id.into(),
            content_source: ContentSource::Inline(content),
            token_budget,
            priority,
            condition: Some(cond_tag.into_condition()),
            category: cat_tag.into_category(),
        });
    }

    store
}

/// List of all built-in prompt file names (for seeding into the user config dir).
pub const BUILTIN_PROMPT_FILES: &[(&str, &str)] = &[
    ("core_identity.txt", PROMPT_IDENTITY),
    ("core_datetime.txt", PROMPT_DATETIME),
    ("core_environment.txt", PROMPT_ENVIRONMENT),
    ("core_guidelines.txt", PROMPT_GUIDELINES),
    ("core_security.txt", PROMPT_SECURITY),
    ("core_tool_protocol.txt", PROMPT_TOOL_PROTOCOL),
    ("core_tool_protocol_remote.txt", PROMPT_TOOL_PROTOCOL_REMOTE),
    ("core_persona.txt", PROMPT_PERSONA),
    ("core_planning.txt", PROMPT_PLANNING),
    ("core_exploration.txt", PROMPT_EXPLORATION),
    ("core_orchestration.txt", PROMPT_ORCHESTRATION),
    ("plan_mode_hint.txt", PROMPT_PLAN_MODE_ACTIVE),
    ("mcp_hint.txt", PROMPT_MCP_HINT),
];

/// Create the default `PromptTemplate` referencing the built-in sections.
///
/// Includes mode overlays for `plan` and `explore` modes as defined in
/// `prompt-design.md`.
pub fn default_template() -> PromptTemplate {
    let sections = vec![
        section_ref("core.plan_mode_active"),
        section_ref("core.identity"),
        section_ref("core.datetime"),
        section_ref("core.environment"),
        section_ref("core.persona"),
        section_ref("core.guidelines"),
        section_ref("core.security"),
        section_ref("core.tool_protocol"),
        section_ref("core.planning"),
        section_ref("core.exploration"),
        section_ref("core.orchestration"),
        section_ref("core.mcp_hint"),
    ];

    let mut mode_overlays = HashMap::new();

    mode_overlays.insert(
        "plan".into(),
        ModeOverlay {
            exclude: vec![],
            include: vec!["core.planning".into()],
            ..Default::default()
        },
    );

    mode_overlays.insert(
        "explore".into(),
        ModeOverlay {
            exclude: vec!["core.security".into()],
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
    fn test_builtin_store_has_12_sections() {
        let store = builtin_section_store();
        assert_eq!(store.len(), 12);
    }

    #[test]
    fn test_builtin_sections_have_inline_content() {
        let store = builtin_section_store();
        let ids = [
            "core.identity",
            "core.datetime",
            "core.environment",
            "core.guidelines",
            "core.security",
            "core.tool_protocol",
            "core.persona",
            "core.planning",
            "core.exploration",
            "core.orchestration",
            "core.mcp_hint",
            "core.plan_mode_active",
        ];
        for id in &ids {
            let content = store.load_content(id);
            assert!(content.is_ok(), "section {id} should have loadable content");
        }
    }

    #[test]
    fn test_builtin_store_with_overrides_uses_defaults() {
        // No override directory — should use compiled defaults.
        let store = builtin_section_store_with_overrides(None, &RuntimeBackend::Native);
        let content = store.load_content("core.identity").unwrap();
        assert!(content.contains("y-agent"));
    }

    #[test]
    fn test_builtin_store_with_overrides_loads_file() {
        // Create a temp dir with an override file.
        let dir = std::env::temp_dir().join("y-agent-prompt-override-test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("core_identity.txt"), "Custom identity prompt").unwrap();

        let store = builtin_section_store_with_overrides(Some(&dir), &RuntimeBackend::Native);
        let content = store.load_content("core.identity").unwrap();
        assert_eq!(content, "Custom identity prompt");

        // Non-overridden section falls back to default.
        let guidelines = store.load_content("core.guidelines").unwrap();
        assert!(guidelines.contains("Guidelines"));

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tool_protocol_native_variant_loaded_for_native_runtime() {
        let store = builtin_section_store_with_overrides(None, &RuntimeBackend::Native);
        let content = store.load_content("core.tool_protocol").unwrap();
        // Native variant lists dedicated file tools.
        assert!(content.contains("FileRead"));
        assert!(content.contains("FileEdit"));
        assert!(!content.contains("Remote Runtime"));
    }

    #[test]
    fn test_tool_protocol_remote_variant_loaded_for_docker_runtime() {
        let store = builtin_section_store_with_overrides(None, &RuntimeBackend::Docker);
        let content = store.load_content("core.tool_protocol").unwrap();
        // Remote variant emphasises ShellExec for target operations.
        assert!(content.contains("Remote Runtime"));
        assert!(content.contains("ShellExec"));
    }

    #[test]
    fn test_tool_protocol_remote_variant_loaded_for_ssh_runtime() {
        let store = builtin_section_store_with_overrides(None, &RuntimeBackend::Ssh);
        let content = store.load_content("core.tool_protocol").unwrap();
        assert!(content.contains("Remote Runtime"));
    }

    #[test]
    fn test_tool_protocol_remote_override_file_takes_precedence() {
        let dir = std::env::temp_dir().join("y-agent-prompt-remote-override-test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("core_tool_protocol_remote.txt"),
            "CUSTOM REMOTE PROTOCOL",
        )
        .unwrap();

        // Remote runtime reads core_tool_protocol_remote.txt as its override.
        let remote_store = builtin_section_store_with_overrides(Some(&dir), &RuntimeBackend::Ssh);
        let remote_content = remote_store.load_content("core.tool_protocol").unwrap();
        assert_eq!(remote_content, "CUSTOM REMOTE PROTOCOL");

        // Native runtime ignores the remote override file.
        let native_store =
            builtin_section_store_with_overrides(Some(&dir), &RuntimeBackend::Native);
        let native_content = native_store.load_content("core.tool_protocol").unwrap();
        assert!(native_content.contains("FileRead"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_tool_protocol_for_helper() {
        assert_eq!(
            tool_protocol_for(&RuntimeBackend::Native),
            PROMPT_TOOL_PROTOCOL
        );
        assert_eq!(
            tool_protocol_for(&RuntimeBackend::Docker),
            PROMPT_TOOL_PROTOCOL_REMOTE
        );
        assert_eq!(
            tool_protocol_for(&RuntimeBackend::Ssh),
            PROMPT_TOOL_PROTOCOL_REMOTE
        );
    }

    #[test]
    fn test_default_template_general_mode() {
        let template = default_template();
        let sections = template.effective_sections("general");
        // general mode: all 12 sections in template, no overlay excludes any
        // (conditions are evaluated later by the provider, not by effective_sections)
        assert_eq!(sections.len(), 12);
    }

    #[test]
    fn test_default_template_plan_mode() {
        let template = default_template();
        let sections = template.effective_sections("plan");
        let ids: Vec<&str> = sections.iter().map(|s| s.section_id.as_str()).collect();
        // plan mode includes core.planning (already in template, not excluded)
        assert!(ids.contains(&"core.planning"));
    }

    #[test]
    fn test_default_template_explore_mode() {
        let template = default_template();
        let sections = template.effective_sections("explore");
        let ids: Vec<&str> = sections.iter().map(|s| s.section_id.as_str()).collect();
        assert!(!ids.contains(&"core.security"));
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
