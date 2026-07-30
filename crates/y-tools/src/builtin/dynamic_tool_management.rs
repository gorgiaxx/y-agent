//! Signal tools for the durable dynamic-tool lifecycle.
//!
//! `y-service` intercepts these calls so mutations remain configuration-gated,
//! durable, registry-synchronized, and subject to normal dangerous-tool HITL.

use super::lifecycle_signal::define_lifecycle_signal_tool;

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Custom,
    true
);

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Custom,
    true
);

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Custom,
    true
);

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Custom,
    false
);

define_lifecycle_signal_tool!(
    ToolListTool,
    "ToolList",
    "List durable dynamic tools with optional name or description filtering.",
    serde_json::json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "additionalProperties": false
    }),
    &[],
    y_core::tool::ToolCategory::Custom,
    false
);
