//! `ToolExecutor`: validates parameters and executes tools through middleware.

use std::sync::Arc;

use tracing::instrument;

use y_core::hook::{ChainType, MiddlewareContext};
use y_core::tool::{ToolInput, ToolOutput};
use y_core::types::ToolName;

use crate::error::ToolRegistryError;
use crate::registry::ToolRegistryImpl;
use crate::validator::JsonSchemaValidator;

/// Executes tools with parameter validation and middleware integration.
///
/// The executor:
/// 1. Looks up the tool in the registry.
/// 2. Validates parameters against its JSON Schema.
/// 3. Runs the Tool middleware chain (pre-execution).
/// 4. Executes the tool.
/// 5. Runs the Tool middleware chain (post-execution).
pub struct ToolExecutor {
    validator: JsonSchemaValidator,
    /// Optional middleware chain for tool execution (from y-hooks).
    middleware_chain: Option<Arc<y_hooks::MiddlewareChain>>,
}

impl ToolExecutor {
    /// Create a new tool executor.
    pub fn new() -> Self {
        Self {
            validator: JsonSchemaValidator::new(),
            middleware_chain: None,
        }
    }

    /// Create a new tool executor with a middleware chain.
    pub fn with_middleware(chain: Arc<y_hooks::MiddlewareChain>) -> Self {
        Self {
            validator: JsonSchemaValidator::new(),
            middleware_chain: Some(chain),
        }
    }

    /// Execute a tool by name with the given input.
    #[instrument(skip(self, registry, input), fields(tool_name = %name.as_str()))]
    pub async fn execute(
        &mut self,
        registry: &ToolRegistryImpl,
        name: &ToolName,
        input: ToolInput,
    ) -> Result<ToolOutput, ToolRegistryError> {
        // 1. Look up the tool.
        let tool = registry
            .get_tool(name)
            .await
            .ok_or_else(|| ToolRegistryError::NotFound {
                name: name.as_str().to_string(),
            })?;

        // 2. Validate parameters against the tool's JSON Schema.
        let definition =
            registry
                .get_definition(name)
                .await
                .ok_or_else(|| ToolRegistryError::NotFound {
                    name: name.as_str().to_string(),
                })?;

        self.validator
            .validate(&definition.parameters, &input.arguments)?;

        // 3. Run pre-execution middleware (if configured).
        if let Some(ref chain) = self.middleware_chain {
            let mut ctx = MiddlewareContext {
                chain_type: ChainType::Tool,
                payload: serde_json::json!({
                    "tool_name": name.as_str(),
                    "arguments": input.arguments,
                    "phase": "pre"
                }),
                metadata: serde_json::json!({}),
                aborted: false,
                abort_reason: None,
            };

            chain
                .execute(&mut ctx)
                .await
                .map_err(|e| ToolRegistryError::MiddlewareError {
                    message: e.to_string(),
                })?;

            if ctx.aborted {
                return Err(ToolRegistryError::ExecutionError {
                    message: format!(
                        "tool execution aborted by middleware: {}",
                        ctx.abort_reason.unwrap_or_default()
                    ),
                });
            }
        }

        // 4. Execute the tool.
        let output = tool
            .execute(input)
            .await
            .map_err(|e| ToolRegistryError::ExecutionError {
                message: e.to_string(),
            })?;

        // 5. Run post-execution middleware (if configured).
        if let Some(ref chain) = self.middleware_chain {
            let mut ctx = MiddlewareContext {
                chain_type: ChainType::Tool,
                payload: serde_json::json!({
                    "tool_name": name.as_str(),
                    "result": output.content,
                    "phase": "post"
                }),
                metadata: serde_json::json!({}),
                aborted: false,
                abort_reason: None,
            };

            if let Err(e) = chain.execute(&mut ctx).await {
                tracing::warn!(tool = %name.as_str(), error = %e, "post-execution middleware failed");
            }
        }

        Ok(output)
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use y_core::runtime::RuntimeCapability;
    use y_core::tool::{Tool, ToolCategory, ToolDefinition, ToolError, ToolType};
    use y_core::types::SessionId;

    use super::*;

    struct EchoTool {
        def: ToolDefinition,
    }

    #[async_trait::async_trait]
    impl Tool for EchoTool {
        async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                success: true,
                content: input.arguments,
                warnings: vec![],
                metadata: serde_json::json!({}),
            })
        }

        fn definition(&self) -> &ToolDefinition {
            &self.def
        }
    }

    async fn make_registry() -> ToolRegistryImpl {
        let reg = ToolRegistryImpl::new(crate::config::ToolRegistryConfig::default());
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        });
        let def = ToolDefinition {
            name: ToolName::from_string("echo"),
            description: "echo tool".into(),
            help: None,
            parameters: schema,
            result_schema: None,
            category: ToolCategory::Custom,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        };
        let tool = Arc::new(EchoTool { def: def.clone() }) as Arc<dyn Tool>;
        reg.register_tool(tool, def).await.unwrap();
        reg
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("echo"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_executor_valid_params() {
        let reg = make_registry().await;
        let mut executor = ToolExecutor::new();
        let input = make_input(serde_json::json!({"message": "hello"}));
        let result = executor
            .execute(&reg, &ToolName::from_string("echo"), input)
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.content["message"], "hello");
    }

    #[tokio::test]
    async fn test_executor_invalid_params_rejected() {
        let reg = make_registry().await;
        let mut executor = ToolExecutor::new();
        let input = make_input(serde_json::json!({"wrong_field": 123}));
        let result = executor
            .execute(&reg, &ToolName::from_string("echo"), input)
            .await;
        assert!(matches!(
            result,
            Err(ToolRegistryError::ValidationError { .. })
        ));
    }

    #[tokio::test]
    async fn test_executor_tool_not_found() {
        let reg = make_registry().await;
        let mut executor = ToolExecutor::new();
        let input = make_input(serde_json::json!({}));
        let result = executor
            .execute(&reg, &ToolName::from_string("nonexistent"), input)
            .await;
        assert!(matches!(result, Err(ToolRegistryError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_executor_with_middleware_chain() {
        let chain = y_hooks::MiddlewareChain::new(ChainType::Tool);
        let reg = make_registry().await;
        let mut executor = ToolExecutor::with_middleware(Arc::new(chain));
        let input = make_input(serde_json::json!({"message": "hello"}));
        let result = executor
            .execute(&reg, &ToolName::from_string("echo"), input)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_executor_schema_cached_across_calls() {
        let reg = make_registry().await;
        let mut executor = ToolExecutor::new();
        let input1 = make_input(serde_json::json!({"message": "first"}));
        let input2 = make_input(serde_json::json!({"message": "second"}));
        executor
            .execute(&reg, &ToolName::from_string("echo"), input1)
            .await
            .unwrap();
        executor
            .execute(&reg, &ToolName::from_string("echo"), input2)
            .await
            .unwrap();
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "message": { "type": "string" } },
            "required": ["message"]
        });
        assert!(executor.validator.is_cached(&schema));
    }
}
