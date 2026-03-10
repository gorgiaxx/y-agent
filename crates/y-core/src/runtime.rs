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
    /// Execute a command in an isolated environment.
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError>;

    /// Check if this runtime backend is available and healthy.
    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError>;

    /// Which backend this adapter represents.
    fn backend(&self) -> RuntimeBackend;

    /// Clean up resources (containers, temp files, etc.).
    async fn cleanup(&self) -> Result<(), RuntimeError>;
}
