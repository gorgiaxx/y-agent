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
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, Notify};
use tracing::instrument;

use y_core::runtime::{
    BackgroundProcessInfo, BackgroundProcessSnapshot, ExecutionRequest, ExecutionResult,
    ProcessHandle, ProcessStatus, ResourceUsage, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};
use y_core::types::SessionId;

use crate::audit::{AuditOutcome, AuditTrail};
use crate::config::RuntimeConfig;

const BACKGROUND_OUTPUT_BUFFER_LIMIT: usize = 1024 * 1024;

type SharedOutputBuffer = Arc<Mutex<Vec<u8>>>;

struct ManagedNativeProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: SharedOutputBuffer,
    stderr: SharedOutputBuffer,
    output_notify: Arc<Notify>,
    command: String,
    working_dir: Option<String>,
    owner_session_id: Option<SessionId>,
    started_at: Instant,
    status: ProcessStatus,
}

/// Native runtime backend using `tokio::process::Command`.
///
/// Executes commands directly on the host OS with timeout protection,
/// output size limiting, path traversal protection, and optional
/// bubblewrap sandboxing.
pub struct NativeRuntime {
    config: RuntimeConfig,
    audit_trail: Option<Arc<AuditTrail>>,
    /// Spawned long-running processes, keyed by handle ID.
    spawned: Arc<Mutex<HashMap<String, ManagedNativeProcess>>>,
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
    fn build_command(request: &ExecutionRequest) -> tokio::process::Command {
        #[cfg(feature = "sandbox_bwrap")]
        {
            if let Some(cmd) = Self::try_build_bwrap_command(request) {
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
    fn try_build_bwrap_command(request: &ExecutionRequest) -> Option<tokio::process::Command> {
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

    fn command_display(request: &ExecutionRequest) -> String {
        std::iter::once(request.command.as_str())
            .chain(request.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn append_capped(buffer: &mut Vec<u8>, chunk: &[u8]) {
        if chunk.len() >= BACKGROUND_OUTPUT_BUFFER_LIMIT {
            buffer.clear();
            buffer.extend_from_slice(&chunk[chunk.len() - BACKGROUND_OUTPUT_BUFFER_LIMIT..]);
            return;
        }

        let overflow = buffer
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(BACKGROUND_OUTPUT_BUFFER_LIMIT);
        if overflow > 0 {
            buffer.drain(0..overflow);
        }
        buffer.extend_from_slice(chunk);
    }

    fn drain_limited(buffer: &mut Vec<u8>, max_output_bytes: usize) -> Vec<u8> {
        let drain_len = buffer.len().min(max_output_bytes);
        buffer.drain(0..drain_len).collect()
    }

    fn spawn_output_reader<R>(mut reader: R, buffer: SharedOutputBuffer, notify: Arc<Notify>)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut chunk = [0_u8; 8192];
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => {
                        {
                            let mut guard = buffer.lock().await;
                            Self::append_capped(&mut guard, &chunk[..n]);
                        }
                        notify.notify_waiters();
                    }
                    Err(error) => {
                        tracing::warn!(%error, "background process output reader failed");
                        break;
                    }
                }
            }
            notify.notify_waiters();
        });
    }

    fn refresh_entry_status(entry: &mut ManagedNativeProcess) -> ProcessStatus {
        if !matches!(entry.status, ProcessStatus::Running) {
            return entry.status.clone();
        }

        match entry.child.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                entry.status = ProcessStatus::Completed { exit_code };
            }
            Ok(None) => {}
            Err(error) => {
                entry.status = ProcessStatus::Failed {
                    error: error.to_string(),
                };
            }
        }

