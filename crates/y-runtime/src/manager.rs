//! Runtime manager: selects the appropriate backend based on request capabilities.

use async_trait::async_trait;
use tracing::instrument;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};

use crate::capability::CapabilityChecker;
use crate::config::RuntimeConfig;
use crate::docker::DockerRuntime;
use crate::native::NativeRuntime;

/// Manages multiple runtime backends and dispatches execution requests.
///
/// The manager:
/// 1. Validates capabilities against the security policy.
/// 2. Selects the appropriate backend based on the request.
/// 3. Dispatches execution to the selected backend.
/// 4. Falls back to alternative backends when the primary is unavailable.
pub struct RuntimeManager {
    config: RuntimeConfig,
    native: NativeRuntime,
    docker: DockerRuntime,
}

impl RuntimeManager {
    /// Create a runtime manager with the given configuration.
    pub fn new(config: RuntimeConfig) -> Self {
        let native = NativeRuntime::new(config.clone());
        let docker = DockerRuntime::new(config.clone());
        Self {
            config,
            native,
            docker,
        }
    }

    /// Select the appropriate backend for the given request.
    ///
    /// Decision logic:
    /// - If the request specifies a container image → Docker
    /// - If the request needs container capabilities → Docker
    /// - Otherwise → Native (or default backend from config)
    fn select_backend(&self, request: &ExecutionRequest) -> RuntimeBackend {
        // Explicit image means Docker.
        if request.image.is_some() {
            return RuntimeBackend::Docker;
        }

        // Container requirements mean Docker.
        if !request.capabilities.container.allowed_images.is_empty() {
            return RuntimeBackend::Docker;
        }

        // Fall back to configured default.
        self.config.default_backend.clone()
    }

    /// Get the backend adapter for the given backend type.
    fn get_adapter(&self, backend: &RuntimeBackend) -> &dyn RuntimeAdapter {
        match backend {
            RuntimeBackend::Docker => &self.docker,
            // SSH not implemented yet; falls back to native.
            RuntimeBackend::Native | RuntimeBackend::Ssh => &self.native,
        }
    }
}

#[async_trait]
impl RuntimeAdapter for RuntimeManager {
    #[instrument(skip(self, request), fields(command = %request.command))]
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        // Step 1: Validate capabilities against policy.
        let checker = CapabilityChecker::new(&self.config);
        let _capped_caps = checker.validate(&request).map_err(|e| -> RuntimeError { e.into() })?;

        // Step 2: Select backend.
        let backend = self.select_backend(&request);
        tracing::info!(?backend, "selected runtime backend");

        // Step 3: Check backend health.
        let adapter = self.get_adapter(&backend);
        let health = adapter.health_check().await?;

        if !health.available {
            // Try fallback to native if Docker is unavailable.
            if backend == RuntimeBackend::Docker {
                tracing::warn!("Docker unavailable, falling back to Native runtime");
                let native_health = self.native.health_check().await?;
                if native_health.available {
                    return self.native.execute(request).await;
                }
            }

            return Err(RuntimeError::RuntimeNotAvailable { backend });
        }

        // Step 4: Execute.
        adapter.execute(request).await
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        // Report overall health: available if any backend is available.
        let native_health = self.native.health_check().await?;
        if native_health.available {
            return Ok(RuntimeHealth {
                backend: RuntimeBackend::Native,
                available: true,
                message: Some("Native runtime available".into()),
            });
        }

        let docker_health = self.docker.health_check().await?;
        if docker_health.available {
            return Ok(docker_health);
        }

        Ok(RuntimeHealth {
            backend: self.config.default_backend.clone(),
            available: false,
            message: Some("No runtime backends available".into()),
        })
    }

    fn backend(&self) -> RuntimeBackend {
        self.config.default_backend.clone()
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        self.native.cleanup().await?;
        self.docker.cleanup().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use y_core::runtime::{ProcessCapability, RuntimeCapability};

    use super::*;

    fn make_request(image: Option<&str>, caps: RuntimeCapability) -> ExecutionRequest {
        ExecutionRequest {
            command: "echo".into(),
            args: vec!["hello".into()],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            capabilities: caps,
            image: image.map(|s| s.to_string()),
        }
    }

    // T-RT-004-01
    #[test]
    fn test_manager_selects_docker_for_container_caps() {
        let config = RuntimeConfig::default();
        let mgr = RuntimeManager::new(config);
        let req = make_request(Some("python:3.11"), RuntimeCapability::default());
        let backend = mgr.select_backend(&req);
        assert_eq!(backend, RuntimeBackend::Docker);
    }

    // T-RT-004-02
    #[test]
    fn test_manager_selects_native_for_simple_commands() {
        let config = RuntimeConfig {
            default_backend: RuntimeBackend::Native,
            ..Default::default()
        };
        let mgr = RuntimeManager::new(config);
        let req = make_request(None, RuntimeCapability::default());
        let backend = mgr.select_backend(&req);
        assert_eq!(backend, RuntimeBackend::Native);
    }

    // T-RT-004-03
    #[tokio::test]
    async fn test_manager_fallback_when_docker_unavailable() {
        let config = RuntimeConfig {
            default_backend: RuntimeBackend::Native,
            image_whitelist: HashSet::from(["python:3.11".into()]),
            ..Default::default()
        };
        let mgr = RuntimeManager::new(config);
        // Docker is unavailable (skeleton), so simple commands should still
        // work via the native backend.
        let req = make_request(None, RuntimeCapability::default());
        let result = mgr.execute(req).await;
        assert!(result.is_ok());
    }

    // T-RT-004-05
    #[tokio::test]
    async fn test_manager_validates_capabilities_before_dispatch() {
        let config = RuntimeConfig {
            allow_shell: false,
            ..Default::default()
        };
        let mgr = RuntimeManager::new(config);
        let req = make_request(
            None,
            RuntimeCapability {
                process: ProcessCapability {
                    shell: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let result = mgr.execute(req).await;
        assert!(matches!(result, Err(RuntimeError::CapabilityDenied { .. })));
    }

    #[tokio::test]
    async fn test_manager_health_check() {
        let config = RuntimeConfig::default();
        let mgr = RuntimeManager::new(config);
        let health = mgr.health_check().await.unwrap();
        // Native should always be available.
        assert!(health.available);
    }
}
