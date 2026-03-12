//! Docker runtime: container-based isolation via the Docker Engine API.
//!
//! When the `runtime_docker` feature is enabled, this module uses the `bollard`
//! crate to communicate with the Docker daemon, providing full container
//! lifecycle management with security hardening.
//!
//! Without the feature flag, it remains a skeleton returning `RuntimeNotAvailable`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tracing::instrument;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, ProcessHandle, ProcessStatus, RuntimeAdapter,
    RuntimeBackend, RuntimeError, RuntimeHealth,
};

use crate::audit::AuditTrail;
use crate::config::RuntimeConfig;

// ---------------------------------------------------------------------------
// Feature-gated Docker internals
// ---------------------------------------------------------------------------

#[cfg(feature = "runtime_docker")]
mod docker_impl {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use bollard::container::{
        Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, StopContainerOptions, WaitContainerOptions,
    };
    use bollard::image::CreateImageOptions;
    use bollard::models::HostConfig;
    use bollard::Docker;
    use futures_util::StreamExt;
    use tokio::sync::Mutex;

    use y_core::runtime::{
        ExecutionRequest, ExecutionResult, NetworkCapability, ProcessHandle, ProcessStatus,
        ResourceUsage, RuntimeBackend, RuntimeError, RuntimeHealth,
    };

    use crate::audit::{AuditOutcome, AuditTrail};
    use crate::config::RuntimeConfig;

    /// Internal Docker state holding the client and spawned container tracking.
    pub struct DockerInner {
        pub client: Docker,
        pub config: RuntimeConfig,
        pub audit_trail: Option<Arc<AuditTrail>>,
        /// Spawned (long-running) containers keyed by ProcessHandle ID.
        pub spawned: Mutex<HashMap<String, String>>, // handle_id -> container_id
    }

    impl DockerInner {
        pub fn new(
            client: Docker,
            config: RuntimeConfig,
            audit_trail: Option<Arc<AuditTrail>>,
        ) -> Self {
            Self {
                client,
                config,
                audit_trail,
                spawned: Mutex::new(HashMap::new()),
            }
        }

        /// Check if Docker daemon is reachable.
        pub async fn ping(&self) -> Result<RuntimeHealth, RuntimeError> {
            match self.client.ping().await {
                Ok(_) => Ok(RuntimeHealth {
                    backend: RuntimeBackend::Docker,
                    available: true,
                    message: Some("Docker daemon reachable".into()),
                }),
                Err(e) => Ok(RuntimeHealth {
                    backend: RuntimeBackend::Docker,
                    available: false,
                    message: Some(format!("Docker daemon unreachable: {e}")),
                }),
            }
        }

        /// Build the HostConfig with security hardening.
        fn build_host_config(&self, request: &ExecutionRequest) -> HostConfig {
            let memory_limit = request
                .capabilities
                .container
                .resources
                .memory_bytes
                .map(|b| b as i64);

            let cpu_quota = request
                .capabilities
                .container
                .resources
                .cpu_quota
                .map(|q| (q * 100_000.0) as i64); // Docker uses microseconds per 100ms

            // Network mode: none by default, bridge when network access requested.
            let network_mode = match &request.capabilities.network {
                NetworkCapability::None => Some("none".to_string()),
                _ => Some("bridge".to_string()),
            };

            // Build bind mounts from filesystem capabilities.
            let binds: Vec<String> = request
                .capabilities
                .filesystem
                .mounts
                .iter()
                .map(|m| {
                    let mode = match m.mode {
                        y_core::runtime::MountMode::ReadOnly => "ro",
                        y_core::runtime::MountMode::ReadWrite => "rw",
                        y_core::runtime::MountMode::WriteOnly => "rw",
                    };
                    format!("{}:{}:{}", m.host_path, m.container_path, mode)
                })
                .collect();

            // Drop all Linux capabilities by default.
            let cap_drop = Some(vec!["ALL".to_string()]);

            // Security opts: no-new-privileges.
            let security_opt = Some(vec!["no-new-privileges:true".to_string()]);

            HostConfig {
                memory: memory_limit,
                cpu_quota,
                cpu_period: cpu_quota.map(|_| 100_000), // 100ms period
                network_mode,
                binds: if binds.is_empty() { None } else { Some(binds) },
                readonly_rootfs: Some(true),
                cap_drop,
                security_opt,
                auto_remove: Some(false), // We manage removal ourselves.
                ..Default::default()
            }
        }

