//! SSH runtime: remote execution (deferred skeleton).
//!
//! This module provides a placeholder SSH runtime that returns
//! `RuntimeNotAvailable` for all operations. Full implementation
//! is deferred to Phase 5.

use async_trait::async_trait;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};

/// SSH runtime backend (placeholder — deferred to Phase 5).
pub struct SshRuntime;

impl SshRuntime {
    /// Create a new SSH runtime.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SshRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeAdapter for SshRuntime {
    async fn execute(&self, _request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        Err(RuntimeError::RuntimeNotAvailable {
            backend: RuntimeBackend::Ssh,
        })
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        Ok(RuntimeHealth {
            backend: RuntimeBackend::Ssh,
            available: false,
            message: Some("SSH runtime not yet implemented".into()),
        })
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Ssh
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use y_core::runtime::RuntimeCapability;

    #[tokio::test]
    async fn test_ssh_runtime_not_available() {
        let rt = SshRuntime::new();
        let req = y_core::runtime::ExecutionRequest {
            command: "echo".into(),
            args: vec![],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: RuntimeCapability::default(),
            image: None,
        };
        let result = rt.execute(req).await;
        assert!(matches!(
            result,
            Err(RuntimeError::RuntimeNotAvailable { .. })
        ));
    }

    #[tokio::test]
    async fn test_ssh_health_check() {
        let rt = SshRuntime::new();
        let health = rt.health_check().await.unwrap();
        assert_eq!(health.backend, RuntimeBackend::Ssh);
        assert!(!health.available);
    }
}
