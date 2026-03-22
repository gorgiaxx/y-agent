//! `shell_exec` built-in tool: execute shell commands.

use async_trait::async_trait;
use std::time::Duration;

use y_core::runtime::{ProcessCapability, RuntimeCapability};
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum output size (bytes) to return to the LLM.
const MAX_OUTPUT_BYTES: usize = 100_000;

/// Built-in tool for executing shell commands.
pub struct ShellExecTool {
    def: ToolDefinition,
}

impl ShellExecTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("shell_exec"),
            description: "Execute a shell command and return stdout/stderr.".into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Working directory for the command (optional, defaults to current directory)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (optional, default: 30)"
                    }
                },
                "required": ["command"]
            }),
            result_schema: None,
            category: ToolCategory::Shell,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability {
                process: ProcessCapability {
                    shell: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            is_dangerous: true,
        }
    }

    fn truncate_output(s: &str) -> String {
        if s.len() > MAX_OUTPUT_BYTES {
            let truncated = &s[..MAX_OUTPUT_BYTES];
            format!(
                "{}...\n\n[output truncated: {} bytes total, showing first {}]",
                truncated,
                s.len(),
                MAX_OUTPUT_BYTES
            )
        } else {
            s.to_string()
        }
    }
}

impl Default for ShellExecTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ShellExecTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let command =
            input.arguments["command"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing 'command' parameter".into(),
                })?;

        let timeout_secs = input.arguments["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let timeout = Duration::from_secs(timeout_secs);
        let working_dir = input.arguments["working_dir"].as_str();

        // Prefer the injected CommandRunner (runtime-aware execution).
        // Falls back to direct local execution when no runner is provided
        // (backward compatibility for tests and standalone usage).
        if let Some(ref runner) = input.command_runner {
            let result = runner
                .run_command(command, working_dir, timeout)
                .await
                .map_err(|e| ToolError::RuntimeError {
                    name: "shell_exec".into(),
                    message: format!("{e}"),
                })?;

            let stdout = String::from_utf8_lossy(&result.stdout).to_string();
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();

            return Ok(ToolOutput {
                success: result.exit_code == 0,
                content: serde_json::json!({
                    "exit_code": result.exit_code,
                    "stdout": Self::truncate_output(&stdout),
                    "stderr": Self::truncate_output(&stderr),
                }),
                warnings: vec![],
                metadata: serde_json::json!({}),
            });
        }

        // Fallback: direct local execution (no runtime manager).
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);

        if let Some(wd) = working_dir {
            cmd.current_dir(wd);
        }

        // Capture output.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let result = tokio::time::timeout(timeout, cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                Ok(ToolOutput {
                    success: exit_code == 0,
                    content: serde_json::json!({
                        "exit_code": exit_code,
                        "stdout": Self::truncate_output(&stdout),
                        "stderr": Self::truncate_output(&stderr),
                    }),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
            Ok(Err(e)) => Err(ToolError::RuntimeError {
                name: "shell_exec".into(),
                message: format!("failed to execute command: {e}"),
            }),
            Err(_) => Err(ToolError::Timeout { timeout_secs }),
        }
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::SessionId;

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("shell_exec"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    #[tokio::test]
    async fn test_shell_exec_success() {
        let tool = ShellExecTool::new();
        let input = make_input(serde_json::json!({
            "command": "echo hello"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["exit_code"], 0);
        assert!(output.content["stdout"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn test_shell_exec_nonzero_exit() {
        let tool = ShellExecTool::new();
        let input = make_input(serde_json::json!({
            "command": "exit 42"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(!output.success);
        assert_eq!(output.content["exit_code"], 42);
    }

    #[tokio::test]
    async fn test_shell_exec_timeout() {
        let tool = ShellExecTool::new();
        let input = make_input(serde_json::json!({
            "command": "sleep 60",
            "timeout_secs": 1
        }));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Timeout { .. }));
    }

    #[tokio::test]
    async fn test_shell_exec_with_working_dir() {
        let tool = ShellExecTool::new();
        let input = make_input(serde_json::json!({
            "command": "pwd",
            "working_dir": "/tmp"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        // On macOS /tmp is a symlink to /private/tmp.
        let stdout = output.content["stdout"].as_str().unwrap();
        assert!(stdout.contains("tmp"));
    }

    #[tokio::test]
    async fn test_shell_exec_missing_command() {
        let tool = ShellExecTool::new();
        let input = make_input(serde_json::json!({}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_exec_definition() {
        let def = ShellExecTool::tool_definition();
        assert_eq!(def.name.as_str(), "shell_exec");
        assert!(def.is_dangerous);
        assert!(def.capabilities.process.shell);
    }

    #[test]
    fn test_truncate_output_short() {
        let s = "short";
        assert_eq!(ShellExecTool::truncate_output(s), "short");
    }
}
