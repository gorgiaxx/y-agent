//! Dynamic tool lifecycle management.
//!
//! Design reference: tools-design.md §Dynamic Tools
//!
//! Dynamic tools are created at runtime by agents — script wrappers,
//! HTTP-API calls, or composite pipelines. They are always sandboxed
//! and run through the same validation/middleware pipeline as built-in tools.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use y_core::runtime::{NetworkCapability, ProcessCapability, RuntimeCapability};
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

// ---------------------------------------------------------------------------
// Dynamic tool definitions
// ---------------------------------------------------------------------------

/// The execution backend for a dynamic tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicToolKind {
    /// A shell script to execute.
    Script {
        /// The interpreter to use (e.g., `"bash"`, `"python3"`).
        interpreter: String,
        /// The script source code.
        source: String,
    },
    /// An HTTP API call.
    HttpApi {
        /// HTTP method.
        method: String,
        /// URL template (supports `{{param}}` substitution).
        url: String,
        /// Optional request headers.
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
        /// Optional body template (JSON).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body_template: Option<String>,
    },
    /// A composite tool that chains multiple tool calls.
    Composite {
        /// Ordered list of tool steps to execute.
        steps: Vec<CompositeStep>,
    },
}

/// A single step in a composite tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeStep {
    /// Tool to invoke.
    pub tool_name: ToolName,
    /// Arguments template (supports `{{prev_output}}` substitution).
    pub arguments: serde_json::Value,
    /// Optional label for referencing in subsequent steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A dynamic tool definition that packages the tool kind with its metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolDef {
    /// Tool name.
    pub name: ToolName,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for input parameters.
    pub parameters: serde_json::Value,
    /// The execution kind.
    pub kind: DynamicToolKind,
    /// The creator (agent ID or user).
    pub created_by: String,
    /// Creation timestamp (ISO 8601).
    #[serde(default = "chrono::Utc::now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Version number (increments on update).
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    1
}

const DEFAULT_DYNAMIC_SCRIPT_TIMEOUT_SECS: u64 = 30;
const SCRIPT_HEREDOC_MARKER: &str = "__Y_AGENT_DYNAMIC_SCRIPT__";
const INPUT_HEREDOC_MARKER: &str = "__Y_AGENT_DYNAMIC_INPUT__";

