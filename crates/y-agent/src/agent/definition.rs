//! Agent definition: TOML-parsed descriptors for agent capabilities.
//!
//! Design reference: multi-agent-design.md §Agent Definition, §Agent Behavioral Modes

use serde::{Deserialize, Serialize};

use crate::agent::error::MultiAgentError;
use crate::agent::trust::TrustTier;

/// Behavioral mode governing tool availability and system prompt focus.
///
/// Design reference: multi-agent-design.md §Agent Behavioral Modes
///
/// Modes are configuration overlays: when an agent runs in `Plan` mode,
/// the mode configuration filters the agent's tool list to read-only tools
/// and prepends a mode-specific instruction to the system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Implementation-focused execution — all allowed tools available.
    Build,
    /// Read-only analysis and planning — only read-only tools.
    Plan,
    /// Fast information gathering — search and read tools.
    Explore,
    /// Balanced conversation and task execution — all allowed tools, default prompt.
    General,
}

/// Context sharing strategy for delegations.
///
/// Design reference: multi-agent-design.md §Context Sharing Strategies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategy {
    /// Only the delegation prompt is provided. Minimal tokens. Full isolation.
    #[default]
    None,
    /// LLM-generated summary of relevant conversation context.
    Summary,
    /// Specific messages matching a filter (by role, recency, keyword).
    Filtered,
    /// Complete conversation history up to token limit.
    Full,
}

/// Complete definition of an agent, parsed from TOML.
///
/// Design reference: multi-agent-design.md §Agent Definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Default behavioral mode. The delegator can override per delegation.
    pub mode: AgentMode,
    pub trust_tier: TrustTier,

    // -- Capabilities & tools --
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Tools the agent is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied (even if otherwise available at workspace level).
    #[serde(default)]
    pub denied_tools: Vec<String>,
    pub system_prompt: String,

    // -- Skills --
    /// Activated skills from the Skill Registry.
    #[serde(default)]
    pub skills: Vec<String>,

    // -- Model preferences --
    /// Preferred model identifiers (tried in order).
    #[serde(default)]
    pub preferred_models: Vec<String>,
    /// Fallback models when preferred are unavailable.
    #[serde(default)]
    pub fallback_models: Vec<String>,
    /// Provider routing tags (e.g. `["general"]`, `["title"]`).
    /// Used as `required_tags` in `RouteRequest` for provider selection.
    #[serde(default)]
    pub provider_tags: Vec<String>,
    /// Temperature setting for LLM calls.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Top-p (nucleus sampling) setting for LLM calls.
    #[serde(default)]
    pub top_p: Option<f64>,

    // -- Limits --
    /// Maximum agent loop iterations before forced termination.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Maximum tool calls per agent run.
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    /// Timeout in seconds for the entire agent run.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,

    // -- Context --
    /// Default context sharing strategy when this agent is delegated to.
    #[serde(default)]
    pub context_sharing: ContextStrategy,
    /// Maximum tokens for context shared with this agent.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

const fn default_max_iterations() -> usize {
    20
}
const fn default_max_tool_calls() -> usize {
    50
}
const fn default_timeout_secs() -> u64 {
    300
}
const fn default_max_context_tokens() -> usize {
    4096
}

impl AgentDefinition {
    /// Parse an agent definition from TOML.
    pub fn from_toml(toml_str: &str) -> Result<Self, MultiAgentError> {
        toml::from_str(toml_str).map_err(|e| MultiAgentError::InvalidDefinition {
            message: e.to_string(),
        })
    }

