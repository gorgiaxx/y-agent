//! Signal tools for governed skill-evolution proposal management.
//!
//! `y-service` intercepts these calls to load durable evidence, delegate
//! candidate generation, validate candidates, and apply supervised decisions.

use async_trait::async_trait;
use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

fn validate_required_strings(input: &ToolInput, fields: &[&str]) -> Result<(), ToolError> {
    for field in fields {
        if input
            .arguments
            .get(*field)
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
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

macro_rules! define_skill_evolution_tool {
    ($type_name:ident, $tool_name:literal, $description:literal, $parameters:expr, $required:expr, $dangerous:expr) => {
        #[doc = concat!("Signal tool for `", $tool_name, "`.")]
        pub struct $type_name {
            def: ToolDefinition,
        }

        impl $type_name {
            /// Create the skill-evolution signal tool.
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

define_skill_evolution_tool!(
    SkillProposalListTool,
    "SkillProposalList",
    "List durable governed skill-evolution proposals without exposing full candidate documents in bulk.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": { "type": "string" },
            "status": {
                "type": "string",
                "enum": [
                    "pending_approval", "approved", "rejected", "deferred",
                    "promoted", "rolled_back"
                ]
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "default": 20
            }
        },
        "additionalProperties": false
    }),
    &[],
    false
);

define_skill_evolution_tool!(
    SkillProposalRefineTool,
    "SkillProposalRefine",
    "Ask the tool-free skill-refiner to draft and validate an evidence-backed candidate. The candidate is persisted for review but the active skill is not mutated.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "proposal_id": { "type": "string" },
            "instructions": {
                "type": "string",
                "description": "Optional reviewer constraints for candidate generation"
            }
        },
        "required": ["proposal_id"],
        "additionalProperties": false
    }),
    &["proposal_id"],
    false
);

define_skill_evolution_tool!(
    SkillProposalDecideTool,
    "SkillProposalDecide",
    "Approve, reject, or defer a governed skill proposal. Approval validates and activates only the persisted candidate as a reversible version and therefore requires dangerous-tool authorization.",
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
