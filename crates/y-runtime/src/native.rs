//! Native runtime: process execution via `tokio::process`.
//!
//! The native runtime executes commands directly on the host. It is the
//! simplest and fastest backend, suitable for trusted tool execution.

use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tracing::instrument;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, ResourceUsage, RuntimeAdapter, RuntimeBackend,
    RuntimeError, RuntimeHealth,
};

use crate::config::RuntimeConfig;

/// Native runtime backend using `tokio::process::Command`.
///
/// Executes commands directly on the host OS with timeout protection
/// and output size limiting.
pub struct NativeRuntime {
    config: RuntimeConfig,
}

impl NativeRuntime {
    /// Create a new native runtime with the given config.
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
}

#[async_trait]
impl RuntimeAdapter for NativeRuntime {
    #[instrument(skip(self, request), fields(command = %request.command, backend = "native"))]
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        let timeout = self.effective_timeout(&request);
        let max_output = self.effective_max_output(&request);

        let mut cmd = tokio::process::Command::new(&request.command);
        cmd.args(&request.args);

        if let Some(ref dir) = request.working_dir {
            cmd.current_dir(dir);
        }

        for (key, val) in &request.env {
            cmd.env(key, val);
        }

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

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
            duration,
            resource_usage: ResourceUsage::default(),
        })
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
        // Native runtime has no persistent resources to clean up.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use y_core::runtime::{ContainerCapability, ResourceLimits, RuntimeCapability};

    use super::*;

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
        let rt = NativeRuntime::new(RuntimeConfig::default());
        let req = default_request("echo", &["hello"]);
        let result = rt.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_string().contains("hello"));
    }

    // T-RT-002-02
    #[tokio::test]
    async fn test_native_execute_failing_command() {
        let rt = NativeRuntime::new(RuntimeConfig::default());
        let req = default_request("false", &[]);
        let result = rt.execute(req).await.unwrap();
        assert_ne!(result.exit_code, 0);
    }

    // T-RT-002-03
    #[tokio::test]
    async fn test_native_execute_with_env() {
        let rt = NativeRuntime::new(RuntimeConfig::default());
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
        let rt = NativeRuntime::new(RuntimeConfig::default());
        let mut req = default_request("cat", &[]);
        req.stdin = Some(b"hello from stdin".to_vec());
        let result = rt.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_string().contains("hello from stdin"));
    }

    // T-RT-002-05
    #[tokio::test]
    async fn test_native_execute_timeout() {
        let rt = NativeRuntime::new(RuntimeConfig::default());
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
        let rt = NativeRuntime::new(config);
        // Generate output much larger than the limit.
        let req = default_request("sh", &["-c", "dd if=/dev/zero bs=1024 count=10 2>/dev/null | tr '\\0' 'A'"]);
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
        let rt = NativeRuntime::new(RuntimeConfig::default());
        let health = rt.health_check().await.unwrap();
        assert_eq!(health.backend, RuntimeBackend::Native);
        assert!(health.available);
    }

    // T-RT-002-08
    #[tokio::test]
    async fn test_native_backend_type() {
        let rt = NativeRuntime::new(RuntimeConfig::default());
        assert_eq!(rt.backend(), RuntimeBackend::Native);
    }
}
