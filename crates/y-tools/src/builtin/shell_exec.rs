//! `ShellExec` built-in tool: execute shell commands.

use async_trait::async_trait;
use std::time::Duration;

use y_core::runtime::{
    BackgroundProcessSnapshot, ProcessCapability, ProcessStatus, RuntimeCapability,
};
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default time to wait for background process output before returning.
const DEFAULT_YIELD_TIME_MS: u64 = 1_000;

/// Maximum output size (bytes) to return to the LLM.
const MAX_OUTPUT_BYTES: usize = 10_000;

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
            name: ToolName::from_string("ShellExec"),
            description: "Execute shell commands and manage runtime-backed background processes."
                .into(),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["run", "start", "poll", "write", "kill", "list"],
                        "description": "Lifecycle action. Defaults to 'run'. Use 'start' for a persistent background process, 'poll' to read incremental output, 'write' to send stdin, 'kill' to terminate, and 'list' to inspect active processes."
                    },
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute for run/start actions"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Working directory for the command (optional, defaults to the active workspace or agent working directory)"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Foreground run timeout in seconds (optional, default: 30)"
                    },
                    "yield_time_ms": {
                        "type": "integer",
                        "description": "For background actions, how long to wait for new output before returning"
                    },
                    "process_id": {
                        "type": "string",
                        "description": "Runtime-managed process id returned by action='start'"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Alias for process_id, accepted for compatibility with terminal session workflows"
                    },
                    "input": {
                        "type": "string",
                        "description": "Bytes/text to write to process stdin for action='write'"
                    },
                    "chars": {
                        "type": "string",
                        "description": "Alias for input for action='write'"
                    }
                },
                "required": []
            }),
            result_schema: None,
            category: ToolCategory::Shell,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability {
                process: ProcessCapability {
                    shell: true,
                    background: true,
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

    fn required_string<'a>(
        arguments: &'a serde_json::Value,
        field: &str,
    ) -> Result<&'a str, ToolError> {
        arguments[field]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::ValidationError {
                message: format!("missing '{field}' parameter"),
            })
    }

    fn process_id(arguments: &serde_json::Value) -> Result<&str, ToolError> {
        arguments
            .get("process_id")
            .or_else(|| arguments.get("session_id"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::ValidationError {
                message: "missing 'process_id' parameter".into(),
            })
    }

    fn yield_time(arguments: &serde_json::Value) -> Duration {
        Duration::from_millis(
            arguments["yield_time_ms"]
                .as_u64()
                .unwrap_or(DEFAULT_YIELD_TIME_MS),
        )
    }

    fn status_label(status: &ProcessStatus) -> &'static str {
        match status {
            ProcessStatus::Running => "running",
            ProcessStatus::Completed { .. } => "completed",
            ProcessStatus::Failed { .. } => "failed",
            ProcessStatus::Unknown => "unknown",
        }
    }

    fn exit_code(status: &ProcessStatus) -> Option<i32> {
        match status {
            ProcessStatus::Completed { exit_code } => Some(*exit_code),
            _ => None,
        }
    }

    fn status_error(status: &ProcessStatus) -> Option<&str> {
        match status {
            ProcessStatus::Failed { error } => Some(error.as_str()),
            _ => None,
        }
    }

    fn output_from_snapshot(snapshot: BackgroundProcessSnapshot) -> ToolOutput {
        let BackgroundProcessSnapshot {
            handle,
            status,
            owner_session_id: _,
            stdout,
            stderr,
            duration,
        } = snapshot;
        let stdout = String::from_utf8_lossy(&stdout).to_string();
        let stderr = String::from_utf8_lossy(&stderr).to_string();
        let success = !matches!(
            status,
            ProcessStatus::Failed { .. } | ProcessStatus::Unknown
        );

        ToolOutput {
            success,
            content: serde_json::json!({
                "process_id": handle.id,
                "backend": handle.backend,
                "status": Self::status_label(&status),
                "exit_code": Self::exit_code(&status),
                "error": Self::status_error(&status),
                "stdout": Self::truncate_output(&stdout),
                "stderr": Self::truncate_output(&stderr),
                "duration_ms": duration.as_millis(),
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
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
        let action = input.arguments["action"].as_str().unwrap_or("run");

        if action == "list" {
            let runner = input
                .command_runner
                .as_ref()
                .ok_or_else(|| ToolError::RuntimeError {
                    name: "ShellExec".into(),
                    message: "background process management requires a runtime command runner"
                        .into(),
                })?;
            let processes = runner
                .list_processes(&input.session_id)
                .await
                .map_err(|e| ToolError::RuntimeError {
                    name: "ShellExec".into(),
                    message: format!("{e}"),
                })?;
            let processes = processes
                .into_iter()
                .map(|process| {
                    serde_json::json!({
                        "process_id": process.handle.id,
                        "backend": process.handle.backend,
                        "command": process.command,
                        "working_dir": process.working_dir,
                        "status": Self::status_label(&process.status),
                        "exit_code": Self::exit_code(&process.status),
                        "error": Self::status_error(&process.status),
                        "duration_ms": process.duration.as_millis(),
                    })
                })
                .collect::<Vec<_>>();
            return Ok(ToolOutput {
                success: true,
                content: serde_json::json!({ "processes": processes }),
                warnings: vec![],
                metadata: serde_json::json!({}),
            });
        }

        let timeout_secs = input.arguments["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let timeout = Duration::from_secs(timeout_secs);
        let working_dir = input
            .arguments
            .get("working_dir")
            .and_then(|value| value.as_str())
            .or(input.working_dir.as_deref());

        if action != "run" {
            let runner = input
                .command_runner
                .as_ref()
                .ok_or_else(|| ToolError::RuntimeError {
                    name: "ShellExec".into(),
                    message: "background process management requires a runtime command runner"
                        .into(),
                })?;
            let yield_time = Self::yield_time(&input.arguments);
            let snapshot = match action {
                "start" => {
                    let command = Self::required_string(&input.arguments, "command")?;
                    tracing::info!(
                        "Starting background shell command: `{command}` (working_dir: {working_dir:?}, timeout_secs: {timeout_secs})"
                    );
                    let handle = runner
                        .spawn_command(&input.session_id, command, working_dir, timeout)
                        .await
                        .map_err(|e| ToolError::RuntimeError {
                            name: "ShellExec".into(),
                            message: format!("{e}"),
                        })?;
                    runner
                        .read_process(&input.session_id, &handle.id, yield_time, MAX_OUTPUT_BYTES)
                        .await
                }
                "poll" => {
                    let process_id = Self::process_id(&input.arguments)?;
                    runner
                        .read_process(&input.session_id, process_id, yield_time, MAX_OUTPUT_BYTES)
                        .await
                }
                "write" => {
                    let process_id = Self::process_id(&input.arguments)?;
                    let chars = input
                        .arguments
                        .get("input")
                        .or_else(|| input.arguments.get("chars"))
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| ToolError::ValidationError {
                            message: "missing 'input' parameter".into(),
                    })?;
                    runner
                        .write_process(
                            &input.session_id,
                            process_id,
                            chars.as_bytes(),
                            yield_time,
                            MAX_OUTPUT_BYTES,
                        )
                        .await
                }
                "kill" => {
                    let process_id = Self::process_id(&input.arguments)?;
                    runner
                        .kill_process(&input.session_id, process_id, yield_time, MAX_OUTPUT_BYTES)
                        .await
                }
                other => {
                    return Err(ToolError::ValidationError {
                        message: format!("unknown ShellExec action '{other}'"),
                    });
                }
            }
            .map_err(|e| ToolError::RuntimeError {
                name: "ShellExec".into(),
                message: format!("{e}"),
            })?;

            return Ok(Self::output_from_snapshot(snapshot));
        }

        let command = Self::required_string(&input.arguments, "command")?;

        tracing::info!(
            "Executing shell command: `{command}` (working_dir: {working_dir:?}, timeout_secs: {timeout_secs})"
        );

        // Prefer the injected CommandRunner (runtime-aware execution).
        // Falls back to direct local execution when no runner is provided
        // (backward compatibility for tests and standalone usage).
        if let Some(ref runner) = input.command_runner {
            let result = runner
                .run_command(command, working_dir, timeout)
                .await
                .map_err(|e| ToolError::RuntimeError {
                    name: "ShellExec".into(),
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
        let (shell, shell_flag): (&str, &str) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg(shell_flag).arg(command);

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
                name: "ShellExec".into(),
                message: format!("failed to execute command: {e}"),
            }),
            Err(_) => Err(ToolError::Timeout { timeout_secs }),
        }
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }

    fn is_destructive(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use y_core::runtime::{
        BackgroundProcessInfo, BackgroundProcessSnapshot, CommandRunner, ExecutionResult,
        ProcessHandle, ProcessStatus, ResourceUsage, RuntimeBackend, RuntimeError,
    };
    use y_core::types::SessionId;

    #[derive(Debug)]
    struct RecordingRunner {
        calls: Mutex<Vec<String>>,
        working_dirs: Mutex<Vec<Option<String>>>,
        owner_session_ids: Mutex<Vec<String>>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                working_dirs: Mutex::new(Vec::new()),
                owner_session_ids: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn working_dirs(&self) -> Vec<Option<String>> {
            self.working_dirs.lock().unwrap().clone()
        }

        fn owner_session_ids(&self) -> Vec<String> {
            self.owner_session_ids.lock().unwrap().clone()
        }

        fn snapshot(
            status: ProcessStatus,
            stdout: &str,
            stderr: &str,
        ) -> BackgroundProcessSnapshot {
            BackgroundProcessSnapshot {
                handle: ProcessHandle {
                    id: "proc-1".into(),
                    backend: RuntimeBackend::Native,
                },
                status,
                owner_session_id: None,
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
                duration: Duration::from_millis(25),
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run_command(
            &self,
            command: &str,
            working_dir: Option<&str>,
            _timeout: Duration,
        ) -> Result<ExecutionResult, RuntimeError> {
            self.calls.lock().unwrap().push(format!("run:{command}"));
            self.working_dirs
                .lock()
                .unwrap()
                .push(working_dir.map(ToOwned::to_owned));
            Ok(ExecutionResult {
                exit_code: 0,
                stdout: b"hello\n".to_vec(),
                stderr: Vec::new(),
                duration: Duration::from_millis(10),
                resource_usage: ResourceUsage::default(),
            })
        }

        async fn spawn_command(
            &self,
            owner_session_id: &SessionId,
            command: &str,
            working_dir: Option<&str>,
            _timeout: Duration,
        ) -> Result<ProcessHandle, RuntimeError> {
            self.owner_session_ids
                .lock()
                .unwrap()
                .push(owner_session_id.to_string());
            self.calls.lock().unwrap().push(format!("spawn:{command}"));
            self.working_dirs
                .lock()
                .unwrap()
                .push(working_dir.map(ToOwned::to_owned));
            Ok(ProcessHandle {
                id: "proc-1".into(),
                backend: RuntimeBackend::Native,
            })
        }

        async fn read_process(
            &self,
            owner_session_id: &SessionId,
            process_id: &str,
            _yield_time: Duration,
            _max_output_bytes: usize,
        ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
            self.owner_session_ids
                .lock()
                .unwrap()
                .push(owner_session_id.to_string());
            self.calls
                .lock()
                .unwrap()
                .push(format!("poll:{process_id}"));
            Ok(Self::snapshot(ProcessStatus::Running, "ready\n", ""))
        }

        async fn write_process(
            &self,
            owner_session_id: &SessionId,
            process_id: &str,
            input: &[u8],
            _yield_time: Duration,
            _max_output_bytes: usize,
        ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
            self.owner_session_ids
                .lock()
                .unwrap()
                .push(owner_session_id.to_string());
            let input = String::from_utf8_lossy(input);
            self.calls
                .lock()
                .unwrap()
                .push(format!("write:{process_id}:{input}"));
            Ok(Self::snapshot(ProcessStatus::Running, "accepted\n", ""))
        }

        async fn kill_process(
            &self,
            owner_session_id: &SessionId,
            process_id: &str,
            _yield_time: Duration,
            _max_output_bytes: usize,
        ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
            self.owner_session_ids
                .lock()
                .unwrap()
                .push(owner_session_id.to_string());
            self.calls
                .lock()
                .unwrap()
                .push(format!("kill:{process_id}"));
            Ok(Self::snapshot(
                ProcessStatus::Completed { exit_code: -1 },
                "stopped\n",
                "",
            ))
        }

        async fn list_processes(
            &self,
            owner_session_id: &SessionId,
        ) -> Result<Vec<BackgroundProcessInfo>, RuntimeError> {
            self.owner_session_ids
                .lock()
                .unwrap()
                .push(owner_session_id.to_string());
            self.calls.lock().unwrap().push("list".into());
            Ok(vec![BackgroundProcessInfo {
                handle: ProcessHandle {
                    id: "proc-1".into(),
                    backend: RuntimeBackend::Native,
                },
                command: "npm run dev".into(),
                working_dir: Some("/workspace".into()),
                owner_session_id: None,
                status: ProcessStatus::Running,
                duration: Duration::from_secs(2),
            }])
        }
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("ShellExec"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: None,
        }
    }

    fn make_input_with_runner(
        args: serde_json::Value,
        runner: Arc<dyn CommandRunner>,
    ) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("ShellExec"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner: Some(runner),
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
        assert_eq!(def.name.as_str(), "ShellExec");
        assert!(def.is_dangerous);
        assert!(def.capabilities.process.shell);
    }

    #[test]
    fn test_truncate_output_short() {
        let s = "short";
        assert_eq!(ShellExecTool::truncate_output(s), "short");
    }

    #[tokio::test]
    async fn test_shell_exec_start_returns_managed_process_id() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let input = make_input_with_runner(
            serde_json::json!({
                "action": "start",
                "command": "npm run dev",
                "yield_time_ms": 250
            }),
            runner.clone(),
        );

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["process_id"], "proc-1");
        assert_eq!(output.content["status"], "running");
        assert!(output.content["stdout"].as_str().unwrap().contains("ready"));
        assert_eq!(runner.calls(), vec!["spawn:npm run dev", "poll:proc-1"]);
    }

    #[tokio::test]
    async fn test_shell_exec_uses_injected_working_dir_when_argument_is_missing() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let mut input = make_input_with_runner(
            serde_json::json!({
                "command": "pwd"
            }),
            runner.clone(),
        );
        input.working_dir = Some("/repo/workspace".into());

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(runner.calls(), vec!["run:pwd"]);
        assert_eq!(runner.working_dirs(), vec![Some("/repo/workspace".into())]);
    }

    #[tokio::test]
    async fn test_shell_exec_poll_accepts_session_id_alias() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let input = make_input_with_runner(
            serde_json::json!({
                "action": "poll",
                "session_id": "proc-1",
                "yield_time_ms": 250
            }),
            runner.clone(),
        );

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["process_id"], "proc-1");
        assert_eq!(output.content["status"], "running");
        assert_eq!(runner.calls(), vec!["poll:proc-1"]);
    }

    #[tokio::test]
    async fn test_shell_exec_write_then_reads_incremental_output() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let input = make_input_with_runner(
            serde_json::json!({
                "action": "write",
                "process_id": "proc-1",
                "input": "rs\n",
                "yield_time_ms": 250
            }),
            runner.clone(),
        );

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert!(output.content["stdout"]
            .as_str()
            .unwrap()
            .contains("accepted"));
        assert_eq!(runner.calls(), vec!["write:proc-1:rs\n"]);
    }

    #[tokio::test]
    async fn test_shell_exec_kill_reports_completed_process() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let input = make_input_with_runner(
            serde_json::json!({
                "action": "kill",
                "process_id": "proc-1"
            }),
            runner.clone(),
        );

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["status"], "completed");
        assert_eq!(output.content["exit_code"], -1);
        assert_eq!(runner.calls(), vec!["kill:proc-1"]);
    }

    #[tokio::test]
    async fn test_shell_exec_list_returns_active_processes() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let input = make_input_with_runner(
            serde_json::json!({
                "action": "list"
            }),
            runner.clone(),
        );

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(output.content["processes"][0]["process_id"], "proc-1");
        assert_eq!(output.content["processes"][0]["status"], "running");
        assert_eq!(runner.calls(), vec!["list"]);
    }

    #[tokio::test]
    async fn test_shell_exec_background_management_uses_tool_session_scope() {
        let tool = ShellExecTool::new();
        let runner = Arc::new(RecordingRunner::new());
        let mut input = make_input_with_runner(
            serde_json::json!({
                "action": "poll",
                "process_id": "proc-1"
            }),
            runner.clone(),
        );
        input.session_id = SessionId::from_string("session-a");

        let output = tool.execute(input).await.unwrap();

        assert!(output.success);
        assert_eq!(runner.owner_session_ids(), vec!["session-a"]);
    }
}
