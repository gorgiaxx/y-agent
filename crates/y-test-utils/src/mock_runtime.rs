//! Mock `RuntimeAdapter` for testing tool execution without real containers.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use y_core::runtime::{
    ExecutionRequest, ExecutionResult, ResourceUsage, RuntimeAdapter, RuntimeBackend, RuntimeError,
    RuntimeHealth,
};

/// Configurable mock runtime for tests.
#[derive(Debug, Clone)]
pub struct MockRuntime {
    /// Pre-configured results keyed by command string.
    results: Arc<RwLock<HashMap<String, ExecutionResult>>>,
    /// Default result when no match is found.
    default_result: ExecutionResult,
}

impl MockRuntime {
    /// Create with a default successful result.
    #[must_use]
    pub fn new() -> Self {
        Self {
            results: Arc::new(RwLock::new(HashMap::new())),
            default_result: ExecutionResult {
                exit_code: 0,
                stdout: b"ok".to_vec(),
                stderr: vec![],
                duration: Duration::from_millis(5),
                resource_usage: ResourceUsage::default(),
            },
        }
    }

    /// Register a canned result for a specific command.
    ///
    /// # Panics
    ///
    /// Panics if the internal `RwLock` is poisoned.
    #[must_use]
    pub fn with_result(self, command: impl Into<String>, result: ExecutionResult) -> Self {
        self.results.write().unwrap().insert(command.into(), result);
        self
    }

    /// Create a mock that always fails execution.
    #[must_use]
    pub fn failing() -> Self {
        Self {
            results: Arc::new(RwLock::new(HashMap::new())),
            default_result: ExecutionResult {
                exit_code: 1,
                stdout: vec![],
                stderr: b"mock execution failed".to_vec(),
                duration: Duration::from_millis(1),
                resource_usage: ResourceUsage::default(),
            },
        }
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeAdapter for MockRuntime {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        let map = self.results.read().unwrap();
        if let Some(result) = map.get(&request.command) {
            Ok(result.clone())
        } else {
            Ok(self.default_result.clone())
        }
    }

    async fn health_check(&self) -> Result<RuntimeHealth, RuntimeError> {
        Ok(RuntimeHealth {
            backend: RuntimeBackend::Native,
            available: true,
            message: Some("mock runtime healthy".into()),
        })
    }

    fn backend(&self) -> RuntimeBackend {
        RuntimeBackend::Native
    }

    async fn cleanup(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::runtime::RuntimeCapability;

    fn make_exec_req(command: &str) -> ExecutionRequest {
        ExecutionRequest {
            command: command.into(),
            args: vec![],
            working_dir: None,
            env: HashMap::new(),
            stdin: None,
            owner_session_id: None,
            capabilities: RuntimeCapability::default(),
            image: None,
        }
    }

    #[tokio::test]
    async fn test_default_execution() {
        let rt = MockRuntime::new();
        let result = rt.execute(make_exec_req("echo hello")).await.unwrap();
        assert!(result.success());
        assert_eq!(result.stdout_string(), "ok");
    }

    #[tokio::test]
    async fn test_canned_result() {
        let rt = MockRuntime::new().with_result(
            "ls -la",
            ExecutionResult {
                exit_code: 0,
                stdout: b"total 42".to_vec(),
                stderr: vec![],
                duration: Duration::from_millis(2),
                resource_usage: ResourceUsage::default(),
            },
        );
        let result = rt.execute(make_exec_req("ls -la")).await.unwrap();
        assert_eq!(result.stdout_string(), "total 42");
    }

    #[tokio::test]
    async fn test_health_check() {
        let rt = MockRuntime::new();
        let health = rt.health_check().await.unwrap();
        assert!(health.available);
    }
}
