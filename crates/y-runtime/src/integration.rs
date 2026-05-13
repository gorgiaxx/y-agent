//! Runtime-Tools integration layer: bridges tool execution with runtime isolation.
//!
//! Design reference: runtime-tools-integration-design.md
//!
//! The integration layer constructs a `RuntimeContext` from a tool's manifest,
//! performs a 4-layer capability check, dispatches to the appropriate execution
//! pattern (local, container, skill-container), and handles cross-module errors.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Execution patterns
// ---------------------------------------------------------------------------

/// Execution pattern for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPattern {
    /// Execute directly on the host (built-in tools).
    Local,
    /// Execute in a Docker container (sandboxed).
    ContainerIsolated,
    /// Execute inside a running skill container.
    SkillContainer,
}

// ---------------------------------------------------------------------------
// Runtime context
// ---------------------------------------------------------------------------

/// Context constructed from a tool manifest for runtime dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContext {
    /// The tool requesting execution.
    pub caller_tool_name: String,
    /// Required capabilities (from tool manifest).
    pub capabilities: Vec<String>,
    /// Preferred container image (if running in container).
    pub preferred_image: Option<String>,
    /// Resource limits.
    pub resource_limits: ResourceLimits,
    /// Volume mounts for container execution.
    pub mounts: Vec<Mount>,
    /// Execution pattern to use.
    pub pattern: ExecutionPattern,
}

/// Resource limits for execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum CPU shares (0 = unlimited).
    pub cpu_shares: u64,
    /// Maximum memory in bytes (0 = unlimited).
    pub memory_bytes: u64,
    /// Maximum execution time in seconds (0 = unlimited).
    pub timeout_seconds: u64,
}

/// A volume mount for container execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    /// Host path.
    pub source: String,
    /// Container path.
    pub target: String,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

/// A command to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Program to run.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: Vec<(String, String)>,
    /// Working directory.
    pub working_dir: Option<String>,
}

// ---------------------------------------------------------------------------
// Capability checking (4-layer)
// ---------------------------------------------------------------------------

/// Result of a capability check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityCheckResult {
    /// All checks passed.
    Allowed,
    /// Check failed at a specific layer.
    Denied { layer: String, reason: String },
}

/// 4-layer capability checking pipeline.
///
/// Layers:
/// 1. Permission Model — user's allow/deny/ask permissions
/// 2. Image Whitelist — container image validation
/// 3. Resource Limits — resource budget validation
/// 4. Structural Rules — config-time structural guardrails
pub fn check_capabilities(ctx: &RuntimeContext) -> CapabilityCheckResult {
    // Layer 1: Permission model (stub — in production, checks user permissions).
    // For now, "ShellExec" requires explicit allowance.
    if ctx.capabilities.contains(&"ShellExec".to_string()) && ctx.pattern == ExecutionPattern::Local
    {
        return CapabilityCheckResult::Denied {
            layer: "permission_model".to_string(),
            reason: "ShellExec requires container isolation".to_string(),
        };
    }

    // Layer 2: Image whitelist (stub — in production, delegates to ImageWhitelist).
    if ctx.pattern == ExecutionPattern::ContainerIsolated {
        if let Some(image) = &ctx.preferred_image {
            if image.is_empty() {
                return CapabilityCheckResult::Denied {
                    layer: "image_whitelist".to_string(),
                    reason: "empty image name".to_string(),
                };
            }
        }
    }

    // Layer 3: Resource limits.
    if ctx.resource_limits.timeout_seconds > 3600 {
        return CapabilityCheckResult::Denied {
            layer: "resource_limits".to_string(),
            reason: "timeout exceeds 1 hour maximum".to_string(),
        };
    }

    // Layer 4: Structural rules (stub).
    // In production, checks against config-time structural guardrails.

    CapabilityCheckResult::Allowed
}

// ---------------------------------------------------------------------------
// Integration error
// ---------------------------------------------------------------------------

/// Cross-module error type wrapping both tool and runtime errors.
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("capability check denied at layer '{layer}': {reason}")]
    CapabilityDenied { layer: String, reason: String },
    #[error("execution failed: {message}")]
    ExecutionFailed { message: String },
    #[error("invalid runtime context: {message}")]
    InvalidContext { message: String },
}

