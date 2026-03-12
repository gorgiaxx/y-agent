//! Native runtime: process execution via `tokio::process`.
//!
//! The native runtime executes commands directly on the host. It is the
//! simplest and fastest backend, suitable for trusted tool execution.
//!
//! Security features:
//! - Path traversal protection (canonicalize + allowed-path check)
//! - Optional bubblewrap sandboxing (feature `sandbox_bwrap`)
//! - `AuditTrail` integration for execution logging

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::instrument;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, ProcessHandle, ProcessStatus, ResourceUsage, RuntimeAdapter,
    RuntimeBackend, RuntimeError, RuntimeHealth,
};

use crate::audit::{AuditOutcome, AuditTrail};
use crate::config::RuntimeConfig;

/// Native runtime backend using `tokio::process::Command`.
///
/// Executes commands directly on the host OS with timeout protection,
/// output size limiting, path traversal protection, and optional
/// bubblewrap sandboxing.
pub struct NativeRuntime {
    config: RuntimeConfig,
    audit_trail: Option<Arc<AuditTrail>>,
    /// Spawned long-running processes, keyed by handle ID.
    spawned: Arc<Mutex<HashMap<String, tokio::process::Child>>>,
}

impl NativeRuntime {
    /// Create a new native runtime with the given config.
    pub fn new(config: RuntimeConfig, audit_trail: Option<Arc<AuditTrail>>) -> Self {
        Self {
            config,
            audit_trail,
            spawned: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the effective timeout for a request.
    fn effective_timeout(&self, request: &ExecutionRequest) -> Duration {
        request
            .capabilities
            .container
            .resources
            .timeout
            .unwrap_or(self.config.default_timeout)
    }

    /// Get the effective max output bytes for a request.
    ///
    /// Uses the minimum of the request's cap and the config's cap.
    fn effective_max_output(&self, request: &ExecutionRequest) -> u64 {
        match request.capabilities.container.resources.max_output_bytes {
            Some(req_max) => req_max.min(self.config.default_max_output_bytes),
            None => self.config.default_max_output_bytes,
        }
    }

    /// Truncate output to the maximum allowed size.
    fn truncate_output(output: Vec<u8>, max_bytes: u64) -> Vec<u8> {
        let Ok(max) = usize::try_from(max_bytes) else {
            return output;
        };
        if output.len() > max {
            output[..max].to_vec()
        } else {
            output
        }
    }

    /// Validate that the working directory is within allowed paths.
    ///
    /// If `allowed_paths` is empty, all paths are allowed (backwards-compatible).
    /// Returns `PathTraversalAttempt` if the path is outside allowed paths.
    fn validate_working_dir(&self, dir: &str) -> Result<(), RuntimeError> {
        if self.config.allowed_paths.is_empty() {
            return Ok(());
        }

        let canonical = std::fs::canonicalize(dir).map_err(|e| RuntimeError::Other {
            message: format!("failed to canonicalize working directory '{dir}': {e}"),
        })?;

        for allowed in &self.config.allowed_paths {
            let allowed_canonical =
                std::fs::canonicalize(allowed).unwrap_or_else(|_| Path::new(allowed).to_path_buf());
            if canonical.starts_with(&allowed_canonical) {
                return Ok(());
            }
        }

        Err(RuntimeError::PathTraversalAttempt {
            path: dir.to_string(),
        })
    }

    /// Build a command, optionally wrapping it with bubblewrap.
    fn build_command(&self, request: &ExecutionRequest) -> tokio::process::Command {
        #[cfg(feature = "sandbox_bwrap")]
        {
            if let Some(cmd) = self.try_build_bwrap_command(request) {
                return cmd;
            }
        }

        // Plain execution (no sandboxing).
        let mut cmd = tokio::process::Command::new(&request.command);
        cmd.args(&request.args);

        if let Some(ref dir) = request.working_dir {
            cmd.current_dir(dir);
        }

        for (key, val) in &request.env {
            cmd.env(key, val);
        }

        cmd
    }

    /// Attempt to build a bubblewrap-wrapped command.
    ///
    /// Returns `None` if bwrap is not available, logging a warning.
    #[cfg(feature = "sandbox_bwrap")]
    fn try_build_bwrap_command(
        &self,
        request: &ExecutionRequest,
    ) -> Option<tokio::process::Command> {
        // Check if bwrap is available.
        let bwrap_check = std::process::Command::new("which")
            .arg("bwrap")
            .output()
            .ok()?;

        if !bwrap_check.status.success() {
            tracing::warn!("bubblewrap (bwrap) not found; falling back to plain execution");
            return None;
        }

        let mut cmd = tokio::process::Command::new("bwrap");

        // Mount system directories read-only.
        cmd.args(["--ro-bind", "/usr", "/usr"]);
        cmd.args(["--ro-bind", "/bin", "/bin"]);

        // Mount /lib if it exists.
        if Path::new("/lib").exists() {
            cmd.args(["--ro-bind", "/lib", "/lib"]);
        }
        if Path::new("/lib64").exists() {
            cmd.args(["--ro-bind", "/lib64", "/lib64"]);
        }

        // Mount workspace (working directory) as writable.
        if let Some(ref dir) = request.working_dir {
            cmd.args(["--bind", dir, dir]);
        }

        // Create required special filesystems.
        cmd.args(["--proc", "/proc"]);
        cmd.args(["--dev", "/dev"]);
        cmd.arg("--unshare-pid");

        // Network isolation based on capability.
        if matches!(
            request.capabilities.network,
            y_core::runtime::NetworkCapability::None
        ) {
            cmd.arg("--unshare-net");
        }

        // Set environment variables.
        for (key, val) in &request.env {
            cmd.args(["--setenv", key, val]);
        }

        // The actual command to execute inside the sandbox.
        cmd.arg("--");
        cmd.arg(&request.command);
        cmd.args(&request.args);

        Some(cmd)
    }

    /// Log an audit event if audit trail is configured.
    async fn log_audit(
        &self,
        command: &str,
        outcome: AuditOutcome,
        metadata: Option<serde_json::Value>,
    ) {
        if let Some(ref audit) = self.audit_trail {
            audit
                .log_tool_execution("native-runtime", command, outcome, metadata)
                .await;
        }
    }
}

#[async_trait]
impl RuntimeAdapter for NativeRuntime {
    fn name(&self) -> &'static str {
        "native"
    }

    #[instrument(skip(self, request), fields(command = %request.command, backend = "native"))]
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        // Validate working directory if set.
        if let Some(ref dir) = request.working_dir {
            if let Err(e) = self.validate_working_dir(dir) {
                self.log_audit(
                    &request.command,
                    AuditOutcome::Denied {
                        reason: format!("path traversal: {dir}"),
                    },
                    None,
                )
                .await;
                return Err(e);
            }
        }

        let timeout = self.effective_timeout(&request);
        let max_output = self.effective_max_output(&request);

        let mut cmd = self.build_command(&request);

        // Set up pipes.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if request.stdin.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        let start = std::time::Instant::now();

        let mut child = cmd.spawn().map_err(|e| RuntimeError::Other {
            message: format!("failed to spawn process: {e}"),
        })?;

        // Write stdin if provided.
        if let Some(ref stdin_data) = request.stdin {
            if let Some(mut stdin_handle) = child.stdin.take() {
                let data = stdin_data.clone();
                tokio::spawn(async move {
                    let _ = stdin_handle.write_all(&data).await;
                    let _ = stdin_handle.shutdown().await;
                });
            }
        }

        // Wait with timeout.
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| RuntimeError::Timeout { timeout })?
            .map_err(|e| RuntimeError::Other {
                message: format!("process error: {e}"),
            })?;