impl DynamicToolDef {
    /// Convert to a `ToolDefinition` for registry insertion.
    pub fn to_tool_definition(&self) -> ToolDefinition {
        let capabilities = match &self.kind {
            DynamicToolKind::Script { .. } => RuntimeCapability {
                process: ProcessCapability {
                    shell: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            DynamicToolKind::HttpApi { .. } => RuntimeCapability {
                network: NetworkCapability::Full,
                ..Default::default()
            },
            DynamicToolKind::Composite { .. } => RuntimeCapability::default(),
        };

        ToolDefinition {
            name: self.name.clone(),
            description: self.description.clone(),
            help: None,
            parameters: self.parameters.clone(),
            result_schema: None,
            category: ToolCategory::Custom,
            tool_type: ToolType::Dynamic,
            capabilities,
            is_dangerous: false,
        }
    }
}

fn validation_error(message: impl Into<String>) -> ToolError {
    ToolError::ValidationError {
        message: message.into(),
    }
}

fn validate_dynamic_tool(def: &DynamicToolDef) -> Result<(), ToolError> {
    if def.description.trim().is_empty() {
        return Err(validation_error(format!(
            "dynamic tool '{}' must have a non-empty description",
            def.name.as_str()
        )));
    }

    if def.created_by.trim().is_empty() {
        return Err(validation_error(format!(
            "dynamic tool '{}' must record its creator",
            def.name.as_str()
        )));
    }

    if !def.parameters.is_object() {
        return Err(validation_error(format!(
            "dynamic tool '{}' parameters must be a JSON object schema",
            def.name.as_str()
        )));
    }

    match &def.kind {
        DynamicToolKind::Script {
            interpreter,
            source,
        } => {
            if interpreter.trim().is_empty() {
                return Err(validation_error(format!(
                    "dynamic tool '{}' script interpreter must be non-empty",
                    def.name.as_str()
                )));
            }

            if source.trim().is_empty() {
                return Err(validation_error(format!(
                    "dynamic tool '{}' script source must be non-empty",
                    def.name.as_str()
                )));
            }

            Ok(())
        }
        DynamicToolKind::HttpApi { .. } => Err(validation_error(format!(
            "dynamic tool '{}' uses HttpApi, which is not enabled in the current phase; only Script tools are supported",
            def.name.as_str()
        ))),
        DynamicToolKind::Composite { .. } => Err(validation_error(format!(
            "dynamic tool '{}' uses Composite, which is not enabled in the current phase; only Script tools are supported",
            def.name.as_str()
        ))),
    }
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(not(windows))]
fn build_script_command(interpreter: &str, source: &str, input_json: &str) -> String {
    format!(
        "set -eu\n\
tmp_script=\"$(mktemp)\"\n\
tmp_input=\"$(mktemp)\"\n\
trap 'rm -f \"$tmp_script\" \"$tmp_input\"' EXIT\n\
cat > \"$tmp_script\" <<'{script_marker}'\n\
{source}\n\
{script_marker}\n\
cat > \"$tmp_input\" <<'{input_marker}'\n\
{input_json}\n\
{input_marker}\n\
{interpreter} \"$tmp_script\" < \"$tmp_input\"",
        script_marker = SCRIPT_HEREDOC_MARKER,
        input_marker = INPUT_HEREDOC_MARKER,
        source = source,
        input_json = input_json,
        interpreter = shell_quote(interpreter),
    )
}

// ---------------------------------------------------------------------------
// Dynamic tool manager
// ---------------------------------------------------------------------------

/// Manages the lifecycle of dynamic (agent-created) tools.
///
/// Provides CRUD operations with audit trail tracking.
pub struct DynamicToolManager {
    /// Known dynamic tool definitions.
    definitions: RwLock<std::collections::HashMap<ToolName, DynamicToolDef>>,
    /// Audit log of all operations.
    audit_log: RwLock<Vec<AuditEntry>>,
}

/// An entry in the dynamic tool audit log (P1-9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp of the operation.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Operation type.
    pub operation: AuditOperation,
    /// Tool name.
    pub tool_name: ToolName,
    /// Who performed the operation.
    pub actor: String,
    /// Optional details (e.g., version number, diff).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// Types of auditable operations on dynamic tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    Create,
    Update,
    Delete,
    Execute,
}

impl DynamicToolManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            definitions: RwLock::new(std::collections::HashMap::new()),
            audit_log: RwLock::new(Vec::new()),
        }
    }

    /// Create a new dynamic tool.
    ///
    /// Returns an error if a tool with the same name already exists.
    pub async fn create_tool(&self, def: DynamicToolDef) -> Result<ToolDefinition, ToolError> {
        validate_dynamic_tool(&def)?;
        let mut defs = self.definitions.write().await;

        if defs.contains_key(&def.name) {
            return Err(ToolError::Other {
                message: format!("dynamic tool '{}' already exists", def.name.as_str()),
            });
        }

        let tool_def = def.to_tool_definition();
        let name = def.name.clone();
        let actor = def.created_by.clone();

        defs.insert(name.clone(), def);
        drop(defs);

        self.log_audit(AuditOperation::Create, &name, &actor, None)
            .await;

        Ok(tool_def)
    }

    /// Update an existing dynamic tool definition.
    ///
    /// Increments the version number automatically.
    pub async fn update_tool(&self, mut def: DynamicToolDef) -> Result<ToolDefinition, ToolError> {
        validate_dynamic_tool(&def)?;
        let mut defs = self.definitions.write().await;

        let existing = defs.get(&def.name).ok_or_else(|| ToolError::NotFound {
            name: def.name.as_str().to_string(),
        })?;

        def.version = existing.version + 1;
        let tool_def = def.to_tool_definition();
        let name = def.name.clone();
        let actor = def.created_by.clone();
        let version = def.version;

        defs.insert(name.clone(), def);
        drop(defs);

        self.log_audit(
            AuditOperation::Update,
            &name,
            &actor,
            Some(format!("version={version}")),
        )
        .await;

        Ok(tool_def)
    }

    /// Delete a dynamic tool.
    pub async fn delete_tool(&self, name: &ToolName, actor: &str) -> Result<(), ToolError> {
        let mut defs = self.definitions.write().await;

        if defs.remove(name).is_none() {
            return Err(ToolError::NotFound {
                name: name.as_str().to_string(),
            });
        }
        drop(defs);

        self.log_audit(AuditOperation::Delete, name, actor, None)
            .await;

        Ok(())
    }

    /// Get a dynamic tool definition.
    pub async fn get_tool(&self, name: &ToolName) -> Option<DynamicToolDef> {
        let defs = self.definitions.read().await;
        defs.get(name).cloned()
    }

    /// List all dynamic tool definitions.
    pub async fn list_tools(&self) -> Vec<DynamicToolDef> {
        let defs = self.definitions.read().await;
        defs.values().cloned().collect()
    }

    /// Record a tool execution event in the audit log.
    pub async fn record_execution(&self, name: &ToolName, actor: &str) {
        self.log_audit(AuditOperation::Execute, name, actor, None)
            .await;
    }

    /// Get the audit log (optionally filtered by tool name).
    pub async fn audit_log(&self, filter_tool: Option<&ToolName>) -> Vec<AuditEntry> {
        let log = self.audit_log.read().await;
        match filter_tool {
            Some(name) => log
                .iter()
                .filter(|e| &e.tool_name == name)
                .cloned()
                .collect(),
            None => log.clone(),
        }
    }

    /// Internal helper to append an audit entry.
    async fn log_audit(
        &self,
        operation: AuditOperation,
        tool_name: &ToolName,
        actor: &str,
        details: Option<String>,
    ) {
        let entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            operation,
            tool_name: tool_name.clone(),
            actor: actor.to_string(),
            details,
        };
        self.audit_log.write().await.push(entry);
    }
}

