//! Service-backed handlers for durable dynamic-tool lifecycle signal tools.

use y_core::types::ToolCallRequest;

use crate::container::ServiceContainer;
use crate::dynamic_tool_service::{DynamicToolCreateRequest, DynamicToolUpdateRequest};

#[derive(serde::Deserialize)]
struct ToolDeleteParams {
    name: String,
    reason: String,
}

#[derive(serde::Deserialize)]
struct ToolGetParams {
    name: String,
}

#[derive(serde::Deserialize, Default)]
struct ToolListParams {
    query: Option<String>,
}

pub(super) async fn handle(
    container: &ServiceContainer,
    config: &super::AgentExecutionConfig,
    tc: &ToolCallRequest,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    let actor = config.agent_name.as_str();
    let content = match tc.name.as_str() {
        "ToolCreate" => {
            let request: DynamicToolCreateRequest = parse_arguments(tc)?;
            let tool = container
                .dynamic_tool_service
                .create(&container.tool_registry, request, actor)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({"tool": tool, "activated": true})
        }
        "ToolUpdate" => {
            let request: DynamicToolUpdateRequest = parse_arguments(tc)?;
            let tool = container
                .dynamic_tool_service
                .update(&container.tool_registry, request, actor)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({"tool": tool, "activated": true})
        }
        "ToolDelete" => {
            let params: ToolDeleteParams = parse_arguments(tc)?;
            if params.reason.trim().is_empty() {
                return Err(y_core::tool::ToolError::ValidationError {
                    message: "'reason' must not be blank".to_string(),
                });
            }
            let tool = container
                .dynamic_tool_service
                .delete(
                    &container.tool_registry,
                    &params.name,
                    actor,
                    &params.reason,
                )
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({"tool": tool, "deleted": true})
        }
        "ToolGet" => {
            let params: ToolGetParams = parse_arguments(tc)?;
            let tool = container
                .dynamic_tool_service
                .get(&params.name)
                .await
                .ok_or_else(|| y_core::tool::ToolError::NotFound {
                    name: params.name.clone(),
                })?;
            serde_json::json!({"tool": tool})
        }
        "ToolList" => {
            let params: ToolListParams = parse_arguments(tc)?;
            let tools = container
                .dynamic_tool_service
                .list(params.query.as_deref())
                .await;
            let summaries: Vec<_> = tools
                .into_iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "version": tool.version,
                        "created_by": tool.created_by,
                        "created_at": tool.created_at,
                        "parameters": tool.parameters,
                        "kind": match tool.kind {
                            y_tools::DynamicToolKind::Script { interpreter, .. } => {
                                serde_json::json!({"type": "script", "interpreter": interpreter})
                            }
                            y_tools::DynamicToolKind::HttpApi { .. } => {
                                serde_json::json!({"type": "http_api"})
                            }
                            y_tools::DynamicToolKind::Composite { .. } => {
                                serde_json::json!({"type": "composite"})
                            }
                        },
                    })
                })
                .collect();
            serde_json::json!({"count": summaries.len(), "tools": summaries})
        }
        _ => unreachable!("dynamic-tool lifecycle names are matched before dispatch"),
    };

    Ok(y_core::tool::ToolOutput {
        success: true,
        content,
        warnings: vec![],
        metadata: serde_json::json!({"action": tc.name}),
    })
}

fn parse_arguments<T: serde::de::DeserializeOwned>(
    tc: &ToolCallRequest,
) -> Result<T, y_core::tool::ToolError> {
    serde_json::from_value(tc.arguments.clone()).map_err(|error| {
        y_core::tool::ToolError::ValidationError {
            message: format!("invalid {} arguments: {error}", tc.name),
        }
    })
}

fn tool_error(
    name: &str,
    error: &crate::dynamic_tool_service::DynamicToolServiceError,
) -> y_core::tool::ToolError {
    y_core::tool::ToolError::RuntimeError {
        name: name.to_string(),
        message: error.to_string(),
    }
}