        let duration = start.elapsed();

        let stdout = Self::truncate_output(output.stdout, max_output);
        let stderr = Self::truncate_output(output.stderr, max_output);

        let exit_code = output.status.code().unwrap_or(-1);

        let result = ExecutionResult {
            exit_code,
            stdout,
            stderr,
            duration,
            resource_usage: ResourceUsage::default(),
        };

        // Log successful execution to audit trail.
        self.log_audit(
            &request.command,
            if result.success() {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failed {
                    error: format!("exit code {exit_code}"),
                }
            },
            Some(serde_json::json!({
                "duration_ms": duration.as_millis(),
                "exit_code": exit_code,
            })),
        )
        .await;

        Ok(result)
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        Ok(RuntimeHealth {
            backend: RuntimeBackend::Native,
            available: cfg!(unix) || cfg!(windows),
            message: None,
        })
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Native
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        // Kill any remaining spawned processes.
        let mut spawned = self.spawned.lock().await;
        for (id, mut child) in spawned.drain() {
            tracing::debug!(process_id = %id, "cleaning up spawned process");
            let _ = child.kill().await;
        }
        Ok(())
    }

    async fn spawn(&self, request: ExecutionRequest) -> Result<ProcessHandle, RuntimeError> {
        // Validate working directory if set.
        if let Some(ref dir) = request.working_dir {
            self.validate_working_dir(dir)?;
        }

        let mut cmd = self.build_command(&request);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::null());

        let child = cmd.spawn().map_err(|e| RuntimeError::Other {
            message: format!("failed to spawn process: {e}"),
        })?;

        let id = uuid::Uuid::new_v4().to_string();
        let handle = ProcessHandle {
            id: id.clone(),
            backend: RuntimeBackend::Native,
        };

        self.spawned.lock().await.insert(id, child);

        Ok(handle)
    }

    async fn kill(&self, handle: &ProcessHandle) -> Result<(), RuntimeError> {
        let mut spawned = self.spawned.lock().await;
        if let Some(mut child) = spawned.remove(&handle.id) {
            child.kill().await.map_err(|e| RuntimeError::Other {
                message: format!("failed to kill process {}: {e}", handle.id),
            })?;
            Ok(())
        } else {
            Err(RuntimeError::Other {
                message: format!("no spawned process with id {}", handle.id),
            })
        }
    }

    async fn status(&self, handle: &ProcessHandle) -> Result<ProcessStatus, RuntimeError> {
        let mut spawned = self.spawned.lock().await;
        if let Some(child) = spawned.get_mut(&handle.id) {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let exit_code = status.code().unwrap_or(-1);
                    // Process finished — remove from map.
                    spawned.remove(&handle.id);
                    Ok(ProcessStatus::Completed { exit_code })
                }
                Ok(None) => Ok(ProcessStatus::Running),
                Err(e) => Ok(ProcessStatus::Failed {
                    error: format!("{e}"),
                }),
            }
        } else {
            Ok(ProcessStatus::Unknown)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use y_core::runtime::{ContainerCapability, ResourceLimits, RuntimeCapability};

    use super::*;

    fn default_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    fn make_runtime() -> NativeRuntime {
        NativeRuntime::new(default_config(), None)
    }

    fn make_runtime_with_audit() -> (NativeRuntime, Arc<AuditTrail>) {
        let audit = Arc::new(AuditTrail::new());
        let rt = NativeRuntime::new(default_config(), Some(audit.clone()));
        (rt, audit)
    }

    fn default_request(command: &str, args: &[&str]) -> ExecutionRequest {
        ExecutionRequest {
            command: command.into(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: RuntimeCapability::default(),
            image: None,
        }
    }

    fn short_timeout_request(command: &str, args: &[&str]) -> ExecutionRequest {
        ExecutionRequest {
            command: command.into(),
            args: args.iter().map(|s| (*s).to_string()).collect(),
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: RuntimeCapability {
                container: ContainerCapability {
                    resources: ResourceLimits {
                        timeout: Some(Duration::from_millis(100)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            image: None,
        }
    }

    // T-RT-002-01
    #[tokio::test]
    async fn test_native_execute_simple_command() {
        let rt = make_runtime();
        let req = default_request("echo", &["hello"]);
        let result = rt.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_string().contains("hello"));
    }

    // T-RT-002-02
    #[tokio::test]
    async fn test_native_execute_failing_command() {
        let rt = make_runtime();
        let req = default_request("false", &[]);
        let result = rt.execute(req).await.unwrap();
        assert_ne!(result.exit_code, 0);
    }

    // T-RT-002-03
    #[tokio::test]
    async fn test_native_execute_with_env() {
        let rt = make_runtime();
        let mut req = default_request("sh", &["-c", "echo $MY_TEST_VAR"]);
        req.env
            .insert("MY_TEST_VAR".into(), "test_value_123".into());
        let result = rt.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_string().contains("test_value_123"));
    }

    // T-RT-002-04
    #[tokio::test]
    async fn test_native_execute_with_stdin() {
        let rt = make_runtime();
        let mut req = default_request("cat", &[]);
        req.stdin = Some(b"hello from stdin".to_vec());
        let result = rt.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_string().contains("hello from stdin"));
    }

    // T-RT-002-05
    #[tokio::test]
    async fn test_native_execute_timeout() {
        let rt = make_runtime();
        let req = short_timeout_request("sleep", &["10"]);
        let result = rt.execute(req).await;
        assert!(matches!(result, Err(RuntimeError::Timeout { .. })));
    }

    // T-RT-002-06
    #[tokio::test]
    async fn test_native_execute_output_limit() {
        let max_bytes: u64 = 100;
        let config = RuntimeConfig {
            default_max_output_bytes: max_bytes,
            ..Default::default()
        };
        let rt = NativeRuntime::new(config, None);
        // Generate output much larger than the limit.
        let req = default_request(
            "sh",
            &[
                "-c",
                "dd if=/dev/zero bs=1024 count=10 2>/dev/null | tr '\\0' 'A'",
            ],
        );
        let result = rt.execute(req).await.unwrap();
        // Output should be truncated to max_bytes.
        assert!(
            result.stdout.len() <= max_bytes as usize,
            "stdout len {} exceeds max {}",
            result.stdout.len(),
            max_bytes
        );
    }

    // T-RT-002-07
    #[tokio::test]
    async fn test_native_health_check() {
        let rt = make_runtime();
        let health = rt.health_check().await.unwrap();
        assert_eq!(health.backend, RuntimeBackend::Native);
        assert!(health.available);
    }

    // T-RT-002-08
    #[tokio::test]
    async fn test_native_backend_type() {
        let rt = make_runtime();
        assert_eq!(rt.backend(), RuntimeBackend::Native);
    }

    // T-R1-01: spawn returns ProcessHandle for NativeRuntime.
    #[tokio::test]
    async fn test_native_spawn_returns_handle() {
        let rt = make_runtime();
        let req = default_request("sleep", &["10"]);
        let handle = rt.spawn(req).await.unwrap();
        assert_eq!(handle.backend, RuntimeBackend::Native);
        assert!(!handle.id.is_empty());
        // Clean up.
        rt.kill(&handle).await.unwrap();
    }

    // T-R1-02: kill terminates a spawned process.
    #[tokio::test]
    async fn test_native_kill_spawned_process() {
        let rt = make_runtime();
        let req = default_request("sleep", &["60"]);
        let handle = rt.spawn(req).await.unwrap();

        // Kill should succeed.
        rt.kill(&handle).await.unwrap();

        // Second kill should fail (already removed).
        let result = rt.kill(&handle).await;
        assert!(result.is_err());
    }

    // T-R1-03: status reports Running then Completed correctly.
    #[tokio::test]
    async fn test_native_status_lifecycle() {
        let rt = make_runtime();
        // Spawn a short-lived process.
        let req = default_request("echo", &["done"]);
        let handle = rt.spawn(req).await.unwrap();

        // Give it a moment to complete.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = rt.status(&handle).await.unwrap();
        assert_eq!(status, ProcessStatus::Completed { exit_code: 0 });

        // After completion, status returns Unknown (removed from map).
        let status2 = rt.status(&handle).await.unwrap();
        assert_eq!(status2, ProcessStatus::Unknown);
    }

    // T-R1-04: Path traversal with `../` is rejected.
    #[tokio::test]
    async fn test_native_path_traversal_rejected() {
        // Create a real temp directory for allowed_paths.
        let safe_dir = std::env::temp_dir().join("y_runtime_test_safe");
        std::fs::create_dir_all(&safe_dir).unwrap();

        let config = RuntimeConfig {
            allowed_paths: vec![safe_dir.to_string_lossy().to_string()],
            ..Default::default()
        };
        let rt = NativeRuntime::new(config, None);
        let mut req = default_request("echo", &["hi"]);
        // Use ../etc to escape the allowed path.
        req.working_dir = Some(format!("{}/../", safe_dir.to_string_lossy()));

        let result = rt.execute(req).await;
        assert!(
            matches!(result, Err(RuntimeError::PathTraversalAttempt { .. })),
            "expected PathTraversalAttempt, got: {result:?}"
        );

        // Cleanup.
        let _ = std::fs::remove_dir_all(&safe_dir);
    }

    // T-R1-05: Path traversal — working_dir outside allowed_paths is rejected.
    #[tokio::test]
    async fn test_native_path_outside_allowed_rejected() {
        let safe_dir = std::env::temp_dir().join("y_runtime_test_safe2");
        std::fs::create_dir_all(&safe_dir).unwrap();

        let config = RuntimeConfig {
            allowed_paths: vec![safe_dir.to_string_lossy().to_string()],
            ..Default::default()
        };
        let rt = NativeRuntime::new(config, None);
        let mut req = default_request("echo", &["hi"]);
        req.working_dir = Some("/etc".into());

        let result = rt.execute(req).await;
        assert!(matches!(
            result,
            Err(RuntimeError::PathTraversalAttempt { .. })
        ));

        // Cleanup.
        let _ = std::fs::remove_dir_all(&safe_dir);
    }

    // T-R1-06: name() returns "native".
    #[test]
    fn test_native_name() {
        let rt = make_runtime();
        assert_eq!(rt.name(), "native");
    }

    // T-R1-07: AuditTrail records execution event after NativeRuntime execution.
    #[tokio::test]
    async fn test_native_audit_trail_records_execution() {
        let (rt, audit) = make_runtime_with_audit();

        let req = default_request("echo", &["audit_test"]);
        let result = rt.execute(req).await.unwrap();
        assert!(result.success());

        // Verify audit trail has an entry.
        assert_eq!(audit.current_count().await, 1);
        let entries = audit.recent(1).await;
        assert_eq!(entries[0].actor, "native-runtime");
        assert_eq!(entries[0].target, "echo");
        assert_eq!(entries[0].outcome, AuditOutcome::Success);
    }

    // T-R1-07b: AuditTrail records path traversal denial.
    #[tokio::test]
    async fn test_native_audit_trail_records_path_denial() {
        let audit = Arc::new(AuditTrail::new());
        let config = RuntimeConfig {
            allowed_paths: vec!["/tmp/safe".into()],
            ..Default::default()
        };
        let rt = NativeRuntime::new(config, Some(audit.clone()));

        let mut req = default_request("echo", &["hi"]);
        req.working_dir = Some("/etc".into());
        let _ = rt.execute(req).await;

        assert_eq!(audit.current_count().await, 1);
        let entries = audit.recent(1).await;
        assert!(matches!(entries[0].outcome, AuditOutcome::Denied { .. }));
    }

    // T-R1-08: Empty allowed_paths allows all (backwards-compatible).
    #[tokio::test]
    async fn test_native_empty_allowed_paths_allows_all() {
        let rt = make_runtime(); // Default config has empty allowed_paths.
        let mut req = default_request("echo", &["hi"]);
        req.working_dir = Some("/tmp".into());

        let result = rt.execute(req).await.unwrap();
        assert!(result.success());
    }
}
