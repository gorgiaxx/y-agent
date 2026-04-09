//! `Plan` tool: trigger the plan-mode orchestration workflow.
//!
//! This is a **signal tool** -- its `execute()` method validates input and
//! returns a pending descriptor. The actual orchestration (creating child
//! sessions, delegating to `plan-writer` and `task-decomposer` sub-agents,
//! executing phases) is performed by `PlanOrchestrator` in `y-service`,
//! which intercepts `Plan` tool calls in `tool_dispatch.rs`.
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
/// 1. A `plan-writer` sub-agent explores the codebase and writes a plan
/// 2. A `task-decomposer` sub-agent converts the plan into structured tasks
/// 3. Each task is executed sequentially by phase-executor sub-agents
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
            description: "Create and execute a structured plan for complex tasks. \
                Delegates to sub-agents for codebase exploration, plan writing, \
                task decomposition, and phased execution. Use this for multi-file \
                changes, architectural design, refactoring, or multi-step coordination."
                .into(),
            help: Some(
                "Triggers a multi-stage planning workflow:\n\
                 1. plan-writer sub-agent explores the codebase and writes a phased plan\n\
                 2. task-decomposer sub-agent converts the plan into structured tasks\n\
                 3. Each phase is executed sequentially by dedicated sub-agents\n\
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
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("request"));
        assert!(props.contains_key("context"));
        let required = def.parameters["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
    }
}
