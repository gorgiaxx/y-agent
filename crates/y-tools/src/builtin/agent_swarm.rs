//! `AgentSwarm` tool contract for bounded map-style agent delegation.
//!
//! The tool validates and materializes the full batch before `y-service`
//! dispatches any child. Actual delegation remains service-owned.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Serialize;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

pub const PROMPT_TEMPLATE_PLACEHOLDER: &str = "{{item}}";
pub const MAX_AGENT_SWARM_TASKS: usize = 32;
const KNOWN_PROPERTIES: &[&str] = &[
    "description",
    "agent_name",
    "prompt_template",
    "items",
    "mode",
    "context_strategy",
    "result_schema",
    "workspace_isolation",
];

#[derive(Debug, Clone)]
pub struct PreparedAgentSwarm {
    pub description: String,
    pub agent_name: String,
    pub mode: Option<String>,
    pub context_strategy: Option<String>,
    pub result_schema: Option<serde_json::Value>,
    pub workspace_isolation: Option<String>,
    pub tasks: Vec<PreparedAgentSwarmTask>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedAgentSwarmTask {
    pub index: usize,
    pub item: String,
    pub prompt: String,
}

pub fn prepare_agent_swarm(arguments: &serde_json::Value) -> Result<PreparedAgentSwarm, ToolError> {
    validate_properties(arguments)?;
    let description = required_non_empty_string(arguments, "description")?;
    let agent_name = required_non_empty_string(arguments, "agent_name")?;
    let prompt_template = required_non_empty_string(arguments, "prompt_template")?;
    if !prompt_template.contains(PROMPT_TEMPLATE_PLACEHOLDER) {
        return validation_error(format!(
            "'prompt_template' must contain {PROMPT_TEMPLATE_PLACEHOLDER}"
        ));
    }

    let items = arguments
        .get("items")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ToolError::ValidationError {
            message: "'items' must be an array".to_string(),
        })?;
    if items.len() < 2 {
        return validation_error("'items' must contain at least 2 entries");
    }
    if items.len() > MAX_AGENT_SWARM_TASKS {
        return validation_error(format!(
            "'items' supports at most {MAX_AGENT_SWARM_TASKS} entries"
        ));
    }

    let mut rendered_prompts = HashMap::new();
    let mut tasks = Vec::with_capacity(items.len());
    for (index, value) in items.iter().enumerate() {
        let item = value
            .as_str()
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .ok_or_else(|| ToolError::ValidationError {
                message: format!("'items[{index}]' must be a non-empty string"),
            })?
            .to_string();
        let prompt = prompt_template.replace(PROMPT_TEMPLATE_PLACEHOLDER, &item);
        if let Some(previous) = rendered_prompts.insert(prompt.clone(), index) {
            return validation_error(format!(
                "duplicate rendered prompt for items[{previous}] and items[{index}]"
            ));
        }
        tasks.push(PreparedAgentSwarmTask {
            index,
            item,
            prompt,
        });
    }

    let result_schema = arguments.get("result_schema").cloned();
    if result_schema
        .as_ref()
        .is_some_and(|schema| !schema.is_object())
    {
        return validation_error("'result_schema' must be an object");
    }

    Ok(PreparedAgentSwarm {
        description,
        agent_name,
        mode: optional_enum(arguments, "mode", &["build", "plan", "explore", "general"])?,
        context_strategy: optional_enum(
            arguments,
            "context_strategy",
            &["none", "summary", "filtered", "full"],
        )?,
        result_schema,
        workspace_isolation: optional_enum(
            arguments,
            "workspace_isolation",
            &["auto", "shared", "prefer_worktree", "require_worktree"],
        )?,
        tasks,
    })
}

fn validate_properties(arguments: &serde_json::Value) -> Result<(), ToolError> {
    let Some(object) = arguments.as_object() else {
        return validation_error("AgentSwarm arguments must be an object");
    };
    if let Some(name) = object
        .keys()
        .find(|name| !KNOWN_PROPERTIES.contains(&name.as_str()))
    {
        return validation_error(format!("unknown AgentSwarm argument '{name}'"));
    }
    Ok(())
}

fn required_non_empty_string(
    arguments: &serde_json::Value,
    name: &str,
) -> Result<String, ToolError> {
    arguments
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("'{name}' is required and must be a non-empty string"),
        })
}

fn optional_enum(
    arguments: &serde_json::Value,
    name: &str,
    allowed: &[&str],
) -> Result<Option<String>, ToolError> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|value| allowed.contains(value))
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("invalid '{name}'; expected one of: {}", allowed.join(", ")),
        })?;
    Ok(Some(value.to_string()))
}

fn validation_error<T>(message: impl Into<String>) -> Result<T, ToolError> {
    Err(ToolError::ValidationError {
        message: message.into(),
    })
}

pub struct AgentSwarmTool {
    def: ToolDefinition,
}

impl AgentSwarmTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("AgentSwarm"),
            description: "Run the same independent task shape across multiple inputs with bounded parallel sub-agents. The complete batch is validated before dispatch and returns ordered per-item results, including partial failures."
                .into(),
            help: Some(
                "Use AgentSwarm for independent map-style work over two or more items. Use Task for one job or differently shaped jobs. Every child follows the ordinary Task permission, isolation, diagnostics, and cancellation path. Agent-context resume is not supported."
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Short description for the whole swarm"
                    },
                    "agent_name": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Registered agent used for every item"
                    },
                    "prompt_template": {
                        "type": "string",
                        "minLength": 1,
                        "description": format!("Prompt template containing {PROMPT_TEMPLATE_PLACEHOLDER}")
                    },
                    "items": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": MAX_AGENT_SWARM_TASKS,
                        "items": { "type": "string", "minLength": 1 },
                        "description": "Values substituted into the prompt template in input order"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["build", "plan", "explore", "general"],
                        "description": "Optional mode override applied to every child"
                    },
                    "context_strategy": {
                        "type": "string",
                        "enum": ["none", "summary", "filtered", "full"],
                        "description": "Optional context strategy applied to every child"
                    },
                    "result_schema": {
                        "type": "object",
                        "description": "Optional JSON Schema applied independently to every child result",
                        "additionalProperties": true
                    },
                    "workspace_isolation": {
                        "type": "string",
                        "enum": ["auto", "shared", "prefer_worktree", "require_worktree"],
                        "description": "Optional workspace isolation request applied independently to every child"
                    }
                },
                "required": ["description", "agent_name", "prompt_template", "items"],
                "additionalProperties": false
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for AgentSwarmTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for AgentSwarmTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let prepared = prepare_agent_swarm(&input.arguments)?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "delegate_swarm",
                "description": prepared.description,
                "agent_name": prepared.agent_name,
                "tasks": prepared.tasks,
                "status": "pending"
            }),
            warnings: Vec::new(),
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}