impl From<CapabilityCheckResult> for Result<(), IntegrationError> {
    fn from(result: CapabilityCheckResult) -> Self {
        match result {
            CapabilityCheckResult::Allowed => Ok(()),
            CapabilityCheckResult::Denied { layer, reason } => {
                Err(IntegrationError::CapabilityDenied { layer, reason })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Integration manager
// ---------------------------------------------------------------------------

/// Manages runtime-tools integration: context construction, capability
/// checking, and execution dispatch.
#[derive(Debug, Default)]
pub struct IntegrationManager;

impl IntegrationManager {
    /// Create a new integration manager.
    pub fn new() -> Self {
        Self
    }

    /// Build a `RuntimeContext` for a tool execution request.
    pub fn build_context(
        &self,
        tool_name: &str,
        capabilities: Vec<String>,
        pattern: ExecutionPattern,
    ) -> RuntimeContext {
        RuntimeContext {
            caller_tool_name: tool_name.to_string(),
            capabilities,
            preferred_image: None,
            resource_limits: ResourceLimits::default(),
            mounts: vec![],
            pattern,
        }
    }

    /// Execute a command through the integration layer.
    ///
    /// 1. Capability check (4-layer)
    /// 2. Dispatch to runtime adapter
    /// 3. Return result or error
    pub fn execute(
        &self,
        ctx: &RuntimeContext,
        _cmd: &Command,
    ) -> Result<String, IntegrationError> {
        // Step 1: Capability check
        let check = check_capabilities(ctx);
        let _: () = Result::from(check)?;

        // Step 2: Dispatch (stub — in production, delegate to DockerRuntime/NativeRuntime).
        match ctx.pattern {
            ExecutionPattern::Local => Ok(format!("local execution of '{}'", ctx.caller_tool_name)),
            ExecutionPattern::ContainerIsolated => Ok(format!(
                "container execution of '{}' in image '{}'",
                ctx.caller_tool_name,
                ctx.preferred_image.as_deref().unwrap_or("default")
            )),
            ExecutionPattern::SkillContainer => Ok(format!(
                "skill-container execution of '{}'",
                ctx.caller_tool_name
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P3-40-01: Build runtime context from tool info.
    #[test]
    fn test_build_context() {
        let mgr = IntegrationManager::new();
        let ctx = mgr.build_context(
            "FileRead",
            vec!["file_system".to_string()],
            ExecutionPattern::Local,
        );
        assert_eq!(ctx.caller_tool_name, "FileRead");
        assert_eq!(ctx.pattern, ExecutionPattern::Local);
    }

    /// T-P3-40-02: Capability check allows safe local execution.
    #[test]
    fn test_capability_check_allowed() {
        let ctx = RuntimeContext {
            caller_tool_name: "FileRead".into(),
            capabilities: vec!["file_system".into()],
            preferred_image: None,
            resource_limits: ResourceLimits::default(),
            mounts: vec![],
            pattern: ExecutionPattern::Local,
        };
        assert_eq!(check_capabilities(&ctx), CapabilityCheckResult::Allowed);
    }

    /// T-P3-40-03: `ShellExec` on local is denied (requires container).
    #[test]
    fn test_capability_check_shell_exec_local_denied() {
        let ctx = RuntimeContext {
            caller_tool_name: "run_script".into(),
            capabilities: vec!["ShellExec".into()],
            preferred_image: None,
            resource_limits: ResourceLimits::default(),
            mounts: vec![],
            pattern: ExecutionPattern::Local,
        };
        let result = check_capabilities(&ctx);
        assert!(matches!(result, CapabilityCheckResult::Denied { .. }));
    }

    /// T-P3-40-04: Excessive timeout is denied.
    #[test]
    fn test_capability_check_timeout_denied() {
        let ctx = RuntimeContext {
            caller_tool_name: "long_task".into(),
            capabilities: vec![],
            preferred_image: None,
            resource_limits: ResourceLimits {
                cpu_shares: 0,
                memory_bytes: 0,
                timeout_seconds: 7200, // 2 hours > 1 hour max
            },
            mounts: vec![],
            pattern: ExecutionPattern::Local,
        };
        let result = check_capabilities(&ctx);
        assert!(matches!(result, CapabilityCheckResult::Denied { .. }));
    }

    /// T-P3-40-05: Container execution goes through integration.
    #[test]
    fn test_container_execution() {
        let mgr = IntegrationManager::new();
        let ctx = RuntimeContext {
            caller_tool_name: "python_script".into(),
            capabilities: vec!["ShellExec".into()],
            preferred_image: Some("python:3.12".into()),
            resource_limits: ResourceLimits::default(),
            mounts: vec![],
            pattern: ExecutionPattern::ContainerIsolated,
        };
        let cmd = Command {
            program: "python3".into(),
            args: vec!["script.py".into()],
            env: vec![],
            working_dir: None,
        };
        let result = mgr.execute(&ctx, &cmd).unwrap();
        assert!(result.contains("container execution"));
    }

    /// T-P3-40-06: `IntegrationError` from denied capability.
    #[test]
    fn test_integration_error_from_denied() {
        let mgr = IntegrationManager::new();
        let ctx = RuntimeContext {
            caller_tool_name: "danger".into(),
            capabilities: vec!["ShellExec".into()],
            preferred_image: None,
            resource_limits: ResourceLimits::default(),
            mounts: vec![],
            pattern: ExecutionPattern::Local,
        };
        let cmd = Command {
            program: "rm".into(),
            args: vec!["-rf".into(), "/".into()],
            env: vec![],
            working_dir: None,
        };
        let result = mgr.execute(&ctx, &cmd);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IntegrationError::CapabilityDenied { .. }
        ));
    }
}