        /// Execute a command inside a new container.
        pub async fn execute(
            &self,
            request: &ExecutionRequest,
        ) -> Result<ExecutionResult, RuntimeError> {
            let image = request
                .image
                .as_deref()
                .ok_or_else(|| RuntimeError::Other {
                    message: "Docker runtime requires an image specification".into(),
                })?;

            // Verify image is whitelisted.
            if !self.config.image_whitelist.contains(image) {
                self.log_audit(
                    &request.command,
                    AuditOutcome::Denied {
                        reason: format!("image not whitelisted: {image}"),
                    },
                    None,
                )
                .await;
                return Err(RuntimeError::ImageNotAllowed {
                    image: image.to_string(),
                });
            }

            // Pull image if needed and allowed.
            self.ensure_image(image).await?;

            let timeout = self.effective_timeout(request);
            let host_config = self.build_host_config(request);

            // Build command: [command, args...]
            let mut cmd = vec![request.command.clone()];
            cmd.extend(request.args.clone());

            // Environment variables.
            let env: Vec<String> = request
                .env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();

            // Container labels for management.
            let mut labels = std::collections::HashMap::new();
            labels.insert("y-agent.managed".to_string(), "true".to_string());

            let config = Config {
                image: Some(image.to_string()),
                cmd: Some(cmd),
                env: if env.is_empty() { None } else { Some(env) },
                working_dir: request.working_dir.clone(),
                host_config: Some(host_config),
                labels: Some(labels),
                ..Default::default()
            };

            let container_name = format!(
                "y-agent-{}",
                uuid::Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("x")
            );

            let start = std::time::Instant::now();

            // Create container.
            let create_result = self
                .client
                .create_container(
                    Some(CreateContainerOptions {
                        name: &container_name,
                        platform: None,
                    }),
                    config,
                )
                .await
                .map_err(|e| RuntimeError::ContainerError {
                    message: format!("failed to create container: {e}"),
                })?;

            let container_id = create_result.id.clone();

            // Start container.
            self.client
                .start_container(&container_id, None::<StartContainerOptions<String>>)
                .await
                .map_err(|e| {
                    let _ = self.remove_container_sync(&container_id);
                    RuntimeError::ContainerError {
                        message: format!("failed to start container: {e}"),
                    }
                })?;

            // Wait for completion with timeout.
            let wait_result = tokio::time::timeout(timeout, self.wait_container(&container_id))
                .await
                .map_err(|_| {
                    // Timeout: kill the container.
                    tracing::warn!(
                        container_id = %container_id,
                        "container execution timed out, killing"
                    );
                    RuntimeError::Timeout { timeout }
                });

            // On timeout, try to stop + collect partial output.
            let exit_code = match wait_result {
                Ok(Ok(code)) => code,
                Ok(Err(e)) => {
                    let _ = self.stop_and_remove(&container_id).await;
                    return Err(e);
                }
                Err(timeout_err) => {
                    // Collect partial output before removing.
                    let (_stdout, _stderr) = self.collect_logs(&container_id).await;
                    let _ = self.stop_and_remove(&container_id).await;
                    let duration = start.elapsed();

                    self.log_audit(
                        &request.command,
                        AuditOutcome::Failed {
                            error: "timeout".into(),
                        },
                        Some(serde_json::json!({
                            "container_id": container_id,
                            "duration_ms": duration.as_millis(),
                        })),
                    )
                    .await;

                    // Return partial result on timeout instead of just an error.
                    return Err(timeout_err);
                }
            };

            let duration = start.elapsed();

            // Collect logs.
            let (stdout, stderr) = self.collect_logs(&container_id).await;

            // Remove container.
            let _ = self.remove_container(&container_id).await;

            let result = ExecutionResult {
                exit_code,
                stdout,
                stderr,
                duration,
                resource_usage: ResourceUsage::default(),
            };

            // Audit logging.
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
                    "container_id": container_id,
                    "image": image,
                    "duration_ms": duration.as_millis(),
                    "exit_code": exit_code,
                })),
            )
            .await;

            Ok(result)
        }

        /// Ensure the image exists locally, pulling if needed and allowed.
        async fn ensure_image(&self, image: &str) -> Result<(), RuntimeError> {
            // Check if image exists locally.
            match self.client.inspect_image(image).await {
                Ok(_) => return Ok(()),
                Err(_) => {
                    // Image not found locally.
                    if !self.config.allow_image_pull {
                        return Err(RuntimeError::Other {
                            message: format!(
                                "image '{image}' not found locally and pulling is disabled"
                            ),
                        });
                    }
                }
            }

            // Pull the image.
            tracing::info!(image = %image, "pulling Docker image");
            let options = Some(CreateImageOptions {
                from_image: image,
                ..Default::default()
            });

            let mut stream = self.client.create_image(options, None, None);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(info) => {
                        tracing::debug!(?info, "image pull progress");
                    }
                    Err(e) => {
                        return Err(RuntimeError::ContainerError {
                            message: format!("failed to pull image '{image}': {e}"),
                        });
                    }
                }
            }

            self.log_audit(
                image,
                AuditOutcome::Success,
                Some(serde_json::json!({"event": "image_pull"})),
            )
            .await;

            Ok(())
        }

        /// Wait for the container to finish and return exit code.
        async fn wait_container(&self, container_id: &str) -> Result<i32, RuntimeError> {
            let options = Some(WaitContainerOptions {
                condition: "not-running",
            });

            let mut stream = self.client.wait_container(container_id, options);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(response) => {
                        return Ok(i32::try_from(response.status_code).unwrap_or(-1));
                    }
                    Err(e) => {
                        return Err(RuntimeError::ContainerError {
                            message: format!("error waiting for container: {e}"),
                        });
                    }
                }
            }

            Err(RuntimeError::ContainerError {
                message: "container wait stream ended without result".into(),
            })
        }

        /// Collect stdout and stderr from container logs.
        async fn collect_logs(&self, container_id: &str) -> (Vec<u8>, Vec<u8>) {
            let options = Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow: false,
                ..Default::default()
            });

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();

            let mut stream = self.client.logs(container_id, options);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(LogOutput::StdOut { message }) => stdout.extend_from_slice(&message),
                    Ok(LogOutput::StdErr { message }) => stderr.extend_from_slice(&message),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "error collecting container logs");
                        break;
                    }
                }
            }

            (stdout, stderr)
        }

        /// Stop and remove a container.
        async fn stop_and_remove(&self, container_id: &str) -> Result<(), RuntimeError> {
            let _ = self
                .client
                .stop_container(
                    container_id,
                    Some(StopContainerOptions { t: 5 }), // 5 second grace period
                )
                .await;
            self.remove_container(container_id).await
        }

        /// Remove a container.
        async fn remove_container(&self, container_id: &str) -> Result<(), RuntimeError> {
            self.client
                .remove_container(
                    container_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        v: true, // Remove anonymous volumes.
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|e| RuntimeError::ContainerError {
                    message: format!("failed to remove container: {e}"),
                })
        }

        /// Synchronous container removal attempt (best-effort, used in error paths).
        fn remove_container_sync(&self, _container_id: &str) -> Result<(), RuntimeError> {
            // Best-effort; actual removal happens in cleanup().
            Ok(())
        }

        /// Spawn a long-running container.
        pub async fn spawn(
            &self,
            request: &ExecutionRequest,
        ) -> Result<ProcessHandle, RuntimeError> {
            let image = request
                .image
                .as_deref()
                .ok_or_else(|| RuntimeError::Other {
                    message: "Docker runtime requires an image specification".into(),
                })?;

            if !self.config.image_whitelist.contains(image) {
                return Err(RuntimeError::ImageNotAllowed {
                    image: image.to_string(),
                });
            }

            self.ensure_image(image).await?;

            let host_config = self.build_host_config(request);
            let mut cmd = vec![request.command.clone()];
            cmd.extend(request.args.clone());

            let env: Vec<String> = request
                .env
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();

            let mut labels = std::collections::HashMap::new();
            labels.insert("y-agent.managed".to_string(), "true".to_string());

            let config = Config {
                image: Some(image.to_string()),
                cmd: Some(cmd),
                env: if env.is_empty() { None } else { Some(env) },
                working_dir: request.working_dir.clone(),
                host_config: Some(host_config),
                labels: Some(labels),
                ..Default::default()
            };

            let container_name = format!(
                "y-agent-{}",
                uuid::Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("x")
            );

            let create_result = self
                .client
                .create_container(
                    Some(CreateContainerOptions {
                        name: &container_name,
                        platform: None,
                    }),
                    config,
                )
                .await
                .map_err(|e| RuntimeError::ContainerError {
                    message: format!("failed to create container: {e}"),
                })?;

            let container_id = create_result.id.clone();

            self.client
                .start_container(&container_id, None::<StartContainerOptions<String>>)
                .await
                .map_err(|e| RuntimeError::ContainerError {
                    message: format!("failed to start container: {e}"),
                })?;

            let handle_id = uuid::Uuid::new_v4().to_string();
            let handle = ProcessHandle {
                id: handle_id.clone(),
                backend: RuntimeBackend::Docker,
            };

            self.spawned.lock().await.insert(handle_id, container_id);

            Ok(handle)
        }

        /// Kill a spawned container.
        pub async fn kill(&self, handle: &ProcessHandle) -> Result<(), RuntimeError> {
            let mut spawned = self.spawned.lock().await;
            if let Some(container_id) = spawned.remove(&handle.id) {
                self.stop_and_remove(&container_id).await
            } else {
                Err(RuntimeError::Other {
                    message: format!("no spawned container with handle {}", handle.id),
                })
            }
        }

        /// Check the status of a spawned container.
        pub async fn status(&self, handle: &ProcessHandle) -> Result<ProcessStatus, RuntimeError> {
            let spawned = self.spawned.lock().await;
            if let Some(container_id) = spawned.get(&handle.id) {
                match self.client.inspect_container(container_id, None).await {
                    Ok(info) => {
                        let state = info.state.as_ref();
                        let running = state.and_then(|s| s.running).unwrap_or(false);
                        if running {
                            Ok(ProcessStatus::Running)
                        } else {
                            let exit_code = state
                                .and_then(|s| s.exit_code)
                                .and_then(|c| i32::try_from(c).ok())
                                .unwrap_or(-1);
                            Ok(ProcessStatus::Completed { exit_code })
                        }
                    }
                    Err(e) => Ok(ProcessStatus::Failed {
                        error: format!("{e}"),
                    }),
                }
            } else {
                Ok(ProcessStatus::Unknown)
            }
        }

        /// Clean up all y-agent managed containers.
        pub async fn cleanup(&self) -> Result<(), RuntimeError> {
            use bollard::container::ListContainersOptions;

            let mut filters = std::collections::HashMap::new();
            filters.insert("label", vec!["y-agent.managed=true"]);

            let options = Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            });

            match self.client.list_containers(options).await {
                Ok(containers) => {
                    for container in containers {
                        if let Some(id) = container.id {
                            tracing::info!(container_id = %id, "cleaning up managed container");
                            let _ = self.stop_and_remove(&id).await;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to list containers for cleanup");
                }
            }

            // Also clean up tracked spawned containers.
            let mut spawned = self.spawned.lock().await;
            for (_, container_id) in spawned.drain() {
                let _ = self.stop_and_remove(&container_id).await;
            }

            Ok(())
        }

        fn effective_timeout(&self, request: &ExecutionRequest) -> Duration {
            request
                .capabilities
                .container
                .resources
                .timeout
                .unwrap_or(self.config.default_timeout)
        }

        async fn log_audit(
            &self,
            command: &str,
            outcome: AuditOutcome,
            metadata: Option<serde_json::Value>,
        ) {
            if let Some(ref audit) = self.audit_trail {
                audit
                    .log_tool_execution("docker-runtime", command, outcome, metadata)
                    .await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public DockerRuntime struct
// ---------------------------------------------------------------------------

/// Docker runtime backend using the Docker Engine API.
///
/// Provides container-based isolation for untrusted tool execution.
/// Each execution creates a transient container that is removed on completion.
///
/// Security hardening (when `runtime_docker` feature is enabled):
/// - Read-only root filesystem
/// - `no-new-privileges` security option
/// - All Linux capabilities dropped by default
/// - Network mode `none` by default
/// - Bind mounts only from declared filesystem capabilities
pub struct DockerRuntime {
    config: RuntimeConfig,
    #[allow(dead_code)]
    audit_trail: Option<Arc<AuditTrail>>,
    #[cfg(feature = "runtime_docker")]
    inner: Option<Arc<docker_impl::DockerInner>>,
}

impl DockerRuntime {
    /// Create a new Docker runtime with the given config.
    pub fn new(config: RuntimeConfig) -> Self {
        Self::with_audit(config, None)
    }

    /// Create a new Docker runtime with config and optional audit trail.
    pub fn with_audit(config: RuntimeConfig, audit_trail: Option<Arc<AuditTrail>>) -> Self {
        #[cfg(feature = "runtime_docker")]
        let inner = {
            match bollard::Docker::connect_with_local_defaults() {
                Ok(client) => Some(Arc::new(docker_impl::DockerInner::new(
                    client,
                    config.clone(),
                    audit_trail.clone(),
                ))),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to connect to Docker daemon");
                    None
                }
            }
        };

        Self {
            config,
            audit_trail,
            #[cfg(feature = "runtime_docker")]
            inner,
        }
    }

    /// Get the effective timeout for a request.
    #[allow(dead_code)]
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
    fn name(&self) -> &'static str {
        "docker"
    }

    #[instrument(skip(self, request), fields(command = %request.command, backend = "docker"))]
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.execute(&request).await;
            }
            return Err(RuntimeError::RuntimeNotAvailable {
                backend: RuntimeBackend::Docker,
            });
        }

        #[cfg(not(feature = "runtime_docker"))]
        {
            let image = request
                .image
                .as_deref()
                .ok_or_else(|| RuntimeError::Other {
                    message: "Docker runtime requires an image specification".into(),
                })?;

            // Verify image is whitelisted even in skeleton mode.
            if !self.config.image_whitelist.contains(image) {
                return Err(RuntimeError::ImageNotAllowed {
                    image: image.to_string(),
                });
            }

            let _timeout = self.effective_timeout(&request);

            Err(RuntimeError::RuntimeNotAvailable {
                backend: RuntimeBackend::Docker,
            })
        }
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.ping().await;
            }
        }

        Ok(RuntimeHealth {
            backend: RuntimeBackend::Docker,
            available: false,
            message: Some("Docker runtime not available".into()),
        })
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Docker
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.cleanup().await;
            }
        }

        Ok(())
    }

    async fn spawn(&self, request: ExecutionRequest) -> Result<ProcessHandle, RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.spawn(&request).await;
            }
        }

        let _ = request;
        Err(RuntimeError::RuntimeNotAvailable {
            backend: RuntimeBackend::Docker,
        })
    }

    async fn kill(&self, handle: &ProcessHandle) -> Result<(), RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.kill(handle).await;
            }
        }

        let _ = handle;
        Err(RuntimeError::RuntimeNotAvailable {
            backend: RuntimeBackend::Docker,
        })
    }

    async fn status(&self, handle: &ProcessHandle) -> Result<ProcessStatus, RuntimeError> {
        #[cfg(feature = "runtime_docker")]
        {
            if let Some(ref inner) = self.inner {
                return inner.status(handle).await;
            }
        }

        let _ = handle;
        Err(RuntimeError::RuntimeNotAvailable {
            backend: RuntimeBackend::Docker,
        })
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
            image: image.map(std::string::ToString::to_string),
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
        // Without the runtime_docker feature or Docker daemon, reports unavailable.
        // (May report available if Docker is running and feature is enabled.)
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

    // T-R1-06: name() returns "docker".
    #[test]
    fn test_docker_name() {
        let rt = DockerRuntime::new(RuntimeConfig::default());
        assert_eq!(rt.name(), "docker");
    }
}
