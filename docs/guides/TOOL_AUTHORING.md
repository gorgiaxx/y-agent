# Tool Authoring Guide

This guide explains how to create custom tools for y-agent.

## Overview

Tools are the primary way y-agent interacts with the external world. The tool system supports four types:

| Type | Description | Sandbox |
|------|-------------|---------|
| **Built-in** | Compiled into the binary | No |
| **MCP** | Provided by MCP servers | Server-side |
| **Custom** | User-defined (config-loaded) | Configurable |
| **Dynamic** | Agent-created at runtime | Always |

## Creating a Built-in Tool

### 1. Define the Tool Struct

```rust
use async_trait::async_trait;
use y_core::tool::*;
use y_core::runtime::RuntimeCapability;
use y_core::types::ToolName;

struct MyTool {
    definition: ToolDefinition,
}

impl MyTool {
    fn new() -> Self {
        Self {
            definition: ToolDefinition {
                name: ToolName::from_string("my_tool"),
                description: "Does something useful".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input": {
                            "type": "string",
                            "description": "The input to process"
                        }
                    },
                    "required": ["input"]
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
```

### 2. Implement the `Tool` Trait

```rust
#[async_trait]
impl Tool for MyTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let text = input.arguments["input"]
            .as_str()
            .ok_or(ToolError::ValidationError {
                message: "missing 'input' field".into(),
            })?;

        // Your tool logic here
        let result = format!("Processed: {text}");

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({"result": result}),
            warnings: vec![],
            metadata: serde_json::Value::Null,
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
}
```

### 3. Register with the Tool Registry

```rust
registry.register(my_tool.definition().clone()).await?;
```

## JSON Schema for Parameters

Tool parameters are validated against JSON Schema (Draft 7) before execution. Define your schema carefully:

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "File path to read"
    },
    "encoding": {
      "type": "string",
      "enum": ["utf-8", "ascii", "binary"],
      "default": "utf-8"
    },
    "max_lines": {
      "type": "integer",
      "minimum": 1,
      "maximum": 10000
    }
  },
  "required": ["path"]
}
```

## Runtime Capabilities

Tools declare their runtime requirements via `RuntimeCapability`:

```rust
RuntimeCapability {
    network: false,        // needs network access?
    filesystem: true,      // needs filesystem access?
    process_spawn: false,  // needs to spawn processes?
    privileged: false,     // needs elevated permissions?
}
```

## Dangerous Tools

Mark tools that perform irreversible operations:

```rust
is_dangerous: true,  // triggers guardrail approval flow
```

## Error Handling

Use the appropriate `ToolError` variant:

| Error | When to Use | Retryable? |
|-------|-------------|------------|
| `NotFound` | Tool doesn't exist | No |
| `ValidationError` | Invalid parameters | No |
| `PermissionDenied` | Insufficient permissions | No |
| `Timeout` | Execution exceeded time limit | Yes |
| `RateLimited` | Too many requests | Yes |
| `RuntimeError` | Sandbox/runtime failure | No |
| `ExternalServiceError` | Third-party API failure | Yes |

## Testing

Use `y-test-utils` for mock-based testing:

```rust
use y_test_utils::MockRuntime;

#[tokio::test]
async fn test_my_tool() {
    let runtime = MockRuntime::new();
    let tool = MyTool::new();

    let input = ToolInput {
        call_id: "test-1".into(),
        name: ToolName::from_string("my_tool"),
        arguments: serde_json::json!({"input": "hello"}),
        session_id: SessionId::new(),
    };

    let output = tool.execute(input).await.unwrap();
    assert!(output.success);
}
```
