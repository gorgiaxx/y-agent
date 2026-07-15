//! Signal tools for durable dynamic-agent lifecycle management.
//!
//! `y-service` intercepts these calls and performs persistence plus live
//! registry synchronization. The implementations here provide schemas and
//! standalone argument validation for capability discovery.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

fn validate_required_strings(input: &ToolInput, fields: &[&str]) -> Result<(), ToolError> {
    for field in fields {
        let present = input
            .arguments
            .get(*field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if !present {
            return Err(ToolError::ValidationError {
                message: format!("'{field}' is required"),
            });
        }
    }
    Ok(())
}

fn pending_output(action: &str, arguments: &serde_json::Value) -> ToolOutput {
    ToolOutput {
        success: true,
        content: serde_json::json!({
            "action": action,
            "arguments": arguments,
            "status": "pending"
        }),
        warnings: vec![],
        metadata: serde_json::json!({}),
    }
}

macro_rules! define_agent_lifecycle_tool {
    ($type_name:ident, $tool_name:literal, $description:literal, $parameters:expr, $required:expr, $dangerous:expr) => {
        #[doc = concat!("Signal tool for `", $tool_name, "`.")]
        pub struct $type_name {
            def: ToolDefinition,
        }

        impl $type_name {
            /// Create the lifecycle signal tool.
            pub fn new() -> Self {
                Self {
                    def: Self::tool_definition(),
                }
            }

            /// Return the tool definition used for discovery and validation.
            pub fn tool_definition() -> ToolDefinition {
                ToolDefinition {
                    name: ToolName::from_string($tool_name),
                    description: $description.into(),
                    help: None,
                    parameters: $parameters,
                    result_schema: None,
                    category: ToolCategory::Agent,
                    tool_type: ToolType::BuiltIn,
                    capabilities: RuntimeCapability::default(),
                    is_dangerous: $dangerous,
                }
            }
        }

        impl Default for $type_name {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait]
        impl Tool for $type_name {
            async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
                validate_required_strings(&input, $required)?;
                Ok(pending_output($tool_name, &input.arguments))
            }

            fn definition(&self) -> &ToolDefinition {
                &self.def
            }
        }
    };
}

define_agent_lifecycle_tool!(
    AgentCreateTool,
    "AgentCreate",
    "Create a durable, runtime-callable dynamic agent. Its tools and numeric limits are always intersected with the creator's effective permissions, and the definition is security-screened before activation.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Unique lowercase agent name; runtime ID becomes dyn-<name>"
            },
            "description": {
                "type": "string",
                "description": "Specific purpose and task boundary for the agent"
            },
            "mode": {
                "type": "string",
                "enum": ["build", "plan", "explore", "general"],
                "default": "general"
            },
            "capabilities": {
                "type": "array",
                "items": { "type": "string" },
                "default": []
            },
            "allowed_tools": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Requested tools; every tool must also be available to the creator",
                "default": []
            },
            "system_prompt": {
                "type": "string",
                "default": ""
            },
            "context_sharing": {
                "type": "string",
                "enum": ["none", "summary", "filtered", "full"],
                "default": "none"
            }
        },
        "required": ["name", "description"],
        "additionalProperties": false
    }),
    &["name", "description"],
    true
);

define_agent_lifecycle_tool!(
    AgentUpdateTool,
    "AgentUpdate",
    "Update a durable dynamic agent and immediately replace its live delegation definition. Updates cannot expand beyond the permissions inherited at creation.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Dynamic agent ID" },
            "description": { "type": "string" },
            "mode": {
                "type": "string",
                "enum": ["build", "plan", "explore", "general"]
            },
            "allowed_tools": {
                "type": "array",
                "items": { "type": "string" }
            },
            "system_prompt": { "type": "string" }
        },
        "required": ["id"],
        "additionalProperties": false
    }),
    &["id"],
    true
);

define_agent_lifecycle_tool!(
    AgentDeactivateTool,
    "AgentDeactivate",
    "Soft-delete a dynamic agent with an audit reason and remove it from new task delegations while preserving its durable history.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Dynamic agent ID" },
            "reason": { "type": "string", "description": "Audit reason for deactivation" }
        },
        "required": ["id", "reason"],
        "additionalProperties": false
    }),
    &["id", "reason"],
    true
);

define_agent_lifecycle_tool!(
    AgentSearchTool,
    "AgentSearch",
    "Search durable dynamic agents by name, description, or capability, with optional lifecycle and mode filters.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "default": "" },
            "mode": {
                "type": "string",
                "enum": ["build", "plan", "explore", "general"]
            },
            "trust_tier": {
                "type": "string",
                "enum": ["built_in", "user_defined", "dynamic"]
            },
            "status": {
                "type": "string",
                "enum": ["active", "deactivated"],
                "default": "active"
            }
        },
        "additionalProperties": false
    }),
    &[],
    false
);

define_agent_lifecycle_tool!(
    AgentEvaluateTool,
    "AgentEvaluate",
    "Evaluate durable execution evidence for dynamic-agent versions and report statistically supported regressions. This tool is read-only; use supervised lifecycle tools for any mutation.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "Optional dynamic agent ID to evaluate; omit for all agents"
            },
            "min_samples": {
                "type": "integer",
                "minimum": 2,
                "maximum": 100,
                "default": 5
            },
            "max_success_rate_drop": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 1.0,
                "default": 0.25
            }
        },
        "additionalProperties": false
    }),
    &[],
    false
);

define_agent_lifecycle_tool!(
    AgentProposalListTool,
    "AgentProposalList",
    "List durable governed dynamic-agent evolution proposals, including evidence, decisions, and applied versions.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": { "type": "string" },
            "status": {
                "type": "string",
                "enum": ["pending", "approved", "rejected", "deferred", "applied", "failed"]
            }
        },
        "additionalProperties": false
    }),
    &[],
    false
);

define_agent_lifecycle_tool!(
    AgentProposalRefineTool,
    "AgentProposalRefine",
    "Ask the read-only agent-refiner to draft a permission-safe candidate update for a governed regression proposal. The candidate is validated and persisted, but the active agent is not mutated until separate approval.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "proposal_id": { "type": "string" },
            "instructions": {
                "type": "string",
                "description": "Optional reviewer constraints for the candidate draft"
            }
        },
        "required": ["proposal_id"],
        "additionalProperties": false
    }),
    &["proposal_id"],
    false
);

define_agent_lifecycle_tool!(
    AgentProposalDecideTool,
    "AgentProposalDecide",
    "Approve, reject, or defer a governed dynamic-agent proposal. Approval may apply a validated candidate or reversible rollback and therefore requires dangerous-tool authorization.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "proposal_id": { "type": "string" },
            "decision": {
                "type": "string",
                "enum": ["approve", "reject", "defer"]
            },
            "reason": { "type": "string" }
        },
        "required": ["proposal_id", "decision"],
        "additionalProperties": false
    }),
    &["proposal_id", "decision"],
    true
);
