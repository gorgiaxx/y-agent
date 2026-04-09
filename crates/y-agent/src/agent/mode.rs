//! Mode overlay: filters tool lists and injects mode-specific system prompts.
//!
//! Design reference: multi-agent-design.md §Agent Behavioral Modes
//!
//! Modes are configuration overlays applied at delegation time:
//! - `Build`: all allowed tools available, implementation-focused prompt
//! - `Plan`: read-only tools only, analysis-focused prompt
//! - `Explore`: search + read tools only, information-gathering prompt
//! - `General`: all allowed tools, balanced prompt

use crate::agent::definition::{AgentDefinition, AgentMode};

// ---------------------------------------------------------------------------
// Read-only / search tool sets
// ---------------------------------------------------------------------------

/// Tools considered read-only (permitted in Plan and Explore modes).
const READ_ONLY_TOOLS: &[&str] = &[
    "FileRead",
    "SearchCode",
    "SearchFiles",
    "ListDirectory",
    "ViewFile",
    "Grep",
    "Tree",
    "GitLog",
    "GitDiff",
    "GitStatus",
    "GitShow",
];

/// Tools classified as search-oriented (permitted in Explore mode).
const SEARCH_TOOLS: &[&str] = &[
    "SearchCode",
    "SearchFiles",
    "Grep",
    "WebSearch",
    "SearchDocs",
];

/// Combined set of tools permitted in Explore mode (search + read).
fn explore_tools() -> Vec<&'static str> {
    let mut tools: Vec<&str> = Vec::new();
    tools.extend_from_slice(READ_ONLY_TOOLS);
    for t in SEARCH_TOOLS {
        if !tools.contains(t) {
            tools.push(t);
        }
    }
    tools
}

// ---------------------------------------------------------------------------
// Mode-specific system prompt prefixes
// ---------------------------------------------------------------------------

/// Returns the mode-specific system prompt prefix.
pub fn mode_prompt_prefix(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Build => {
            "You are in BUILD mode. Focus on implementation: write code, create files, \
             run tests, and make concrete changes. All write tools are available to you."
        }
        AgentMode::Plan => {
            "You are in PLAN mode. Focus on analysis and planning only. You have read-only \
             access — do NOT attempt to modify files or execute commands. Analyze the \
             codebase, identify issues, and produce a structured plan."
        }
        AgentMode::Explore => {
            "You are in EXPLORE mode. Focus on information gathering. Use search and read \
             tools to quickly locate relevant code, files, and documentation. Summarize \
             your findings concisely."
        }
        AgentMode::General => {
            "You are in GENERAL mode. Balance conversation and task execution. All tools \
             are available. Respond naturally and take action when appropriate."
        }
    }
}

// ---------------------------------------------------------------------------
// Filtered definition
// ---------------------------------------------------------------------------

/// A definition with mode overlay applied: filtered tools and augmented prompt.
#[derive(Debug, Clone)]
pub struct FilteredDefinition {
    /// The effective mode (may differ from the definition's default).
    pub mode: AgentMode,
    /// Filtered list of allowed tools based on mode.
    pub allowed_tools: Vec<String>,
    /// Denied tools (unchanged from definition).
    pub denied_tools: Vec<String>,
    /// System prompt with mode prefix prepended.
    pub system_prompt: String,
}

