//! Runtime adapter traits and capability model.
//!
//! Design reference: runtime-design.md
//!
//! The runtime layer provides isolated execution environments for tools.
//! Three backends (Docker, Native/bubblewrap, SSH) implement the same trait.
//! Capabilities are declared by tools and enforced by the runtime -- tools
//! never handle their own security.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::SessionId;

// ---------------------------------------------------------------------------
// Capability model
// ---------------------------------------------------------------------------

/// Complete capability requirements for a tool execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeCapability {
    pub network: NetworkCapability,
    pub filesystem: FilesystemCapability,
    pub container: ContainerCapability,
    pub process: ProcessCapability,
}

/// Network access requirements.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NetworkCapability {
    /// No network access (default, safest).
    #[default]
    None,
    /// Internal network only (specified CIDRs).
    Internal { cidrs: Vec<String> },
    /// External access to specific domains.
    External { domains: Vec<String> },
    /// Unrestricted network access.
    Full,
}

/// Filesystem access requirements.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemCapability {
    /// Allowed filesystem mounts.
    #[serde(default)]
    pub mounts: Vec<MountSpec>,
    /// Whether host filesystem access is permitted at all.
    #[serde(default)]
    pub host_access: bool,
}

/// A single filesystem mount specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountSpec {
    /// Path on the host (or virtual path).
    pub host_path: String,
    /// Path inside the execution environment.
    pub container_path: String,
    /// Access mode.
    pub mode: MountMode,
}

/// Filesystem mount access mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    ReadOnly,
    ReadWrite,
    WriteOnly,
}

/// Container-specific requirements.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerCapability {
    /// Allowed container images.
    #[serde(default)]
    pub allowed_images: Vec<String>,
    /// Whether to allow pulling new images.
    #[serde(default)]
    pub allow_pull: bool,
    /// Resource limits.
    #[serde(default)]
    pub resources: ResourceLimits,
}

/// Resource limits for execution environments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Memory limit in bytes.
    pub memory_bytes: Option<u64>,
    /// CPU quota (e.g., 1.0 = 1 CPU core).
    pub cpu_quota: Option<f64>,
    /// Maximum execution time.
    pub timeout: Option<Duration>,
    /// Maximum output size in bytes.
    pub max_output_bytes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_bytes: Some(512 * 1024 * 1024), // 512 MB
            cpu_quota: Some(1.0),
            timeout: Some(Duration::from_secs(300)),
            max_output_bytes: Some(10 * 1024 * 1024), // 10 MB
        }
    }
}

/// Process execution requirements.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessCapability {
    /// Whether shell execution is allowed.
    #[serde(default)]
    pub shell: bool,
    /// Allowed command prefixes (empty = no restriction beyond shell flag).
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Whether background processes are allowed.
    #[serde(default)]
    pub background: bool,
}

// ---------------------------------------------------------------------------
// Execution request / result
// ---------------------------------------------------------------------------

/// A request to execute code or a command in an isolated environment.
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    /// The command or script to execute.
    pub command: String,
    /// Arguments to the command.
    pub args: Vec<String>,
    /// Working directory inside the execution environment.
    pub working_dir: Option<String>,
    /// Environment variables.
    pub env: HashMap<String, String>,
    /// Standard input to pipe to the process.
    pub stdin: Option<Vec<u8>>,
    /// Session that owns a spawned background process.
    pub owner_session_id: Option<SessionId>,
    /// Capability requirements (determines isolation level).
    pub capabilities: RuntimeCapability,
    /// Preferred container image (for Docker runtime).
    pub image: Option<String>,
}

/// Result of an execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
    pub resource_usage: ResourceUsage,
}

impl ExecutionResult {
    /// Whether the execution completed successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Get stdout as a UTF-8 string, lossy.
    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Get stderr as a UTF-8 string, lossy.
    pub fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Resource usage reported after execution.
#[derive(Debug, Clone, Default)]
pub struct ResourceUsage {
    /// Peak memory usage in bytes.
    pub peak_memory_bytes: Option<u64>,
    /// CPU time consumed.
    pub cpu_time: Option<Duration>,
}

/// Handle for a spawned long-running process.
#[derive(Debug, Clone)]
pub struct ProcessHandle {
    /// Unique identifier for the spawned process.
    pub id: String,
    /// Which backend is managing this process.
    pub backend: RuntimeBackend,
}

/// Status of a spawned long-running process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Process is still running.
    Running,
    /// Process completed with an exit code.
    Completed { exit_code: i32 },
    /// Process failed with an error.
    Failed { error: String },
    /// Status cannot be determined.
    Unknown,
}

/// Incremental output snapshot for a runtime-managed background process.
#[derive(Debug, Clone)]
pub struct BackgroundProcessSnapshot {
    /// Handle identifying the managed process.
    pub handle: ProcessHandle,
    /// Current process status after collecting output.
    pub status: ProcessStatus,
    /// Session that owns this process.
    pub owner_session_id: Option<SessionId>,
    /// Stdout bytes captured since the previous snapshot.
    pub stdout: Vec<u8>,
    /// Stderr bytes captured since the previous snapshot.
    pub stderr: Vec<u8>,
    /// Elapsed process lifetime when the snapshot was created.
    pub duration: Duration,
}

