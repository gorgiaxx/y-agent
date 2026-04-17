//! Agent definition: TOML-parsed descriptors for agent capabilities.
//!
//! Design reference: multi-agent-design.md §Agent Definition, §Agent Behavioral Modes

use serde::{Deserialize, Serialize};

use crate::agent::error::MultiAgentError;
use crate::agent::trust::TrustTier;
use y_core::permission_types::PermissionMode;
use y_core::provider::{ResponseFormat, ThinkingConfig, ThinkingEffort};

/// Response format configuration as written in TOML agent definitions.
///
/// Supports structured output via JSON Schema. The schema can be provided
/// as either a TOML table (`schema`) or a JSON string (`schema_json`).
///
/// # TOML examples
///
/// Simple schema via TOML table:
/// ```toml
/// [response_format]
/// type = "json_schema"
/// name = "metadata"
///
/// [response_format.schema]
/// type = "object"
/// required = ["title", "tags"]
/// additionalProperties = false
///
/// [response_format.schema.properties.title]
/// type = "string"
///
/// [response_format.schema.properties.tags]
/// type = "array"
/// ```
///
/// Complex schema via JSON string:
/// ```toml
/// [response_format]
/// type = "json_schema"
/// name = "metadata"
/// schema_json = '''
/// {
///   "type": "object",
///   "properties": { "title": { "type": "string" } },
///   "required": ["title"],
///   "additionalProperties": false
/// }
/// '''
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormatConfig {
    /// Format type: `"text"`, `"json_object"`, or `"json_schema"`.
    #[serde(rename = "type")]
    pub format_type: String,
    /// Schema name (required for `json_schema` type).
    #[serde(default)]
    pub name: Option<String>,
    /// Schema as a TOML table (deserialized to JSON value).
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
    /// Schema as a JSON string (alternative for complex schemas).
    /// Takes precedence over `schema` if both are provided.
    #[serde(default)]
    pub schema_json: Option<String>,
}

