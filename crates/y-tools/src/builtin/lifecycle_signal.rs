use y_core::tool::{ToolError, ToolInput, ToolOutput};

pub(crate) fn validate_required_strings(
    input: &ToolInput,
    fields: &[&str],
) -> Result<(), ToolError> {
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

pub(crate) fn pending_output(action: &str, arguments: &serde_json::Value) -> ToolOutput {
    signal_output(serde_json::json!({
        "action": action,
        "arguments": arguments,
        "status": "pending"
    }))
}

pub(crate) fn signal_output(content: serde_json::Value) -> ToolOutput {
    ToolOutput {
        success: true,
        content,
        warnings: vec![],
        metadata: serde_json::json!({}),
    }
}

macro_rules! define_lifecycle_signal_tool {
    ($type_name:ident, $tool_name:literal, $description:literal, $parameters:expr, $required:expr, $category:expr, $dangerous:expr) => {
        #[doc = concat!("Signal tool for `", $tool_name, "`.")]
        pub struct $type_name {
            def: y_core::tool::ToolDefinition,
        }

        impl $type_name {
            /// Create the lifecycle signal tool.
            pub fn new() -> Self {
                Self {
                    def: Self::tool_definition(),
                }
            }

            /// Return the tool definition used for discovery and validation.
            pub fn tool_definition() -> y_core::tool::ToolDefinition {
                y_core::tool::ToolDefinition {
                    name: y_core::types::ToolName::from_string($tool_name),
                    description: $description.into(),
                    help: None,
                    parameters: $parameters,
                    result_schema: None,
                    category: $category,
                    tool_type: y_core::tool::ToolType::BuiltIn,
                    capabilities: y_core::runtime::RuntimeCapability::default(),
                    is_dangerous: $dangerous,
                }
            }
        }

        impl Default for $type_name {
            fn default() -> Self {
                Self::new()
            }
        }

        #[async_trait::async_trait]
        impl y_core::tool::Tool for $type_name {
            async fn execute(
                &self,
                input: y_core::tool::ToolInput,
            ) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
                $crate::builtin::lifecycle_signal::validate_required_strings(&input, $required)?;
                Ok($crate::builtin::lifecycle_signal::pending_output(
                    $tool_name,
                    &input.arguments,
                ))
            }

            fn definition(&self) -> &y_core::tool::ToolDefinition {
                &self.def
            }
        }
    };
}

pub(crate) use define_lifecycle_signal_tool;

#[cfg(test)]
mod tests {
    use serde_json::json;
    use y_core::tool::{ToolError, ToolInput};
    use y_core::types::{SessionId, ToolName};

    use super::{pending_output, validate_required_strings};

    #[test]
    fn required_string_validation_rejects_whitespace() {
        let input = ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("Create"),
            arguments: json!({"name": "  "}),
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        };

        assert!(matches!(
            validate_required_strings(&input, &["name"]),
            Err(ToolError::ValidationError { .. })
        ));
    }

    #[test]
    fn pending_signal_preserves_action_and_arguments() {
        let arguments = json!({"name": "example"});
        let output = pending_output("Create", &arguments);

        assert!(output.success);
        assert_eq!(output.content["action"], "Create");
        assert_eq!(output.content["arguments"], arguments);
        assert_eq!(output.content["status"], "pending");
    }
}
