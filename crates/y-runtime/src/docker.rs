//! Docker runtime: container-based isolation (feature-gated).
//!
//! This module is only compiled when the `runtime_docker` feature is enabled.
//! It uses the `bollard` crate to communicate with the Docker daemon.
//!
//! In the current phase, this is a structural skeleton. The actual Docker
//! API integration will be implemented when Docker testing infrastructure
//! is available.

use std::time::Duration;

use async_trait::async_trait;
use tracing::instrument;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};

use crate::config::RuntimeConfig;

/// Docker runtime backend using the Docker Engine API.
///
/// Provides container-based isolation for untrusted tool execution.
/// Each execution creates a transient container that is removed on completion.
pub struct DockerRuntime {
    config: RuntimeConfig,
}

impl DockerRuntime {
    /// Create a new Docker runtime with the given config.
    pub fn new(config: RuntimeConfig) -> Self {
        Self { config }
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
}

#[async_trait]
impl RuntimeAdapter for DockerRuntime {
    #[instrument(skip(self, request), fields(command = %request.command, backend = "docker"))]
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        let image = request.image.as_deref().ok_or_else(|| RuntimeError::Other {
            message: "Docker runtime requires an image specification".into(),
        })?;

        // Verify image is whitelisted.
        if !self.config.image_whitelist.contains(image) {
            return Err(RuntimeError::ImageNotAllowed {
                image: image.to_string(),
            });
        }

        let _timeout = self.effective_timeout(&request);

        // TODO: Implement actual Docker container lifecycle via bollard:
        // 1. Create container with resource limits, mounts, network config
        // 2. Start container
        // 3. Wait for completion with timeout
        // 4. Capture stdout/stderr
        // 5. Remove container

        Err(RuntimeError::RuntimeNotAvailable {
            backend: RuntimeBackend::Docker,
        })
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        // TODO: Connect to Docker daemon and check health.
        // For now, report unavailable.
        Ok(RuntimeHealth {
            backend: RuntimeBackend::Docker,
            available: false,
            message: Some("Docker runtime not yet implemented".into()),
        })
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Docker
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        // TODO: List and remove orphaned containers with y-agent label.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use y_core::runtime::RuntimeCapability;

    use super::*;

    fn make_request(image: Option<&str>) -> ExecutionRequest {
        ExecutionRequest {
            command: "echo".into(),
            args: vec!["hello".into()],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: RuntimeCapability::default(),
            image: image.map(|s| s.to_string()),
        }
    }

    // T-RT-003-08: Image pull denied when allow_pull is false and image missing.
    #[tokio::test]
    async fn test_docker_image_not_whitelisted() {
        let config = RuntimeConfig {
            image_whitelist: HashSet::from(["python:3.11".into()]),
            ..Default::default()
        };
        let rt = DockerRuntime::new(config);
        let req = make_request(Some("evil:latest"));
        let result = rt.execute(req).await;
        assert!(matches!(result, Err(RuntimeError::ImageNotAllowed { .. })));
    }

    // T-RT-003-09
    #[tokio::test]
    async fn test_docker_health_check() {
        let rt = DockerRuntime::new(RuntimeConfig::default());
        let health = rt.health_check().await.unwrap();
        assert_eq!(health.backend, RuntimeBackend::Docker);
        // Until Docker integration is complete, this reports unavailable.
        assert!(!health.available);
    }

    #[tokio::test]
    async fn test_docker_requires_image() {
        let rt = DockerRuntime::new(RuntimeConfig::default());
        let req = make_request(None);
        let result = rt.execute(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_docker_backend_type() {
        let rt = DockerRuntime::new(RuntimeConfig::default());
        assert_eq!(rt.backend(), RuntimeBackend::Docker);
    }
}