    /// Validate the definition has required fields.
    pub fn validate(&self) -> Result<(), MultiAgentError> {
        if self.id.is_empty() {
            return Err(MultiAgentError::InvalidDefinition {
                message: "agent id is required".to_string(),
            });
        }
        if self.name.is_empty() {
            return Err(MultiAgentError::InvalidDefinition {
                message: "agent name is required".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_toml() -> String {
        r#"
id = "code-reviewer"
name = "Code Reviewer"
description = "Reviews code for bugs and style"
mode = "plan"
trust_tier = "user_defined"
capabilities = ["code_review", "static_analysis"]
allowed_tools = ["file_read", "search_code"]
denied_tools = ["shell_exec"]
system_prompt = "You are a code reviewer."
skills = ["code-analysis"]
preferred_models = ["gpt-4o"]
fallback_models = ["gpt-4o-mini"]
temperature = 0.3
max_iterations = 10
context_sharing = "summary"
max_context_tokens = 8192
"#
        .to_string()
    }

    /// T-MA-001-01: Valid TOML agent definition parses.
    #[test]
    fn test_definition_parse_valid() {
        let def = AgentDefinition::from_toml(&valid_toml()).unwrap();
        assert_eq!(def.id, "code-reviewer");
        assert_eq!(def.name, "Code Reviewer");
        assert_eq!(def.mode, AgentMode::Plan);
        assert_eq!(def.trust_tier, TrustTier::UserDefined);
        assert_eq!(def.capabilities.len(), 2);
        assert_eq!(def.denied_tools, vec!["shell_exec"]);
        assert_eq!(def.skills, vec!["code-analysis"]);
        assert_eq!(def.preferred_models, vec!["gpt-4o"]);
        assert_eq!(def.temperature, Some(0.3));
        assert_eq!(def.max_iterations, 10);
        assert_eq!(def.context_sharing, ContextStrategy::Summary);
        assert_eq!(def.max_context_tokens, 8192);
    }

    /// T-MA-001-02: All four agent modes parse correctly.
    #[test]
    fn test_definition_mode_enum() {
        let modes = [
            ("build", AgentMode::Build),
            ("plan", AgentMode::Plan),
            ("explore", AgentMode::Explore),
            ("general", AgentMode::General),
        ];
        for (toml_val, expected) in modes {
            let toml_str = format!(
                r#"
id = "test"
name = "Test"
description = "test"
mode = "{toml_val}"
trust_tier = "built_in"
system_prompt = ""
"#
            );
            let def = AgentDefinition::from_toml(&toml_str).unwrap();
            assert_eq!(
                def.mode, expected,
                "mode '{toml_val}' did not parse correctly"
            );
        }
    }

    /// T-MA-001-03: Invalid TOML returns error.
    #[test]
    fn test_definition_invalid_toml() {
        let result = AgentDefinition::from_toml("not valid toml {{{}");
        assert!(result.is_err());
    }

    /// T-MA-001-04: Validation catches empty ID.
    #[test]
    fn test_definition_validation() {
        let def = AgentDefinition {
            id: String::new(),
            name: "Test".to_string(),
            description: String::new(),
            mode: AgentMode::General,
            trust_tier: TrustTier::Dynamic,
            capabilities: vec![],
            allowed_tools: vec![],
            denied_tools: vec![],
            system_prompt: String::new(),
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
        };
        assert!(def.validate().is_err());
    }

    /// T-MA-001-05: Default values for optional fields.
    #[test]
    fn test_definition_defaults() {
        let toml_str = r#"
id = "minimal"
name = "Minimal Agent"
description = "Minimal definition"
mode = "general"
trust_tier = "dynamic"
system_prompt = "Hello"
"#;
        let def = AgentDefinition::from_toml(toml_str).unwrap();
        assert!(def.allowed_tools.is_empty());
        assert!(def.denied_tools.is_empty());
        assert!(def.skills.is_empty());
        assert!(def.preferred_models.is_empty());
        assert_eq!(def.temperature, None);
        assert_eq!(def.max_iterations, 20);
        assert_eq!(def.max_tool_calls, 50);
        assert_eq!(def.timeout_secs, 300);
        assert_eq!(def.context_sharing, ContextStrategy::None);
        assert_eq!(def.max_context_tokens, 4096);
    }

    /// T-MA-001-06: Context strategies parse correctly.
    #[test]
    fn test_context_strategy_parse() {
        let strategies = [
            ("none", ContextStrategy::None),
            ("summary", ContextStrategy::Summary),
            ("filtered", ContextStrategy::Filtered),
            ("full", ContextStrategy::Full),
        ];
        for (val, expected) in strategies {
            let toml_str = format!(
                r#"
id = "t"
name = "T"
description = "t"
mode = "general"
trust_tier = "dynamic"
system_prompt = ""
context_sharing = "{val}"
"#
            );
            let def = AgentDefinition::from_toml(&toml_str).unwrap();
            assert_eq!(def.context_sharing, expected);
        }
    }
}
