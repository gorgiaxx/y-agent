//! Runtime module errors.

use y_core::runtime::{RuntimeBackend, RuntimeError};

/// Errors specific to the runtime module implementation.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeModuleError {
    #[error("capability denied: {capability}")]
    CapabilityDenied { capability: String },

    #[error("image not whitelisted: {image}")]
    ImageNotAllowed { image: String },

    #[error("execution timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("execution failed: exit code {exit_code}")]
    ExecutionFailed { exit_code: i32, stderr: String },

    #[error("resource limit exceeded: {resource}")]
    ResourceExceeded { resource: String },

    #[error("runtime not available: {backend:?}")]
    RuntimeNotAvailable { backend: RuntimeBackend },

    #[error("container error: {message}")]
    ContainerError { message: String },

    #[error("configuration error: {message}")]
    ConfigError { message: String },

    #[error("{message}")]
    Other { message: String },
}

impl From<RuntimeModuleError> for RuntimeError {
    fn from(e: RuntimeModuleError) -> Self {
        match e {
            RuntimeModuleError::CapabilityDenied { capability } => {
                RuntimeError::CapabilityDenied { capability }
            }
            RuntimeModuleError::ImageNotAllowed { image } => {
                RuntimeError::ImageNotAllowed { image }
            }
            RuntimeModuleError::Timeout { timeout_ms } => RuntimeError::Timeout {
                timeout: std::time::Duration::from_millis(timeout_ms),
            },
            RuntimeModuleError::ExecutionFailed { exit_code, stderr } => {
                RuntimeError::ExecutionFailed { exit_code, stderr }
            }
            RuntimeModuleError::ResourceExceeded { resource } => {
                RuntimeError::ResourceExceeded { resource }
            }
            RuntimeModuleError::RuntimeNotAvailable { backend } => {
                RuntimeError::RuntimeNotAvailable { backend }
            }
            RuntimeModuleError::ContainerError { message } => {
                RuntimeError::ContainerError { message }
            }
            RuntimeModuleError::ConfigError { message } | RuntimeModuleError::Other { message } => {
                RuntimeError::Other { message }
            }
        }
    }
}
