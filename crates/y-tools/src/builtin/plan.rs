//! `Plan` tool: trigger the plan-mode orchestration workflow.
//!
//! This is a **signal tool** -- its `execute()` method validates input and
//! returns a pending descriptor. The actual orchestration (creating child
//! sessions, delegating to the `plan-writer` sub-agent, resolving the review
//! policy, and then executing approved phases) is performed by
//! `PlanOrchestrator` in `y-service`, which intercepts `Plan` tool calls in
//! `tool_dispatch.rs`.
//!
//! Follows the same pattern as `Task` / `TaskDelegationOrchestrator`.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// The `Plan` tool for plan-mode orchestration.
///
/// When invoked by the LLM, it triggers a multi-stage planning workflow:
/// 1. A `plan-writer` sub-agent produces a structured JSON plan with tasks
/// 2. The configured operation mode decides whether review is automatic or manual
/// 3. Executable tasks are run by phase-executor sub-agents
///
/// The presentation layer (GUI) renders child session transcripts inline.
pub struct PlanTool {
    def: ToolDefinition,
}

impl PlanTool {
    /// Create a new `Plan` tool.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `Plan`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("Plan"),
            description: "Create a structured plan for complex tasks, render it for \
                review, and follow the current operation mode plus Guardrails \
                plan-review policy. Delegates to sub-agents for plan writing and \
                phased execution. Use this for multi-file changes, architectural \
                design, refactoring, or multi-step coordination."
                .into(),
            help: Some(
                "Triggers a multi-stage planning workflow:\n\
                 1. plan-writer sub-agent produces a structured JSON plan with tasks\n\
                 2. operation mode and Guardrails decide auto vs manual review\n\
                 3. executable phases are run by dedicated sub-agents\n\
                 \n\
                 Parameters:\n\
                 - request (required): The user's original task description\n\
                 - context (optional): Additional context from prior exploration"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "The user's original task description to plan for"
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional additional context from prior exploration"
                    }
                },
                "required": ["request"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for PlanTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PlanTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let request = input.arguments.get("request").and_then(|v| v.as_str());
        let context = input
            .arguments
            .get("context")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let Some(request) = request else {
            return Err(ToolError::ValidationError {
                message: "'request' is required".into(),
            });
        };

        // The actual orchestration is performed by PlanOrchestrator in
        // y-service. This tool validates input and returns a descriptor.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "plan",
                "request": request,
                "context": context,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    use y_core::types::SessionId;

    use super::*;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("Plan"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_plan_with_valid_args() {
        let tool = PlanTool::new();
        let input = make_input(serde_json::json!({
            "request": "Refactor the plan mode architecture"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "plan");
        assert_eq!(
            output.content["request"],
            "Refactor the plan mode architecture"
        );
        assert_eq!(output.content["status"], "pending");
    }

    #[tokio::test]
    async fn test_plan_with_context() {
        let tool = PlanTool::new();
        let input = make_input(serde_json::json!({
            "request": "Add new feature",
            "context": "The codebase uses Rust and has 24 crates"
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(
            output.content["context"],
            "The codebase uses Rust and has 24 crates"
        );
    }

    #[tokio::test]
    async fn test_plan_missing_request() {
        let tool = PlanTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[test]
    fn test_plan_definition() {
        let def = PlanTool::tool_definition();
        assert_eq!(def.name.as_str(), "Plan");
        assert_eq!(def.category, ToolCategory::Agent);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        assert!(def.description.contains("operation mode"));
        assert!(def.description.contains("Guardrails"));
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("request"));
        assert!(props.contains_key("context"));
        let required = def.parameters["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
    }
}
