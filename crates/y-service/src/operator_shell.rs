//! Guardrail-aware execution for operator-entered shell composer commands.

use std::sync::Arc;

use y_core::permission_types::{PermissionBehavior, PermissionResult};
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::types::{SessionId, ToolCallRequest, ToolName};

use crate::agent_service::tool_dispatch::{
    evaluate_operator_tool_permission, permission_reason_text,
};
use crate::ServiceContainer;

/// Permission result shown by presentation layers before direct execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorShellDecision {
    Allow,
    Confirm { reason: String },
    Deny { reason: String },
}

/// Output from a guardrail-approved operator shell command.
#[derive(Debug, Clone, PartialEq)]
pub struct OperatorShellOutput {
    pub success: bool,
    pub content: serde_json::Value,
    pub warnings: Vec<String>,
    pub metadata: serde_json::Value,
}

/// Inputs for one direct shell execution.
pub struct OperatorShellRequest<'a> {
    pub session_id: &'a SessionId,
    pub command: &'a str,
    pub working_dir: Option<&'a str>,
    pub additional_read_dirs: &'a [String],
    pub confirmed: bool,
    pub cancellation: Option<&'a tokio_util::sync::CancellationToken>,
}

#[derive(Debug, thiserror::Error)]
pub enum OperatorShellError {
    #[error("shell command requires confirmation: {reason}")]
    RequiresConfirmation { reason: String },
    #[error("shell command denied: {reason}")]
    Denied { reason: String },
    #[error("ShellExec is not registered")]
    ToolUnavailable,
    #[error("shell execution failed: {0}")]
    Execution(#[from] y_core::tool::ToolError),
}

pub struct OperatorShellService;

impl OperatorShellService {
    /// Evaluate the same tool and guardrail pipeline used by agent tool calls.
    pub async fn preflight(
        container: &ServiceContainer,
        session_id: &SessionId,
        command: &str,
        working_dir: Option<&str>,
        additional_read_dirs: &[String],
    ) -> OperatorShellDecision {
        let tool_call = shell_tool_call(command);
        let result = evaluate_operator_tool_permission(
            container,
            &tool_call,
            session_id,
            working_dir,
            additional_read_dirs,
        )
        .await;
        decision_from_permission(&result)
    }

    /// Execute a command after re-evaluating policy to avoid stale approvals.
    /// A confirmation can satisfy `Ask`, but can never override `Deny`.
    pub async fn execute(
        container: &ServiceContainer,
        request: OperatorShellRequest<'_>,
    ) -> Result<OperatorShellOutput, OperatorShellError> {
        let mut tool_call = shell_tool_call(request.command);
        let permission = evaluate_operator_tool_permission(
            container,
            &tool_call,
            request.session_id,
            request.working_dir,
            request.additional_read_dirs,
        )
        .await;
        match decision_from_permission(&permission) {
            OperatorShellDecision::Confirm { reason } if !request.confirmed => {
                return Err(OperatorShellError::RequiresConfirmation { reason });
            }
            OperatorShellDecision::Allow | OperatorShellDecision::Confirm { .. } => {}
            OperatorShellDecision::Deny { reason } => {
                return Err(OperatorShellError::Denied { reason });
            }
        }
        if let Some(arguments) = permission.updated_input {
            tool_call.arguments = arguments;
        }

        let tool_name = ToolName::from_string("ShellExec");
        let tool = container
            .tool_registry
            .get_tool(&tool_name)
            .await
            .ok_or(OperatorShellError::ToolUnavailable)?;
        let execute = tool.execute(ToolInput {
            call_id: tool_call.id,
            name: tool_name,
            arguments: tool_call.arguments,
            session_id: request.session_id.clone(),
            working_dir: request.working_dir.map(ToOwned::to_owned),
            additional_read_dirs: request.additional_read_dirs.to_vec(),
            command_runner: Some(Arc::clone(&container.runtime_manager) as Arc<dyn CommandRunner>),
        });
        let output = if let Some(cancellation) = request.cancellation {
            tokio::select! {
                output = execute => output?,
                () = cancellation.cancelled() => {
                    return Err(OperatorShellError::Execution(y_core::tool::ToolError::Cancelled));
                }
            }
        } else {
            execute.await?
        };
        Ok(OperatorShellOutput {
            success: output.success,
            content: output.content,
            warnings: output.warnings,
            metadata: output.metadata,
        })
    }
}

fn shell_tool_call(command: &str) -> ToolCallRequest {
    ToolCallRequest {
        id: uuid::Uuid::new_v4().to_string(),
        name: "ShellExec".into(),
        arguments: serde_json::json!({
            "action": "run",
            "command": command,
        }),
    }
}

fn decision_from_permission(result: &PermissionResult) -> OperatorShellDecision {
    let reason = permission_reason_text(&result.reason);
    match result.behavior {
        PermissionBehavior::Allow | PermissionBehavior::Notify => OperatorShellDecision::Allow,
        PermissionBehavior::Ask | PermissionBehavior::Passthrough => {
            OperatorShellDecision::Confirm { reason }
        }
        PermissionBehavior::Deny => OperatorShellDecision::Deny { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::permission_types::PermissionReason;

    fn permission(behavior: PermissionBehavior) -> PermissionResult {
        PermissionResult {
            behavior,
            reason: PermissionReason::GlobalDefault,
            message: None,
            updated_input: None,
        }
    }

    #[test]
    fn asks_require_an_explicit_confirmation() {
        assert_eq!(
            decision_from_permission(&permission(PermissionBehavior::Ask)),
            OperatorShellDecision::Confirm {
                reason: "global default policy".into()
            }
        );
    }

    #[test]
    fn denial_is_never_downgraded_to_confirmation() {
        assert_eq!(
            decision_from_permission(&permission(PermissionBehavior::Deny)),
            OperatorShellDecision::Deny {
                reason: "global default policy".into()
            }
        );
    }

    #[test]
    fn shell_tool_call_uses_foreground_run_action() {
        let call = shell_tool_call("cargo check");
        assert_eq!(call.name, "ShellExec");
        assert_eq!(call.arguments["action"], "run");
        assert_eq!(call.arguments["command"], "cargo check");
    }
}