impl ResponseFormatConfig {
    /// Convert to the core `ResponseFormat` type.
    ///
    /// Returns `Err` if the configuration is invalid (e.g., `json_schema`
    /// without a name or schema).
    pub fn to_response_format(&self) -> Result<ResponseFormat, MultiAgentError> {
        match self.format_type.as_str() {
            "text" => Ok(ResponseFormat::Text),
            "json_object" => Ok(ResponseFormat::JsonObject),
            "json_schema" => {
                let name = self
                    .name
                    .clone()
                    .ok_or_else(|| MultiAgentError::InvalidDefinition {
                        message: "response_format.name is required for json_schema type"
                            .to_string(),
                    })?;
                // Prefer schema_json over schema table.
                let schema = if let Some(ref json_str) = self.schema_json {
                    serde_json::from_str(json_str).map_err(|e| {
                        MultiAgentError::InvalidDefinition {
                            message: format!("response_format.schema_json is not valid JSON: {e}"),
                        }
                    })?
                } else if let Some(ref table) = self.schema {
                    table.clone()
                } else {
                    return Err(MultiAgentError::InvalidDefinition {
                        message: "response_format requires either schema or schema_json \
                                  for json_schema type"
                            .to_string(),
                    });
                };
                Ok(ResponseFormat::JsonSchema { name, schema })
            }
            other => Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "unknown response_format type '{other}': \
                     expected text, json_object, or json_schema"
                ),
            }),
        }
    }
}

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
    /// Optional emoji or short icon used by presentation layers.
    #[serde(default)]
    pub icon: Option<String>,
    /// Preferred working directory for user-facing agent sessions.
    #[serde(default)]
    pub working_directory: Option<String>,
    /// Whether tool calling is enabled for user-facing sessions of this agent.
    ///
    /// `None` preserves legacy behavior for older definitions.
    #[serde(default)]
    pub toolcall_enabled: Option<bool>,
    /// Whether skill injection is enabled for user-facing sessions of this agent.
    ///
    /// `None` preserves legacy behavior for older definitions.
    #[serde(default)]
    pub skills_enabled: Option<bool>,
    /// Whether knowledge injection is enabled for user-facing sessions of this agent.
    ///
    /// `None` preserves legacy behavior for older definitions.
    #[serde(default)]
    pub knowledge_enabled: Option<bool>,
    /// Tools the agent is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub system_prompt: String,

    // -- Skills --
    /// Activated skills from the Skill Registry.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Knowledge collections injected into the context pipeline for this agent.
    #[serde(default)]
    pub knowledge_collections: Vec<String>,
    /// Explicit built-in prompt sections to include for user-facing sessions.
    ///
    /// Empty means use the default template-driven section selection.
    #[serde(default)]
    pub prompt_section_ids: Vec<String>,

    // -- Model preferences --
    /// Preferred provider identifier for user-facing chat sessions.
    #[serde(default)]
    pub provider_id: Option<String>,
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
    /// Default plan-mode preference for GUI sessions (`fast`, `auto`, `plan`).
    #[serde(default)]
    pub plan_mode: Option<String>,
    /// Default thinking-effort preference for GUI sessions.
    #[serde(default)]
    pub thinking_effort: Option<String>,
    /// Default permission mode for this agent's sessions.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,

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
    /// Maximum tokens available for the agent's combined input context window.
    ///
    /// Used by context management (`IntraTurnPruner`, context injection) to decide
    /// how much history to retain. This is a budget for the *input* side and does
    /// NOT map directly to the provider's `max_tokens` API parameter.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Maximum tokens the provider may generate in a single completion call.
    ///
    /// Maps directly to the provider's `max_tokens` (or `max_completion_tokens`)
    /// API parameter. When `None` (the default), the provider's own default is
    /// used. Set this only when the agent's output is bounded and you need to
    /// avoid exhausting the model's total context window.
    #[serde(default)]
    pub max_completion_tokens: Option<usize>,

    // -- Visibility --
    /// Whether this agent can be directly invoked by users for task delegation.
    /// `false` (default) = internal system agent; `true` = user-callable.
    #[serde(default)]
    pub user_callable: bool,

    // -- MCP --
    /// Default MCP mode for this agent's sessions.
    ///
    /// One of `"auto"`, `"manual"`, or `"disabled"`. `None` = `"auto"`.
    /// - `"auto"`: all enabled MCP server tools are available.
    /// - `"manual"`: only tools from servers listed in `mcp_servers` are available.
    /// - `"disabled"`: no MCP tools are exposed.
    #[serde(default)]
    pub mcp_mode: Option<String>,
    /// MCP server names whose tools are exposed when `mcp_mode = "manual"`.
    #[serde(default)]
    pub mcp_servers: Vec<String>,

    // -- Working history pruning --
    /// Whether to prune historical tool call pairs from `working_history`.
    ///
    /// When `true`, after each iteration the agent loop removes all but the
    /// most recent batch of assistant+tool message pairs from `working_history`.
    /// Both the assistant message (with its `tool_calls`) and the corresponding
    /// tool result messages are removed together -- no orphaned references.
    ///
    /// Default: `false` -- all tool call history is preserved.
    #[serde(default)]
    pub prune_tool_history: bool,

    // -- Auto-update --
    /// Whether this agent's seed file should be auto-updated on startup when
    /// a newer builtin version is available.
    ///
    /// Default: `true`. Set to `false` in the user's agents directory to
    /// preserve local customizations across y-agent upgrades.
    #[serde(default = "default_auto_update")]
    pub auto_update: bool,

    /// Response format for structured output.
    ///
    /// When set, the provider enforces the response conforms to the
    /// specified format (e.g., a JSON Schema). See [`ResponseFormatConfig`]
    /// for TOML syntax.
    #[serde(default)]
    pub response_format: Option<ResponseFormatConfig>,
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
const fn default_auto_update() -> bool {
    true
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

    #[must_use]
    pub fn toolcall_enabled_resolved(&self) -> bool {
        self.toolcall_enabled
            .unwrap_or(!self.allowed_tools.is_empty())
    }

    #[must_use]
    pub fn skills_enabled_resolved(&self) -> bool {
        self.skills_enabled.unwrap_or(!self.skills.is_empty())
    }

    #[must_use]
    pub fn knowledge_enabled_resolved(&self) -> bool {
        self.knowledge_enabled
            .unwrap_or(!self.knowledge_collections.is_empty())
    }

    #[must_use]
    pub fn thinking_config(&self) -> Option<ThinkingConfig> {
        let effort = match self.thinking_effort.as_deref()? {
            "low" => ThinkingEffort::Low,
            "medium" => ThinkingEffort::Medium,
            "high" => ThinkingEffort::High,
            "max" => ThinkingEffort::Max,
            _ => return None,
        };
        Some(ThinkingConfig { effort })
    }

    /// Resolve the response format configuration to a core `ResponseFormat`.
    ///
    /// Returns `None` if no response format is configured, or `Err` if the
    /// configuration is invalid.
    pub fn resolved_response_format(&self) -> Result<Option<ResponseFormat>, MultiAgentError> {
        self.response_format
            .as_ref()
            .map(ResponseFormatConfig::to_response_format)
            .transpose()
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
allowed_tools = ["FileRead", "SearchCode"]
system_prompt = "You are a code reviewer."
skills = ["code-analysis"]
preferred_models = ["gpt-4o"]
fallback_models = ["gpt-4o-mini"]
temperature = 0.3
max_iterations = 20
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
        assert_eq!(def.skills, vec!["code-analysis"]);
        assert_eq!(def.preferred_models, vec!["gpt-4o"]);
        assert_eq!(def.temperature, Some(0.3));
        assert_eq!(def.max_iterations, 20);
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
            icon: None,
            working_directory: None,
            toolcall_enabled: None,
            skills_enabled: None,
            knowledge_enabled: None,
            allowed_tools: vec![],
            system_prompt: String::new(),
            skills: vec![],
            knowledge_collections: vec![],
            prompt_section_ids: vec![],
            provider_id: None,
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            plan_mode: None,
            thinking_effort: None,
            permission_mode: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 4096,
            max_completion_tokens: None,
            user_callable: false,
            mcp_mode: None,
            mcp_servers: vec![],
            prune_tool_history: false,
            auto_update: true,
            response_format: None,
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
        assert!(def.skills.is_empty());
        assert!(def.knowledge_collections.is_empty());
        assert!(def.prompt_section_ids.is_empty());
        assert_eq!(def.icon, None);
        assert_eq!(def.working_directory, None);
        assert_eq!(def.toolcall_enabled, None);
        assert_eq!(def.skills_enabled, None);
        assert_eq!(def.knowledge_enabled, None);
        assert_eq!(def.provider_id, None);
        assert!(def.preferred_models.is_empty());
        assert_eq!(def.temperature, None);
        assert_eq!(def.plan_mode, None);
        assert_eq!(def.thinking_effort, None);
        assert_eq!(def.permission_mode, None);
        assert_eq!(def.max_iterations, 20);
        assert_eq!(def.max_tool_calls, 50);
        assert_eq!(def.timeout_secs, 300);
        assert_eq!(def.context_sharing, ContextStrategy::None);
        assert_eq!(def.max_context_tokens, 4096);
        assert_eq!(def.max_completion_tokens, None);
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

    #[test]
    fn test_definition_parse_user_facing_profile_fields() {
        let toml = r#"
id = "workspace-coder"
name = "Workspace Coder"
description = "General coding assistant for a repository"
mode = "build"
trust_tier = "user_defined"
user_callable = true
icon = "tool"
working_directory = "/tmp/project"
toolcall_enabled = true
skills_enabled = true
knowledge_enabled = true
skills = ["rust-review", "repo-map"]
knowledge_collections = ["engineering", "product"]
allowed_tools = ["FileRead", "FileWrite", "ShellExec"]
system_prompt = "You are a coding assistant."
prompt_section_ids = ["core.identity", "core.guidelines", "core.tool_protocol"]
provider_id = "local-main"
preferred_models = ["gpt-4o"]
fallback_models = ["gpt-4o-mini"]
provider_tags = ["coding"]
plan_mode = "plan"
thinking_effort = "high"
permission_mode = "accept_edits"
temperature = 0.2
top_p = 0.9
max_iterations = 12
max_tool_calls = 24
timeout_secs = 180
context_sharing = "summary"
max_context_tokens = 8192
max_completion_tokens = 2048
"#;

        let def = AgentDefinition::from_toml(toml).unwrap();

        assert_eq!(def.icon.as_deref(), Some("tool"));
        assert_eq!(def.working_directory.as_deref(), Some("/tmp/project"));
        assert_eq!(def.toolcall_enabled, Some(true));
        assert_eq!(def.skills_enabled, Some(true));
        assert_eq!(def.knowledge_enabled, Some(true));
        assert_eq!(
            def.knowledge_collections,
            vec!["engineering".to_string(), "product".to_string()]
        );
        assert_eq!(
            def.prompt_section_ids,
            vec![
                "core.identity".to_string(),
                "core.guidelines".to_string(),
                "core.tool_protocol".to_string()
            ]
        );
        assert_eq!(def.provider_id.as_deref(), Some("local-main"));
        assert_eq!(def.plan_mode.as_deref(), Some("plan"));
        assert_eq!(def.thinking_effort.as_deref(), Some("high"));
        assert_eq!(
            def.permission_mode,
            Some(y_core::permission_types::PermissionMode::AcceptEdits)
        );
    }
}