/// Apply a mode overlay to an agent definition.
///
/// If `mode_override` is `Some`, that mode is used instead of the definition's
/// default mode. The overlay filters the tool list and prepends a mode-specific
/// instruction to the system prompt.
pub fn apply_mode_overlay(
    definition: &AgentDefinition,
    mode_override: Option<AgentMode>,
) -> FilteredDefinition {
    let effective_mode = mode_override.unwrap_or(definition.mode);

    let allowed_tools = match effective_mode {
        AgentMode::Build | AgentMode::General => {
            // All declared tools available
            definition.allowed_tools.clone()
        }
        AgentMode::Plan => {
            // Only read-only tools
            definition
                .allowed_tools
                .iter()
                .filter(|t| READ_ONLY_TOOLS.contains(&t.as_str()))
                .cloned()
                .collect()
        }
        AgentMode::Explore => {
            // Search + read tools
            let explore = explore_tools();
            definition
                .allowed_tools
                .iter()
                .filter(|t| explore.contains(&t.as_str()))
                .cloned()
                .collect()
        }
    };

    let prefix = mode_prompt_prefix(effective_mode);
    let system_prompt = if definition.system_prompt.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}\n\n{}", definition.system_prompt)
    };

    FilteredDefinition {
        mode: effective_mode,
        allowed_tools,
        denied_tools: definition.denied_tools.clone(),
        system_prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::ContextStrategy;
    use crate::agent::trust::TrustTier;

    fn full_definition() -> AgentDefinition {
        AgentDefinition {
            id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            description: "A test agent with many tools".to_string(),
            mode: AgentMode::General,
            trust_tier: TrustTier::UserDefined,
            capabilities: vec![],
            allowed_tools: vec![
                "FileRead".to_string(),
                "FileWrite".to_string(),
                "SearchCode".to_string(),
                "ShellExec".to_string(),
                "Grep".to_string(),
                "WebSearch".to_string(),
            ],
            denied_tools: vec!["dangerous_tool".to_string()],
            system_prompt: "You are a helpful agent.".to_string(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 4096,
            max_completion_tokens: None,
            user_callable: false,
            prune_tool_history: false,
            auto_update: true,
        }
    }

    /// T-MA-R3-01: Plan mode retains only read-only tools.
    #[test]
    fn test_plan_mode_read_only_tools() {
        let def = full_definition();
        let filtered = apply_mode_overlay(&def, Some(AgentMode::Plan));

        assert_eq!(filtered.mode, AgentMode::Plan);
        // FileRead, SearchCode, Grep are read-only; FileWrite, ShellExec, WebSearch are not
        assert!(filtered.allowed_tools.contains(&"FileRead".to_string()));
        assert!(filtered.allowed_tools.contains(&"SearchCode".to_string()));
        assert!(filtered.allowed_tools.contains(&"Grep".to_string()));
        assert!(!filtered.allowed_tools.contains(&"FileWrite".to_string()));
        assert!(!filtered.allowed_tools.contains(&"ShellExec".to_string()));
    }

    /// T-MA-R3-02: Explore mode retains only search + read tools.
    #[test]
    fn test_explore_mode_search_read_tools() {
        let def = full_definition();
        let filtered = apply_mode_overlay(&def, Some(AgentMode::Explore));

        assert_eq!(filtered.mode, AgentMode::Explore);
        // FileRead, SearchCode, Grep, WebSearch are explore-allowed
        assert!(filtered.allowed_tools.contains(&"FileRead".to_string()));
        assert!(filtered.allowed_tools.contains(&"SearchCode".to_string()));
        assert!(filtered.allowed_tools.contains(&"Grep".to_string()));
        assert!(filtered.allowed_tools.contains(&"WebSearch".to_string()));
        // FileWrite and ShellExec are not explore-allowed
        assert!(!filtered.allowed_tools.contains(&"FileWrite".to_string()));
        assert!(!filtered.allowed_tools.contains(&"ShellExec".to_string()));
    }

    /// T-MA-R3-03: Mode-specific system prompt correctly injected.
    #[test]
    fn test_mode_prompt_injection() {
        let def = full_definition();

        let build = apply_mode_overlay(&def, Some(AgentMode::Build));
        assert!(build.system_prompt.contains("BUILD mode"));
        assert!(build.system_prompt.contains("You are a helpful agent."));

        let plan = apply_mode_overlay(&def, Some(AgentMode::Plan));
        assert!(plan.system_prompt.contains("PLAN mode"));
        assert!(plan.system_prompt.contains("read-only"));

        let explore = apply_mode_overlay(&def, Some(AgentMode::Explore));
        assert!(explore.system_prompt.contains("EXPLORE mode"));
        assert!(explore.system_prompt.contains("information gathering"));

        let general = apply_mode_overlay(&def, Some(AgentMode::General));
        assert!(general.system_prompt.contains("GENERAL mode"));
    }

    /// Build and General modes keep all tools.
    #[test]
    fn test_build_general_keep_all_tools() {
        let def = full_definition();

        let build = apply_mode_overlay(&def, Some(AgentMode::Build));
        assert_eq!(build.allowed_tools.len(), def.allowed_tools.len());

        let general = apply_mode_overlay(&def, Some(AgentMode::General));
        assert_eq!(general.allowed_tools.len(), def.allowed_tools.len());
    }

    /// No mode override uses the definition's default mode.
    #[test]
    fn test_no_mode_override_uses_default() {
        let mut def = full_definition();
        def.mode = AgentMode::Plan;

        let filtered = apply_mode_overlay(&def, None);
        assert_eq!(filtered.mode, AgentMode::Plan);
        // Should apply Plan filtering
        assert!(!filtered.allowed_tools.contains(&"FileWrite".to_string()));
    }

    /// Empty system prompt gets only the mode prefix.
    #[test]
    fn test_empty_prompt_gets_prefix_only() {
        let mut def = full_definition();
        def.system_prompt = String::new();

        let filtered = apply_mode_overlay(&def, Some(AgentMode::Build));
        assert!(filtered.system_prompt.contains("BUILD mode"));
        // Should not have double newlines from empty original
        assert!(!filtered.system_prompt.contains("\n\n"));
    }

    /// Denied tools are preserved unchanged.
    #[test]
    fn test_denied_tools_preserved() {
        let def = full_definition();
        let filtered = apply_mode_overlay(&def, Some(AgentMode::Plan));
        assert_eq!(filtered.denied_tools, def.denied_tools);
    }
}
