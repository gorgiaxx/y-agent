//! Tool traits and associated types.
//!
//! Design reference: tools-design.md
//!
//! The tool system supports four types: built-in, MCP, custom, and dynamic.
//! Tools are lazily loaded via `ToolIndex` + `ToolSearch` to minimize
//! context window consumption (60-90% token reduction).

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::runtime::{CommandRunner, RuntimeCapability};
use crate::types::{SessionId, ToolName};

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

/// Complete definition of a tool, including its schema and runtime requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool name.
    pub name: ToolName,
    /// Human-readable description (injected into LLM context).
    pub description: String,
    /// Detailed usage instructions for the LLM. Returned only on direct
    /// tool lookup (`ToolSearch(tool: "name")`), not in search/browse results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// JSON Schema (Draft 7) for input parameters.
    pub parameters: serde_json::Value,
    /// JSON Schema for output (optional, for documentation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<serde_json::Value>,
    /// Tool category for organization and filtering.
    pub category: ToolCategory,
    /// Tool type.
    pub tool_type: ToolType,
    /// Runtime capability requirements.
    pub capabilities: RuntimeCapability,
    /// Whether this tool is considered dangerous (triggers guardrail checks).
    #[serde(default)]
    pub is_dangerous: bool,
}

/// Tool category for organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    FileSystem,
    Network,
    Shell,
    Search,
    Memory,
    Knowledge,
    Agent,
    Workflow,
    Schedule,
    Interaction,
    Custom,
}

/// The origin/type of a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolType {
    /// Compiled into the binary.
    BuiltIn,
    /// Provided by an MCP server.
    Mcp,
    /// User-defined (loaded from config).
    Custom,
    /// Agent-created at runtime (always sandboxed).
    Dynamic,
}

/// Compact tool entry for the `ToolIndex` (lazy loading).
/// Contains only name and description -- enough for LLM to decide
/// whether to invoke `ToolSearch` for the full definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIndexEntry {
    pub name: ToolName,
    pub description: String,
    pub category: ToolCategory,
}

// ---------------------------------------------------------------------------
// Execution types
// ---------------------------------------------------------------------------

/// Input to a tool execution.
#[derive(Clone)]
pub struct ToolInput {
    /// Tool call ID (links back to LLM request).
    pub call_id: String,
    /// Tool name.
    pub name: ToolName,
    /// Validated arguments (already passed JSON Schema validation).
    pub arguments: serde_json::Value,
    /// Session context for the execution.
    pub session_id: SessionId,
    /// Runtime command runner, injected by the executor.
    /// `None` for tools that don't execute shell commands (`FileRead`, etc.).
    /// When `Some`, tools like `ShellExec` delegate execution through this
    /// runner instead of spawning local processes directly.
    pub command_runner: Option<Arc<dyn CommandRunner>>,
}

impl std::fmt::Debug for ToolInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolInput")
            .field("call_id", &self.call_id)
            .field("name", &self.name)
            .field("arguments", &self.arguments)
            .field("session_id", &self.session_id)
            .field(
                "command_runner",
                &self.command_runner.as_ref().map(|_| ".."),
            )
            .finish()
    }
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Tool output content (typically JSON or text).
    pub content: serde_json::Value,
    /// Non-fatal warnings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Execution metadata (duration, resource usage, etc.).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from tool operations.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {name}")]
    NotFound { name: String },

    #[error("parameter validation failed: {message}")]
    ValidationError { message: String },

    #[error("permission denied for tool {name}: {reason}")]
    PermissionDenied { name: String, reason: String },

    #[error("tool execution timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },

    #[error("tool {name} rate limited")]
    RateLimited { name: String },

    #[error("runtime error executing {name}: {message}")]
    RuntimeError { name: String, message: String },

    #[error("external service error in {name}: {message}")]
    ExternalServiceError { name: String, message: String },

    #[error("cancelled by user")]
    Cancelled,

    #[error("{message}")]
    Other { message: String },
}

impl ToolError {
    /// Whether this error is safe to retry.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout { .. } | Self::RateLimited { .. } | Self::ExternalServiceError { .. }
        )
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// A single executable tool.
///
/// Implementations handle the business logic of the tool. Security
/// enforcement (capability checks, sandboxing) is handled by the
/// `RuntimeAdapter`, not by the tool itself.
///
/// Tools participate in the permission pipeline via three hooks:
/// - [`check_permissions`](Tool::check_permissions) -- input-specific permission check
/// - [`is_read_only`](Tool::is_read_only) -- declares no side effects
/// - [`is_destructive`](Tool::is_destructive) -- declares irreversible side effects
#[async_trait]
pub trait Tool: Send + Sync {
    /// Execute the tool with validated input.
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError>;

    /// Return the tool's definition (schema, capabilities, metadata).
    fn definition(&self) -> &ToolDefinition;

    /// Tool-specific permission check.
    ///
    /// Called after global deny/ask rules but before mode-based overrides.
    /// The tool can inspect the input to make content-specific decisions:
    /// - `ShellExec` can check the command against subcommand rules
    /// - `FileWrite` can check the target path against path safety rules
    ///
    /// Return `Passthrough` to defer to the general permission system.
    ///
    /// Default implementation: read-only tools return `Allow`,
    /// all others return `Passthrough`.
    fn check_permissions(
        &self,
        _input: &ToolInput,
        _context: &crate::permission_types::PermissionContext,
    ) -> crate::permission_types::PermissionResult {
        if self.is_read_only() {
            crate::permission_types::PermissionResult::allow("read-only tool")
        } else {
            crate::permission_types::PermissionResult::passthrough()
        }
    }

    /// Whether this tool is read-only (no side effects).
    ///
    /// Read-only tools default to `Allow` in the permission pipeline.
    /// Override to `true` for tools like `FileRead`, `Glob`, `Grep`.
    fn is_read_only(&self) -> bool {
        false
    }

    /// Whether executing this tool can cause irreversible changes.
    ///
    /// Destructive tools may receive additional scrutiny in the
    /// permission pipeline (e.g., auto-escalate to Ask even if
    /// a general Allow rule exists).
    fn is_destructive(&self) -> bool {
        false
    }

    /// Downcast support for concrete type access (e.g. hot-reload).
    fn as_any(&self) -> &dyn std::any::Any {
        // Default returns a type that won't match anything useful.
        // Concrete types should override this to return `self`.
        &()
    }
}

/// Registry managing all available tools with lazy loading support.
///
/// At session start, only the `ToolIndex` (name + description) is injected
/// into the LLM context. Full definitions are loaded on demand via
/// `ToolSearch`, and active tools are tracked in a session-scoped
/// `ToolActivationSet` with LRU eviction.
#[async_trait]
pub trait ToolRegistry: Send + Sync {
    /// Get the compact tool index for context injection.
    async fn tool_index(&self) -> Vec<ToolIndexEntry>;

    /// Search for tools by name or keyword, returning full definitions.
    async fn search(&self, query: &str) -> Result<Vec<ToolDefinition>, ToolError>;

    /// Get a specific tool by name.
    async fn get(&self, name: &ToolName) -> Result<Box<dyn Tool>, ToolError>;

    /// Register a new tool (used for dynamic tool creation).
    async fn register(&self, definition: ToolDefinition) -> Result<(), ToolError>;

    /// Remove a tool from the registry.
    async fn unregister(&self, name: &ToolName) -> Result<(), ToolError>;
}
