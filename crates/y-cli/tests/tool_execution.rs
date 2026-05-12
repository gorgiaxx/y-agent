//! E2E integration test: Tool registration, dispatch, and execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use y_core::runtime::{ExecutionResult, ResourceUsage, RuntimeCapability};
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::{SessionId, ToolName};
use y_test_utils::MockRuntime;

/// A simple echo tool for testing.
struct EchoTool {
    definition: ToolDefinition,
}

impl EchoTool {
    fn new() -> Self {
        Self {
            definition: ToolDefinition {
                name: ToolName::from_string("echo"),
                description: "Echoes the input text".into(),
                help: None,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"}
                    },
                    "required": ["text"]
                }),
                result_schema: None,
                category: ToolCategory::Custom,
                tool_type: ToolType::BuiltIn,
                capabilities: RuntimeCapability::default(),
                is_dangerous: false,
            },
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let text = input.arguments["text"]
            .as_str()
            .unwrap_or("(empty)")
            .to_string();
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({"echo": text}),
            warnings: vec![],
            metadata: serde_json::Value::Null,
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
}

#[tokio::test]
async fn e2e_tool_register_and_execute() {
    let tool = EchoTool::new();

    // Execute directly
    let input = ToolInput {
        call_id: "call-1".into(),
        name: ToolName::from_string("echo"),
        arguments: serde_json::json!({"text": "hello world"}),
        session_id: SessionId::new(),
        working_dir: None,
        additional_read_dirs: vec![],
        command_runner: None,
    };

    let output = tool.execute(input).await.unwrap();
    assert!(output.success);
    assert_eq!(output.content["echo"], "hello world");
}

#[tokio::test]
async fn e2e_tool_with_mock_runtime() {
    let runtime = MockRuntime::new().with_result(
        "echo hello",
        ExecutionResult {
            exit_code: 0,
            stdout: b"hello\n".to_vec(),
            stderr: vec![],
            duration: Duration::from_millis(10),
            resource_usage: ResourceUsage::default(),
        },
    );

    use y_core::runtime::{ExecutionRequest, RuntimeAdapter};
    let req = ExecutionRequest {
        command: "echo hello".into(),
        args: vec![],
        working_dir: None,
        env: HashMap::new(),
        stdin: None,
        owner_session_id: None,
        capabilities: RuntimeCapability::default(),
        image: None,
    };

    let result = runtime.execute(req).await.unwrap();
    assert!(result.success());
    assert_eq!(result.stdout_string().trim(), "hello");
}

#[tokio::test]
async fn e2e_tool_registry_simulation() {
    // Simulate a simple tool registry using a HashMap
    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert("echo".into(), Arc::new(EchoTool::new()));

    // Look up a tool
    let tool = tools.get("echo").expect("tool should exist");
    assert_eq!(tool.definition().name, ToolName::from_string("echo"));

    // Execute it
    let input = ToolInput {
        call_id: "call-2".into(),
        name: ToolName::from_string("echo"),
        arguments: serde_json::json!({"text": "from registry"}),
        session_id: SessionId::new(),
        working_dir: None,
        additional_read_dirs: vec![],
        command_runner: None,
    };
    let output = tool.execute(input).await.unwrap();
    assert_eq!(output.content["echo"], "from registry");

    // Missing tool
    assert!(tools.get("nonexistent").is_none());
}
