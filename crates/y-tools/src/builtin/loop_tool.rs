//! `Loop` tool: trigger the iterative convergence workflow.
//!
//! This is a **signal tool** -- its `execute()` method validates input and
//! returns a pending descriptor. The actual orchestration (creating child
//! sessions, managing rounds, reading progress files) is performed by
//! `LoopOrchestrator` in `y-service`, which intercepts `Loop` tool calls
//! in `tool_dispatch.rs`.
//!
//! Follows the same pattern as `Plan` / `PlanOrchestrator`.

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

const DEFAULT_MAX_ROUNDS: usize = 10;
const MIN_MAX_ROUNDS: usize = 2;
const MAX_ROUNDS_CEILING: usize = 25;

pub struct LoopTool {
    def: ToolDefinition,
}

impl LoopTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("Loop"),
            description: "Work on a task iteratively until convergence. Spawns fresh \
                agent rounds, each reading a persistent progress file. Each round works \
                on remaining tasks, updates progress, and signals when done. A mandatory \
                self-review verifies completion before stopping. Use this for complex \
                tasks that benefit from iterative refinement."
                .into(),
            help: Some(
                "Triggers an iterative convergence workflow:\n\
                 1. Creates a progress file as persistent inter-round memory\n\
                 2. Spawns fresh agent rounds, each reading the progress file\n\
                 3. Each round works on remaining tasks and updates progress\n\
                 4. A mandatory self-review verifies completion before stopping\n\
                 5. Returns results when converged or budget exhausted\n\
                 \n\
                 Parameters:\n\
                 - request (required): The user's task description\n\
                 - context (optional): Additional context from prior exploration\n\
                 - max_rounds (optional): Maximum rounds before forced stop (default: 10, max: 25)"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "The user's task description to work on iteratively"
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional additional context"
                    },
                    "max_rounds": {
                        "type": "integer",
                        "description": "Maximum rounds before forced stop (default: 10, max: 25)"
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

impl Default for LoopTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LoopTool {
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

        let max_rounds = input
            .arguments
            .get("max_rounds")
            .and_then(serde_json::Value::as_u64)
            .map_or(DEFAULT_MAX_ROUNDS, |v| (v as usize).clamp(MIN_MAX_ROUNDS, MAX_ROUNDS_CEILING));

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "loop",
                "request": request,
                "context": context,
                "max_rounds": max_rounds,
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
            name: ToolName::from_string("Loop"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_loop_with_valid_args() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Research distributed consensus algorithms"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "loop");
        assert_eq!(
            output.content["request"],
            "Research distributed consensus algorithms"
        );
        assert_eq!(output.content["status"], "pending");
        assert_eq!(output.content["max_rounds"], DEFAULT_MAX_ROUNDS);
    }

    #[tokio::test]
    async fn test_loop_with_context() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Build a task queue",
            "context": "Using Redis Streams as the backend"
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(
            output.content["context"],
            "Using Redis Streams as the backend"
        );
    }

    #[tokio::test]
    async fn test_loop_missing_request() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_loop_max_rounds_default() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Do something"
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["max_rounds"], DEFAULT_MAX_ROUNDS);
    }

    #[tokio::test]
    async fn test_loop_max_rounds_clamped_high() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Do something",
            "max_rounds": 100
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["max_rounds"], MAX_ROUNDS_CEILING);
    }

    #[tokio::test]
    async fn test_loop_max_rounds_clamped_low() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Do something",
            "max_rounds": 1
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["max_rounds"], MIN_MAX_ROUNDS);
    }

    #[tokio::test]
    async fn test_loop_max_rounds_custom() {
        let tool = LoopTool::new();
        let input = make_input(serde_json::json!({
            "request": "Do something",
            "max_rounds": 15
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["max_rounds"], 15);
    }

    #[test]
    fn test_loop_definition() {
        let def = LoopTool::tool_definition();
        assert_eq!(def.name.as_str(), "Loop");
        assert_eq!(def.category, ToolCategory::Agent);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("request"));
        assert!(props.contains_key("context"));
        assert!(props.contains_key("max_rounds"));
        let required = def.parameters["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
    }
}
