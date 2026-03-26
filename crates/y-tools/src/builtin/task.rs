//! `task` tool: delegate work to a sub-agent.
//!
//! This tool allows the LLM to invoke agent delegation within a conversation.
//! The LLM discovers available agents via `tool_search` and then uses `task`
//! to delegate work to a specific agent.
//!
//! The `execute()` method validates input and returns a pending descriptor.
//! Actual delegation is performed by the `TaskDelegationOrchestrator` in
//! `y-service`, which intercepts `task` calls and routes them through the
//! `AgentDelegator` (same pattern as `tool_search` / `ToolSearchOrchestrator`).

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// The `task` tool for agent delegation.
///
/// When invoked by the LLM, it delegates a task to a named sub-agent.
/// The actual execution is handled by the orchestrator in `y-service`.
pub struct TaskTool {
    def: ToolDefinition,
}

impl TaskTool {
    /// Create a new `task` tool.
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    /// The tool definition for `task`.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("task"),
            description: "Delegate a task to a sub-agent. \
                Use tool_search to discover available agents first, \
                then call this tool with the agent's id and a prompt."
                .into(),
            help: Some(
                "Delegates a task to a registered sub-agent and returns the agent's output.\n\
                 \n\
                 Usage:\n\
                 1. Search for agents: tool_search({\"query\": \"agent-architect\"})\n\
                 2. Delegate: task({\"agent_name\": \"agent-architect\", \"prompt\": \"...\"})\n\
                 \n\
                 Optional parameters:\n\
                 - mode: Override the agent's default mode (build/plan/explore/general)\n\
                 - context_strategy: Control context sharing (none/summary/filtered/full)"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_name": {
                        "type": "string",
                        "description": "Name or ID of the target agent"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Task description to delegate to the agent"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["build", "plan", "explore", "general"],
                        "description": "Optional mode override for the delegation"
                    },
                    "context_strategy": {
                        "type": "string",
                        "enum": ["none", "summary", "filtered", "full"],
                        "description": "Optional context sharing strategy override"
                    }
                },
                "required": ["agent_name", "prompt"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for TaskTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TaskTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let agent_name = input.arguments.get("agent_name").and_then(|v| v.as_str());
        let prompt = input.arguments.get("prompt").and_then(|v| v.as_str());
        let mode = input.arguments.get("mode").and_then(|v| v.as_str());
        let context_strategy = input
            .arguments
            .get("context_strategy")
            .and_then(|v| v.as_str());

        let Some(agent_name) = agent_name else {
            return Err(ToolError::ValidationError {
                message: "'agent_name' is required".into(),
            });
        };

        let Some(prompt) = prompt else {
            return Err(ToolError::ValidationError {
                message: "'prompt' is required".into(),
            });
        };

        // The actual delegation is performed by TaskDelegationOrchestrator
        // in y-service. This tool validates input and returns a descriptor.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "delegate",
                "agent_name": agent_name,
                "prompt": prompt,
                "mode": mode,
                "context_strategy": context_strategy,
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
            name: ToolName::from_string("task"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_task_with_valid_args() {
        let tool = TaskTool::new();
        let input = make_input(serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "Design a disk info agent"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "delegate");
        assert_eq!(output.content["agent_name"], "agent-architect");
        assert_eq!(output.content["prompt"], "Design a disk info agent");
        assert_eq!(output.content["status"], "pending");
    }

    #[tokio::test]
    async fn test_task_with_all_params() {
        let tool = TaskTool::new();
        let input = make_input(serde_json::json!({
            "agent_name": "tool-engineer",
            "prompt": "Build a search tool",
            "mode": "build",
            "context_strategy": "summary"
        }));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["mode"], "build");
        assert_eq!(output.content["context_strategy"], "summary");
    }

    #[tokio::test]
    async fn test_task_missing_agent_name() {
        let tool = TaskTool::new();
        let input = make_input(serde_json::json!({"prompt": "do something"}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_task_missing_prompt() {
        let tool = TaskTool::new();
        let input = make_input(serde_json::json!({"agent_name": "agent-architect"}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_task_empty_args() {
        let tool = TaskTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_task_definition() {
        let def = TaskTool::tool_definition();
        assert_eq!(def.name.as_str(), "task");
        assert_eq!(def.category, ToolCategory::Agent);
        assert_eq!(def.tool_type, ToolType::BuiltIn);
        assert!(!def.is_dangerous);
        let props = def.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("agent_name"));
        assert!(props.contains_key("prompt"));
        assert!(props.contains_key("mode"));
        assert!(props.contains_key("context_strategy"));
        let required = def.parameters["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
    }
}