impl Default for DynamicToolManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Dynamic tool executor adapter
// ---------------------------------------------------------------------------

/// A wrapper that makes a `DynamicToolDef` executable as a `Tool`.
///
/// The current phase supports runtime-backed Script execution only.
/// `HttpApi` and Composite definitions are kept for forward compatibility
/// but are rejected until their later implementation phases.
pub struct DynamicToolAdapter {
    def: DynamicToolDef,
    /// Cached tool definition for zero-alloc access from `definition()`.
    cached_definition: std::sync::OnceLock<ToolDefinition>,
}

impl DynamicToolAdapter {
    /// Create an adapter from a dynamic tool definition.
    pub fn new(def: DynamicToolDef) -> Self {
        Self {
            def,
            cached_definition: std::sync::OnceLock::new(),
        }
    }

    /// Get the underlying definition.
    pub fn def(&self) -> &DynamicToolDef {
        &self.def
    }
}

#[async_trait::async_trait]
impl Tool for DynamicToolAdapter {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        match &self.def.kind {
            DynamicToolKind::Script {
                interpreter,
                source,
            } => {
                let runner =
                    input
                        .command_runner
                        .as_ref()
                        .ok_or_else(|| ToolError::RuntimeError {
                            name: self.def.name.as_str().to_string(),
                            message: "dynamic script execution requires a runtime command runner"
                                .into(),
                        })?;

                #[cfg(windows)]
                {
                    let _ = interpreter;
                    let _ = source;
                    let _ = runner;
                    return Err(ToolError::RuntimeError {
                        name: self.def.name.as_str().to_string(),
                        message: "dynamic script execution is not yet supported on Windows".into(),
                    });
                }

                #[cfg(not(windows))]
                {
                    let serialized_input =
                        serde_json::to_string(&input.arguments).map_err(|e| {
                            ToolError::RuntimeError {
                                name: self.def.name.as_str().to_string(),
                                message: format!("failed to serialize dynamic tool input: {e}"),
                            }
                        })?;

                    let command = build_script_command(interpreter, source, &serialized_input);
                    let result = runner
                        .run_command(
                            &command,
                            None,
                            Duration::from_secs(DEFAULT_DYNAMIC_SCRIPT_TIMEOUT_SECS),
                        )
                        .await
                        .map_err(|e| ToolError::RuntimeError {
                            name: self.def.name.as_str().to_string(),
                            message: format!("{e}"),
                        })?;

                    Ok(ToolOutput {
                        success: result.success(),
                        content: serde_json::json!({
                            "exit_code": result.exit_code,
                            "stdout": result.stdout_string(),
                            "stderr": result.stderr_string(),
                        }),
                        warnings: vec![],
                        metadata: serde_json::json!({
                            "dynamic_tool_kind": "script",
                        }),
                    })
                }
            }
            DynamicToolKind::HttpApi { .. } => Err(validation_error(
                "HttpApi dynamic tools are not enabled in the current phase",
            )),
            DynamicToolKind::Composite { steps } => {
                tracing::warn!(
                    tool = %input.name.as_str(),
                    steps = steps.len(),
                    "composite dynamic tools are not enabled in the current phase"
                );
                Err(validation_error(
                    "Composite dynamic tools are not enabled in the current phase",
                ))
            }
        }
    }

    fn definition(&self) -> &ToolDefinition {
        self.cached_definition
            .get_or_init(|| self.def.to_tool_definition())
    }
}

