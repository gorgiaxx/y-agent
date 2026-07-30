//! Signal tools for governed skill-evolution proposal management.
//!
//! `y-service` intercepts these calls to load durable evidence, delegate
//! candidate generation, validate candidates, and apply supervised decisions.

use super::lifecycle_signal::define_lifecycle_signal_tool;

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Agent,
    false
);

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Agent,
    false
);

define_lifecycle_signal_tool!(
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
    y_core::tool::ToolCategory::Agent,
    true
);
