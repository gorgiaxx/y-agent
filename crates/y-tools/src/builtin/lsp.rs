use std::sync::Arc;

use async_trait::async_trait;
use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspOperation {
    Definition,
    References,
    Hover,
    DocumentSymbols,
    WorkspaceSymbols,
    Diagnostics,
}

impl LspOperation {
    pub fn tool_name(self) -> &'static str {
        match self {
            Self::Definition => "LspDefinition",
            Self::References => "LspReferences",
            Self::Hover => "LspHover",
            Self::DocumentSymbols => "LspDocumentSymbols",
            Self::WorkspaceSymbols => "LspWorkspaceSymbols",
            Self::Diagnostics => "LspDiagnostics",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Definition => "Find the definition of the symbol at a source position.",
            Self::References => "Find references to the symbol at a source position.",
            Self::Hover => "Get type and documentation information at a source position.",
            Self::DocumentSymbols => "List semantic symbols in a source file.",
            Self::WorkspaceSymbols => "Search semantic symbols across a project.",
            Self::Diagnostics => "Get current language-server diagnostics for a source file.",
        }
    }

    pub fn tool_definition(self) -> ToolDefinition {
        let (properties, required) = match self {
            Self::Definition | Self::Hover => {
                (position_properties(), vec!["path", "line", "character"])
            }
            Self::References => {
                let mut properties = position_properties();
                properties["include_declaration"] = serde_json::json!({
                    "type": "boolean",
                    "default": true,
                    "description": "Include the symbol declaration in reference results."
                });
                (properties, vec!["path", "line", "character"])
            }
            Self::DocumentSymbols | Self::Diagnostics => (
                serde_json::json!({
                    "path": {
                        "type": "string",
                        "description": "Source file path, absolute or relative to the workspace."
                    }
                }),
                vec!["path"],
            ),
            Self::WorkspaceSymbols => (
                serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "Symbol name or query text."
                    },
                    "working_directory": {
                        "type": "string",
                        "description": "Project directory used for language-server selection."
                    },
                    "language": {
                        "type": "string",
                        "description": "Configured server id or language id when a workspace contains multiple languages."
                    }
                }),
                vec!["query"],
            ),
        };
        ToolDefinition {
            name: ToolName::from_string(self.tool_name()),
            description: self.description().to_string(),
            help: Some(
                "Language-server results are read-only evidence. Line and character are zero-based."
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            }),
            result_schema: None,
            category: ToolCategory::Search,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

pub struct LspTool {
    operation: LspOperation,
    definition: ToolDefinition,
}

impl LspTool {
    pub fn new(operation: LspOperation) -> Self {
        Self {
            operation,
            definition: operation.tool_definition(),
        }
    }
}

#[async_trait]
impl Tool for LspTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "status": "pending",
                "operation": self.operation.tool_name(),
                "arguments": input.arguments,
            }),
            warnings: Vec::new(),
            metadata: serde_json::json!({"intercepted_by": "y-service"}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

pub fn lsp_tools() -> Vec<Arc<dyn Tool>> {
    [
        LspOperation::Definition,
        LspOperation::References,
        LspOperation::Hover,
        LspOperation::DocumentSymbols,
        LspOperation::WorkspaceSymbols,
        LspOperation::Diagnostics,
    ]
    .into_iter()
    .map(|operation| Arc::new(LspTool::new(operation)) as Arc<dyn Tool>)
    .collect()
}

fn position_properties() -> serde_json::Value {
    serde_json::json!({
        "path": {
            "type": "string",
            "description": "Source file path, absolute or relative to the workspace."
        },
        "line": {
            "type": "integer",
            "minimum": 0,
            "description": "Zero-based line number."
        },
        "character": {
            "type": "integer",
            "minimum": 0,
            "description": "Zero-based UTF-16 character offset."
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{lsp_tools, LspOperation};

    #[test]
    fn lsp_tool_definitions_are_read_only_and_non_dangerous() {
        let tools = lsp_tools();

        assert_eq!(tools.len(), 6);
        for tool in tools {
            assert!(tool.is_read_only());
            assert!(!tool.definition().is_dangerous);
        }
    }

    #[test]
    fn position_operations_require_path_line_and_character() {
        for operation in [
            LspOperation::Definition,
            LspOperation::References,
            LspOperation::Hover,
        ] {
            let definition = operation.tool_definition();
            let required = definition.parameters["required"]
                .as_array()
                .expect("required fields");
            assert!(required.iter().any(|value| value == "path"));
            assert!(required.iter().any(|value| value == "line"));
            assert!(required.iter().any(|value| value == "character"));
        }
    }

    #[test]
    fn operation_names_are_stable() {
        assert_eq!(LspOperation::Definition.tool_name(), "LspDefinition");
        assert_eq!(LspOperation::References.tool_name(), "LspReferences");
        assert_eq!(LspOperation::Hover.tool_name(), "LspHover");
        assert_eq!(
            LspOperation::DocumentSymbols.tool_name(),
            "LspDocumentSymbols"
        );
        assert_eq!(
            LspOperation::WorkspaceSymbols.tool_name(),
            "LspWorkspaceSymbols"
        );
        assert_eq!(LspOperation::Diagnostics.tool_name(), "LspDiagnostics");
    }

    #[test]
    fn workspace_symbols_accept_an_explicit_language_selector() {
        let definition = LspOperation::WorkspaceSymbols.tool_definition();

        assert_eq!(
            definition.parameters["properties"]["language"]["type"],
            "string"
        );
    }
}