// ---------------------------------------------------------------------------
// Helper: build an Arc<dyn Tool> from a DynamicToolDef
// ---------------------------------------------------------------------------

/// Convert a `DynamicToolDef` into an `Arc<dyn Tool>`.
pub fn make_dynamic_tool(def: DynamicToolDef) -> Arc<dyn Tool> {
    Arc::new(DynamicToolAdapter::new(def))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc as StdArc, Mutex};
    use std::time::Duration;

    use y_core::runtime::{CommandRunner, ExecutionResult, ResourceUsage, RuntimeError};
    use y_core::types::SessionId;

    use super::*;

    #[derive(Debug)]
    struct RecordingRunner {
        commands: Mutex<Vec<String>>,
        result: ExecutionResult,
    }

    impl RecordingRunner {
        fn succeed(stdout: &str) -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                result: ExecutionResult {
                    exit_code: 0,
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: Vec::new(),
                    duration: Duration::from_millis(10),
                    resource_usage: ResourceUsage::default(),
                },
            }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run_command(
            &self,
            command: &str,
            _working_dir: Option<&str>,
            _timeout: Duration,
        ) -> Result<ExecutionResult, RuntimeError> {
            self.commands.lock().unwrap().push(command.to_string());
            Ok(self.result.clone())
        }
    }

    fn make_input(
        name: &str,
        args: serde_json::Value,
        command_runner: Option<StdArc<dyn CommandRunner>>,
    ) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string(name),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            additional_read_dirs: vec![],
            command_runner,
        }
    }

    fn sample_script_def(name: &str) -> DynamicToolDef {
        DynamicToolDef {
            name: ToolName::from_string(name),
            description: "test script".into(),
            parameters: serde_json::json!({"type": "object"}),
            kind: DynamicToolKind::Script {
                interpreter: "bash".into(),
                source: "echo hello".into(),
            },
            created_by: "test-agent".into(),
            created_at: chrono::Utc::now(),
            version: 1,
        }
    }

    fn sample_http_def(name: &str) -> DynamicToolDef {
        DynamicToolDef {
            name: ToolName::from_string(name),
            description: "test http".into(),
            parameters: serde_json::json!({"type": "object"}),
            kind: DynamicToolKind::HttpApi {
                method: "GET".into(),
                url: "https://example.com/api/{{id}}".into(),
                headers: std::collections::HashMap::new(),
                body_template: None,
            },
            created_by: "test-agent".into(),
            created_at: chrono::Utc::now(),
            version: 1,
        }
    }

    fn sample_composite_def(name: &str) -> DynamicToolDef {
        DynamicToolDef {
            name: ToolName::from_string(name),
            description: "test composite".into(),
            parameters: serde_json::json!({"type": "object"}),
            kind: DynamicToolKind::Composite {
                steps: vec![CompositeStep {
                    tool_name: ToolName::from_string("step1"),
                    arguments: serde_json::json!({"key": "value"}),
                    label: Some("first".into()),
                }],
            },
            created_by: "test-agent".into(),
            created_at: chrono::Utc::now(),
            version: 1,
        }
    }

    // -----------------------------------------------------------------------
    // CRUD tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_create_dynamic_tool() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("my_script");
        let result = mgr.create_tool(def).await;
        assert!(result.is_ok());
        let tool_def = result.unwrap();
        assert_eq!(tool_def.tool_type, ToolType::Dynamic);

        // Verify it's in the list.
        let tools = mgr.list_tools().await;
        assert_eq!(tools.len(), 1);
    }

    #[tokio::test]
    async fn test_create_duplicate_fails() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("dup");
        mgr.create_tool(def.clone()).await.unwrap();
        let result = mgr.create_tool(def).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_dynamic_tool() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("updatable");
        mgr.create_tool(def).await.unwrap();

        let updated = DynamicToolDef {
            name: ToolName::from_string("updatable"),
            description: "updated description".into(),
            parameters: serde_json::json!({"type": "object"}),
            kind: DynamicToolKind::Script {
                interpreter: "python3".into(),
                source: "print('hello')".into(),
            },
            created_by: "test-agent".into(),
            created_at: chrono::Utc::now(),
            version: 0, // will be auto-incremented
        };

        let result = mgr.update_tool(updated).await.unwrap();
        assert_eq!(result.description, "updated description");

        // Check version was incremented.
        let tool = mgr
            .get_tool(&ToolName::from_string("updatable"))
            .await
            .unwrap();
        assert_eq!(tool.version, 2);
    }

    #[tokio::test]
    async fn test_update_nonexistent_fails() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("ghost");
        let result = mgr.update_tool(def).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_dynamic_tool() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("deletable");
        mgr.create_tool(def).await.unwrap();

        let result = mgr
            .delete_tool(&ToolName::from_string("deletable"), "test-agent")
            .await;
        assert!(result.is_ok());

        // Verify it's gone.
        assert!(mgr
            .get_tool(&ToolName::from_string("deletable"))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_fails() {
        let mgr = DynamicToolManager::new();
        let result = mgr
            .delete_tool(&ToolName::from_string("ghost"), "test-agent")
            .await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Audit trail tests (P1-9)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_audit_log_records_crud() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("audited");
        mgr.create_tool(def).await.unwrap();

        let name = ToolName::from_string("audited");
        mgr.record_execution(&name, "executor").await;
        mgr.delete_tool(&name, "admin").await.unwrap();

        let log = mgr.audit_log(None).await;
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].operation, AuditOperation::Create);
        assert_eq!(log[1].operation, AuditOperation::Execute);
        assert_eq!(log[2].operation, AuditOperation::Delete);
    }

    #[tokio::test]
    async fn test_audit_log_filter_by_tool() {
        let mgr = DynamicToolManager::new();
        mgr.create_tool(sample_script_def("tool_a")).await.unwrap();
        mgr.create_tool(sample_script_def("tool_b")).await.unwrap();

        let name_a = ToolName::from_string("tool_a");
        let filtered = mgr.audit_log(Some(&name_a)).await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tool_name, name_a);
    }

    #[tokio::test]
    async fn test_audit_update_records_version() {
        let mgr = DynamicToolManager::new();
        let def = sample_script_def("versioned");
        mgr.create_tool(def).await.unwrap();

        let updated = DynamicToolDef {
            name: ToolName::from_string("versioned"),
            description: "v2".into(),
            parameters: serde_json::json!({"type": "object"}),
            kind: DynamicToolKind::Script {
                interpreter: "bash".into(),
                source: "echo v2".into(),
            },
            created_by: "test-agent".into(),
            created_at: chrono::Utc::now(),
            version: 0,
        };
        mgr.update_tool(updated).await.unwrap();

        let log = mgr.audit_log(None).await;
        let update_entry = log
            .iter()
            .find(|e| e.operation == AuditOperation::Update)
            .unwrap();
        assert!(update_entry
            .details
            .as_deref()
            .unwrap()
            .contains("version=2"));
    }

    // -----------------------------------------------------------------------
    // Tool kind tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dynamic_tool_kind_serialization() {
        let kind = DynamicToolKind::Script {
            interpreter: "python3".into(),
            source: "print('hello')".into(),
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"type\":\"script\""));

        let kind = DynamicToolKind::HttpApi {
            method: "GET".into(),
            url: "https://api.example.com".into(),
            headers: std::collections::HashMap::new(),
            body_template: None,
        };
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"type\":\"http_api\""));
    }

    #[test]
    fn test_to_tool_definition() {
        let def = sample_script_def("test");
        let tool_def = def.to_tool_definition();
        assert_eq!(tool_def.tool_type, ToolType::Dynamic);
        assert_eq!(tool_def.category, ToolCategory::Custom);
        assert!(!tool_def.is_dangerous);
        assert!(tool_def.capabilities.process.shell);
    }

    #[test]
    fn test_composite_step_serialization() {
        let def = sample_composite_def("comp");
        let json = serde_json::to_string(&def.kind).unwrap();
        assert!(json.contains("\"type\":\"composite\""));
        assert!(json.contains("step1"));
    }

    #[tokio::test]
    async fn test_create_http_dynamic_tool_rejected_in_current_phase() {
        let mgr = DynamicToolManager::new();
        let err = mgr
            .create_tool(sample_http_def("http_tool"))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::ValidationError { .. }));
    }

    #[tokio::test]
    async fn test_update_composite_dynamic_tool_rejected_in_current_phase() {
        let mgr = DynamicToolManager::new();
        mgr.create_tool(sample_script_def("updatable"))
            .await
            .unwrap();

        let err = mgr
            .update_tool(sample_composite_def("updatable"))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::ValidationError { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_script_dynamic_tool_executes_via_command_runner() {
        let tool = DynamicToolAdapter::new(sample_script_def("script_tool"));
        let runner = StdArc::new(RecordingRunner::succeed("hello from dynamic tool"));
        let output = tool
            .execute(make_input(
                "script_tool",
                serde_json::json!({ "city": "Taipei" }),
                Some(runner.clone()),
            ))
            .await
            .unwrap();

        assert!(output.success);
        assert_eq!(output.content["exit_code"], 0);
        assert_eq!(output.content["stdout"], "hello from dynamic tool");

        let commands = runner.commands();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].contains("bash"));
        assert!(commands[0].contains("echo hello"));
        assert!(commands[0].contains("\"city\":\"Taipei\""));
    }

    #[tokio::test]
    async fn test_script_dynamic_tool_requires_command_runner() {
        let tool = DynamicToolAdapter::new(sample_script_def("script_tool"));
        let err = tool
            .execute(make_input(
                "script_tool",
                serde_json::json!({ "city": "Taipei" }),
                None,
            ))
            .await
            .unwrap_err();

        assert!(matches!(err, ToolError::RuntimeError { .. }));
    }
}