/// Summary of an active runtime-managed background process.
#[derive(Debug, Clone)]
pub struct BackgroundProcessInfo {
    /// Handle identifying the managed process.
    pub handle: ProcessHandle,
    /// Original command string or executable display name.
    pub command: String,
    /// Working directory used for the process.
    pub working_dir: Option<String>,
    /// Session that owns this process.
    pub owner_session_id: Option<SessionId>,
    /// Current process status.
    pub status: ProcessStatus,
    /// Elapsed process lifetime when listed.
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// Health status of a runtime backend.
#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    pub backend: RuntimeBackend,
    pub available: bool,
    pub message: Option<String>,
}

/// Runtime backend type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackend {
    Docker,
    Native,
    Ssh,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from runtime operations.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("capability denied: {capability}")]
    CapabilityDenied { capability: String },

    #[error("image not whitelisted: {image}")]
    ImageNotAllowed { image: String },

    #[error("execution timeout after {timeout:?}")]
    Timeout { timeout: Duration },

    #[error("execution failed: exit code {exit_code}")]
    ExecutionFailed { exit_code: i32, stderr: String },

    #[error("resource limit exceeded: {resource}")]
    ResourceExceeded { resource: String },

    #[error("runtime not available: {backend:?}")]
    RuntimeNotAvailable { backend: RuntimeBackend },

    #[error("container error: {message}")]
    ContainerError { message: String },

    #[error("path traversal attempt on: {path}")]
    PathTraversalAttempt { path: String },

    #[error("background process access denied for session {session_id}: {process_id}")]
    BackgroundProcessAccessDenied {
        process_id: String,
        session_id: String,
    },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Adapter for isolated code execution.
///
/// Three implementations exist:
/// - `DockerRuntime`: Container-based isolation (primary for untrusted code)
/// - `NativeRuntime`: bubblewrap sandbox (lightweight, for trusted tools)
/// - `SshRuntime`: Remote execution (for distributed scenarios)
///
/// Tools declare their [`RuntimeCapability`] requirements; the runtime
/// enforces them through a 7-layer security model.
#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    /// Human-readable name identifying this runtime backend.
    fn name(&self) -> &'static str;

    /// Execute a command in an isolated environment.
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError>;

    /// Check if this runtime backend is available and healthy.
    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError>;

    /// Which backend this adapter represents.
    fn backend(&self) -> RuntimeBackend;

    /// Clean up resources (containers, temp files, etc.).
    async fn cleanup(&self) -> Result<(), RuntimeError>;

    /// Spawn a long-running process and return a handle for management.
    async fn spawn(&self, _request: ExecutionRequest) -> Result<ProcessHandle, RuntimeError> {
        Err(RuntimeError::Other {
            message: "spawn not supported by this backend".into(),
        })
    }

    /// Kill a previously spawned process.
    async fn kill(&self, _handle: &ProcessHandle) -> Result<(), RuntimeError> {
        Err(RuntimeError::Other {
            message: "kill not supported by this backend".into(),
        })
    }

    /// Query the status of a previously spawned process.
    async fn status(&self, _handle: &ProcessHandle) -> Result<ProcessStatus, RuntimeError> {
        Err(RuntimeError::Other {
            message: "status not supported by this backend".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// CommandRunner — bridge between Tool and Runtime layers
// ---------------------------------------------------------------------------

/// Minimal trait for executing shell commands.
///
/// This is the bridge between the Tool layer (`y-tools`) and the Runtime
/// layer (`y-runtime`). Tools receive a `dyn CommandRunner`, removing their
/// direct dependency on any specific runtime backend.
///
/// `RuntimeManager` implements this trait, routing commands through the
/// configured backend (Native, Docker, or SSH) based on `RuntimeConfig`.
/// Background process lifecycle methods are scoped to the owner session so
/// one session cannot list, poll, write to, or terminate another session's
/// process.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Execute a shell command string and return the result.
    ///
    /// The implementation decides *where* the command runs (local, container,
    /// remote host) based on the runtime configuration.
    async fn run_command(
        &self,
        command: &str,
        working_dir: Option<&str>,
        timeout: Duration,
    ) -> Result<ExecutionResult, RuntimeError>;

    /// Spawn a shell command as a runtime-managed background process.
    async fn spawn_command(
        &self,
        _owner_session_id: &SessionId,
        _command: &str,
        _working_dir: Option<&str>,
        _timeout: Duration,
    ) -> Result<ProcessHandle, RuntimeError> {
        Err(RuntimeError::Other {
            message: "background command spawn not supported by this runner".into(),
        })
    }

    /// Drain output captured from a background process.
    async fn read_process(
        &self,
        _owner_session_id: &SessionId,
        _process_id: &str,
        _yield_time: Duration,
        _max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        Err(RuntimeError::Other {
            message: "background process polling not supported by this runner".into(),
        })
    }

    /// Write bytes to a background process stdin, then drain new output.
    async fn write_process(
        &self,
        _owner_session_id: &SessionId,
        _process_id: &str,
        _input: &[u8],
        _yield_time: Duration,
        _max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        Err(RuntimeError::Other {
            message: "background process stdin not supported by this runner".into(),
        })
    }

    /// Terminate a background process and drain final output.
    async fn kill_process(
        &self,
        _owner_session_id: &SessionId,
        _process_id: &str,
        _yield_time: Duration,
        _max_output_bytes: usize,
    ) -> Result<BackgroundProcessSnapshot, RuntimeError> {
        Err(RuntimeError::Other {
            message: "background process termination not supported by this runner".into(),
        })
    }

    /// List active background processes known to this runner.
    async fn list_processes(
        &self,
        _owner_session_id: &SessionId,
    ) -> Result<Vec<BackgroundProcessInfo>, RuntimeError> {
        Err(RuntimeError::Other {
            message: "background process listing not supported by this runner".into(),
        })
    }
}