        entry.status.clone()
    }

    async fn output_available(entry: &ManagedNativeProcess) -> bool {
        !entry.stdout.lock().await.is_empty() || !entry.stderr.lock().await.is_empty()
    }

    fn ensure_process_owner(
        entry: &ManagedNativeProcess,
        process_id: &str,
        owner_session_id: Option<&SessionId>,
    ) -> Result<(), RuntimeError> {
        let Some(owner_session_id) = owner_session_id else {
            return Ok(());
        };
        if entry.owner_session_id.as_ref() == Some(owner_session_id) {
            return Ok(());
        }
        Err(RuntimeError::BackgroundProcessAccessDenied {
            process_id: process_id.to_string(),
            session_id: owner_session_id.to_string(),
        })
    }

    async fn read_process_inner(
        &self,
        owner_session_id: Option<&SessionId>,
        process_id: &str,
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        let wait_for_output = {
            let mut spawned = self.spawned.lock().await;
            let entry = spawned
                .get_mut(process_id)
                .ok_or_else(|| RuntimeError::Other {
                    message: format!("no spawned process with id {process_id}"),
                })?;
            Self::ensure_process_owner(entry, process_id, owner_session_id)?;
            let status = Self::refresh_entry_status(entry);
            status == ProcessStatus::Running && !Self::output_available(entry).await
        };

        if wait_for_output && yield_time > Duration::ZERO {
            let notify = {
                let spawned = self.spawned.lock().await;
                spawned
                    .get(process_id)
                    .map(|entry| entry.output_notify.clone())
            };
            if let Some(notify) = notify {
                let _ = tokio::time::timeout(yield_time, notify.notified()).await;
            }
        }
        if yield_time > Duration::ZERO {
            tokio::time::sleep(Duration::from_millis(10).min(yield_time)).await;
        }

        let (snapshot, remove_process) = {
            let mut spawned = self.spawned.lock().await;
            let entry = spawned
                .get_mut(process_id)
                .ok_or_else(|| RuntimeError::Other {
                    message: format!("no spawned process with id {process_id}"),
                })?;
            Self::ensure_process_owner(entry, process_id, owner_session_id)?;
            let status = Self::refresh_entry_status(entry);
            let stdout = {
                let mut guard = entry.stdout.lock().await;
                Self::drain_limited(&mut guard, max_output_bytes)
            };
            let stderr = {
                let mut guard = entry.stderr.lock().await;
                Self::drain_limited(&mut guard, max_output_bytes)
            };
            let snapshot = BackgroundProcessSnapshot {
                handle: ProcessHandle {
                    id: process_id.to_string(),
                    backend: RuntimeBackend::Native,
                },
                status: status.clone(),
                owner_session_id: entry.owner_session_id.clone(),
                stdout,
                stderr,
                duration: entry.started_at.elapsed(),
            };
            (snapshot, status != ProcessStatus::Running)
        };

        if remove_process {
            self.spawned.lock().await.remove(process_id);
        }

        Ok(snapshot)
    }

    /// Drain incremental output for a session-owned managed background process.
    pub async fn read_process_for_session(
        &self,
        owner_session_id: &SessionId,
        process_id: &str,
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        self.read_process_inner(
            Some(owner_session_id),
            process_id,
            yield_time,
            max_output_bytes,
        )
        .await
    }

    async fn write_process_inner(
        &self,
        owner_session_id: Option<&SessionId>,
        process_id: &str,
        input: &[u8],
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        {
            let mut spawned = self.spawned.lock().await;
            let entry = spawned
                .get_mut(process_id)
                .ok_or_else(|| RuntimeError::Other {
                    message: format!("no spawned process with id {process_id}"),
                })?;
            Self::ensure_process_owner(entry, process_id, owner_session_id)?;
            if Self::refresh_entry_status(entry) != ProcessStatus::Running {
                return Err(RuntimeError::Other {
                    message: format!("process {process_id} is not running"),
                });
            }
            let stdin = entry.stdin.as_mut().ok_or_else(|| RuntimeError::Other {
                message: format!("stdin is closed for process {process_id}"),
            })?;
            stdin
                .write_all(input)
                .await
                .map_err(|error| RuntimeError::Other {
                    message: format!("failed to write stdin for process {process_id}: {error}"),
                })?;
            stdin.flush().await.map_err(|error| RuntimeError::Other {
                message: format!("failed to flush stdin for process {process_id}: {error}"),
            })?;
        }

        self.read_process_inner(owner_session_id, process_id, yield_time, max_output_bytes)
            .await
    }

    /// Write stdin to a session-owned managed process.
    pub async fn write_process_for_session(
        &self,
        owner_session_id: &SessionId,
        process_id: &str,
        input: &[u8],
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        self.write_process_inner(
            Some(owner_session_id),
            process_id,
            input,
            yield_time,
            max_output_bytes,
        )
        .await
    }

    async fn kill_process_inner(
        &self,
        owner_session_id: Option<&SessionId>,
        process_id: &str,
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        let mut entry = {
            let mut spawned = self.spawned.lock().await;
            let entry = spawned.get(process_id).ok_or_else(|| RuntimeError::Other {
                message: format!("no spawned process with id {process_id}"),
            })?;
            Self::ensure_process_owner(entry, process_id, owner_session_id)?;
            spawned
                .remove(process_id)
                .ok_or_else(|| RuntimeError::Other {
                    message: format!("no spawned process with id {process_id}"),
                })?
        };

        let status = match entry.child.try_wait() {
            Ok(Some(status)) => ProcessStatus::Completed {
                exit_code: status.code().unwrap_or(-1),
            },
            Ok(None) => {
                entry
                    .child
                    .kill()
                    .await
                    .map_err(|error| RuntimeError::Other {
                        message: format!("failed to kill process {process_id}: {error}"),
                    })?;
                ProcessStatus::Completed { exit_code: -1 }
            }
            Err(error) => ProcessStatus::Failed {
                error: error.to_string(),
            },
        };

        if yield_time > Duration::ZERO && !Self::output_available(&entry).await {
            let _ = tokio::time::timeout(yield_time, entry.output_notify.notified()).await;
        }
        if yield_time > Duration::ZERO {
            tokio::time::sleep(Duration::from_millis(10).min(yield_time)).await;
        }

        let stdout = {
            let mut guard = entry.stdout.lock().await;
            Self::drain_limited(&mut guard, max_output_bytes)
        };
        let stderr = {
            let mut guard = entry.stderr.lock().await;
            Self::drain_limited(&mut guard, max_output_bytes)
        };

        Ok(BackgroundProcessSnapshot {
            handle: ProcessHandle {
                id: process_id.to_string(),
                backend: RuntimeBackend::Native,
            },
            status,
            owner_session_id: entry.owner_session_id.clone(),
            stdout,
            stderr,
            duration: entry.started_at.elapsed(),
        })
    }

    /// Terminate a session-owned managed process.
    pub async fn kill_process_for_session(
        &self,
        owner_session_id: &SessionId,
        process_id: &str,
        yield_time: Duration,
        max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        self.kill_process_inner(
            Some(owner_session_id),
            process_id,
            yield_time,
            max_output_bytes,
        )
        .await
    }

    async fn list_processes_inner(
        &self,
        owner_session_id: Option<&SessionId>,
    ) -> Vec<BackgroundProcessInfo> {
        let mut completed = Vec::new();
        let mut results = Vec::new();
        {
            let mut spawned = self.spawned.lock().await;
            for (id, entry) in spawned.iter_mut() {
                if let Some(owner_session_id) = owner_session_id {
                    if entry.owner_session_id.as_ref() != Some(owner_session_id) {
                        continue;
                    }
                }
                let status = Self::refresh_entry_status(entry);
                if status != ProcessStatus::Running {
                    completed.push(id.clone());
                }
                results.push(BackgroundProcessInfo {
                    handle: ProcessHandle {
                        id: id.clone(),
                        backend: RuntimeBackend::Native,
                    },
                    command: entry.command.clone(),
                    working_dir: entry.working_dir.clone(),
                    owner_session_id: entry.owner_session_id.clone(),
                    status,
                    duration: entry.started_at.elapsed(),
                });
            }
            for id in &completed {
                spawned.remove(id);
            }
        }
        results
    }

    /// List managed background processes owned by a session.
    pub async fn list_processes_for_session(
        &self,
        owner_session_id: &SessionId,
    ) -> Vec<BackgroundProcessInfo> {
        self.list_processes_inner(Some(owner_session_id)).await
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

        let mut cmd = Self::build_command(&request);

        // Set up pipes.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if request.stdin.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::inherit());
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
        for (id, mut entry) in spawned.drain() {
            tracing::debug!(process_id = %id, "cleaning up spawned process");
            let _ = entry.child.kill().await;
        }
        Ok(())
    }

    async fn spawn(&self, request: ExecutionRequest) -> Result<ProcessHandle, RuntimeError> {
        // Validate working directory if set.
        if let Some(ref dir) = request.working_dir {
            self.validate_working_dir(dir)?;
        }

        let mut cmd = Self::build_command(&request);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdin(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| RuntimeError::Other {
            message: format!("failed to spawn process: {e}"),
        })?;

        let id = uuid::Uuid::new_v4().to_string();
        let handle = ProcessHandle {
            id: id.clone(),
            backend: RuntimeBackend::Native,
        };

        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let output_notify = Arc::new(Notify::new());

        if let Some(stdout_reader) = child.stdout.take() {
            Self::spawn_output_reader(
                stdout_reader,
                Arc::clone(&stdout),
                Arc::clone(&output_notify),
            );
        }
        if let Some(stderr_reader) = child.stderr.take() {
            Self::spawn_output_reader(
                stderr_reader,
                Arc::clone(&stderr),
                Arc::clone(&output_notify),
            );
        }
        let stdin = child.stdin.take();

        let entry = ManagedNativeProcess {
            child,
            stdin,
            stdout,
            stderr,
            output_notify,
            command: Self::command_display(&request),
            working_dir: request.working_dir.clone(),
            owner_session_id: request.owner_session_id.clone(),
            started_at: Instant::now(),
            status: ProcessStatus::Running,
        };

        self.spawned.lock().await.insert(id, entry);

        Ok(handle)
    }

    async fn kill(&self, handle: &ProcessHandle) -> Result<(), RuntimeError> {
        if let Some(mut entry) = self.spawned.lock().await.remove(&handle.id) {
            entry.child.kill().await.map_err(|e| RuntimeError::Other {
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
        if let Some(entry) = spawned.get_mut(&handle.id) {
            let status = Self::refresh_entry_status(entry);
            if status != ProcessStatus::Running {
                spawned.remove(&handle.id);
            }
            Ok(status)
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
            owner_session_id: None,
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
            owner_session_id: None,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn test_native_spawn_poll_and_kill_managed_process() {
        let rt = make_runtime();
        let owner_session_id = SessionId::from_string("session-a");
        let mut req = default_request("sh", &["-c", "printf ready; sleep 60"]);
        req.owner_session_id = Some(owner_session_id.clone());

        let handle = rt.spawn(req).await.unwrap();
        let snapshot = rt
            .read_process_for_session(
                &owner_session_id,
                &handle.id,
                Duration::from_secs(1),
                10_000,
            )
            .await
            .unwrap();

        assert_eq!(snapshot.handle.id, handle.id);
        assert_eq!(snapshot.status, ProcessStatus::Running);
        assert_eq!(String::from_utf8_lossy(&snapshot.stdout), "ready");

        let killed = rt
            .kill_process_for_session(
                &owner_session_id,
                &handle.id,
                Duration::from_millis(100),
                10_000,
            )
            .await
            .unwrap();

        assert!(matches!(
            killed.status,
            ProcessStatus::Completed { .. } | ProcessStatus::Failed { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_native_background_processes_are_session_scoped() {
        let rt = make_runtime();
        let session_a = SessionId::from_string("session-a");
        let session_b = SessionId::from_string("session-b");
        let mut req_a = default_request("sh", &["-c", "printf a; sleep 60"]);
        let mut req_b = default_request("sh", &["-c", "printf b; sleep 60"]);
        req_a.owner_session_id = Some(session_a.clone());
        req_b.owner_session_id = Some(session_b.clone());

        let handle_a = rt.spawn(req_a).await.unwrap();
        let handle_b = rt.spawn(req_b).await.unwrap();

        let session_a_processes = rt.list_processes_for_session(&session_a).await;
        assert_eq!(
            session_a_processes
                .iter()
                .map(|process| process.handle.id.as_str())
                .collect::<Vec<_>>(),
            vec![handle_a.id.as_str()]
        );

        let denied = rt
            .read_process_for_session(&session_b, &handle_a.id, Duration::ZERO, 10_000)
            .await;
        assert!(matches!(
            denied,
            Err(RuntimeError::BackgroundProcessAccessDenied { .. })
        ));

        rt.kill_process_for_session(&session_a, &handle_a.id, Duration::ZERO, 10_000)
            .await
            .unwrap();
        rt.kill_process_for_session(&session_b, &handle_b.id, Duration::ZERO, 10_000)
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_session_list_does_not_prune_other_session_completed_process() {
        let rt = make_runtime();
        let session_a = SessionId::from_string("session-a");
        let session_b = SessionId::from_string("session-b");
        let mut req_a = default_request("sh", &["-c", "sleep 60"]);
        let mut req_b = default_request("sh", &["-c", "printf b"]);
        req_a.owner_session_id = Some(session_a.clone());
        req_b.owner_session_id = Some(session_b.clone());

        let handle_a = rt.spawn(req_a).await.unwrap();
        let handle_b = rt.spawn(req_b).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        let session_a_processes = rt.list_processes_for_session(&session_a).await;
        assert_eq!(
            session_a_processes
                .iter()
                .map(|process| process.handle.id.as_str())
                .collect::<Vec<_>>(),
            vec![handle_a.id.as_str()]
        );

        let session_b_snapshot = rt
            .read_process_for_session(&session_b, &handle_b.id, Duration::from_secs(1), 10_000)
            .await
            .unwrap();
        assert_eq!(
            session_b_snapshot.status,
            ProcessStatus::Completed { exit_code: 0 }
        );
        assert_eq!(String::from_utf8_lossy(&session_b_snapshot.stdout), "b");

        rt.kill_process_for_session(&session_a, &handle_a.id, Duration::ZERO, 10_000)
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_native_completed_process_is_removed_after_final_poll() {
        let rt = make_runtime();
        let owner_session_id = SessionId::from_string("session-a");
        let mut req = default_request("sh", &["-c", "printf done"]);
        req.owner_session_id = Some(owner_session_id.clone());

        let handle = rt.spawn(req).await.unwrap();
        let snapshot = rt
            .read_process_for_session(
                &owner_session_id,
                &handle.id,
                Duration::from_secs(1),
                10_000,
            )
            .await
            .unwrap();

        assert_eq!(snapshot.status, ProcessStatus::Completed { exit_code: 0 });
        assert_eq!(String::from_utf8_lossy(&snapshot.stdout), "done");

        let status = rt.status(&handle).await.unwrap();
        assert_eq!(status, ProcessStatus::Unknown);
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
