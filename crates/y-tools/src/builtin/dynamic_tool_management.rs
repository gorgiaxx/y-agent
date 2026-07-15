//! Signal tools for the durable dynamic-tool lifecycle.
//!
//! `y-service` intercepts these calls so mutations remain configuration-gated,
//! durable, registry-synchronized, and subject to normal dangerous-tool HITL.

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

macro_rules! define_dynamic_tool_lifecycle_tool {
    ($type_name:ident, $tool_name:literal, $description:literal, $parameters:expr, $required:expr, $dangerous:expr) => {
        #[doc = concat!("Signal tool for `", $tool_name, "`.")]
        pub struct $type_name {
            def: ToolDefinition,
        }

        impl $type_name {
            pub fn new() -> Self {
                Self {
                    def: Self::tool_definition(),
                }
            }

            pub fn tool_definition() -> ToolDefinition {
                ToolDefinition {
                    name: ToolName::from_string($tool_name),
                    description: $description.into(),
                    help: None,
                    parameters: $parameters,
                    result_schema: None,
                    category: ToolCategory::Custom,
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

define_dynamic_tool_lifecycle_tool!(
    ToolCreateTool,
    "ToolCreate",
    "Create and activate a durable sandboxed script tool. Dynamic tools must be explicitly enabled, use an approved interpreter, and pass dangerous-tool authorization.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "minLength": 1, "maxLength": 64 },
            "description": { "type": "string", "minLength": 1, "maxLength": 500 },
            "parameters": {
                "type": "object",
                "description": "JSON Schema object for tool arguments"
            },
            "interpreter": {
                "type": "string",
                "enum": ["bash", "sh", "python", "python3", "node", "bun"]
            },
            "source": { "type": "string", "minLength": 1 }
        },
        "required": ["name", "description", "parameters", "interpreter", "source"],
        "additionalProperties": false
    }),
    &["name", "description", "interpreter", "source"],
    true
);

define_dynamic_tool_lifecycle_tool!(
    ToolUpdateTool,
    "ToolUpdate",
    "Update a durable dynamic script tool as a new version and replace its live registry definition. Omitted fields retain their current values.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "description": { "type": "string", "minLength": 1, "maxLength": 500 },
            "parameters": { "type": "object" },
            "interpreter": {
                "type": "string",
                "enum": ["bash", "sh", "python", "python3", "node", "bun"]
            },
            "source": { "type": "string", "minLength": 1 }
        },
        "required": ["name"],
        "additionalProperties": false
    }),
    &["name"],
    true
);

define_dynamic_tool_lifecycle_tool!(
    ToolDeleteTool,
    "ToolDelete",
    "Delete a dynamic tool from the live registry while preserving its append-only lifecycle journal.",
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["name", "reason"],
        "additionalProperties": false
    }),
    &["name", "reason"],
    true
);

define_dynamic_tool_lifecycle_tool!(
    ToolGetTool,
    "ToolGet",
    "Get one durable dynamic-tool definition, version, creator, and execution kind.",
    serde_json::json!({
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "required": ["name"],
        "additionalProperties": false
    }),
    &["name"],
    false
);

define_dynamic_tool_lifecycle_tool!(
    ToolListTool,
    "ToolList",
    "List durable dynamic tools with optional name or description filtering.",
    serde_json::json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "additionalProperties": false
    }),
    &[],
    false
);
