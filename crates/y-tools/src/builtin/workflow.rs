//! Workflow and schedule management meta-tools for agent self-orchestration.
//!
//! These tools allow the LLM to create, list, get, update, delete, validate
//! workflows and manage scheduled tasks programmatically. Like the `Task` and
//! `ToolSearch` tools, the actual persistence is handled by a service-layer
//! orchestrator that intercepts these tool calls.
//!
//! ## Workflow tools
//!
//! | Tool | Purpose |
//! |------|---------|
//! | `WorkflowCreate` | Create a new workflow template |
//! | `WorkflowList` | List available workflow templates |
//! | `WorkflowGet` | Get workflow details by ID |
//! | `WorkflowUpdate` | Update an existing workflow |
//! | `WorkflowDelete` | Delete a workflow template |
//! | `WorkflowValidate` | Validate a workflow definition |
//!
//! ## Schedule tools
//!
//! | Tool | Purpose |
//! |------|---------|
//! | `ScheduleCreate` | Create a scheduled Task |
//! | `ScheduleList` | List scheduled tasks |
//! | `SchedulePause` | Pause a schedule |
//! | `ScheduleResume` | Resume a paused schedule |
//! | `ScheduleDelete` | Delete a schedule |

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

// ---------------------------------------------------------------------------
// Helper: extract required string parameter
// ---------------------------------------------------------------------------

fn require_str<'a>(input: &'a ToolInput, field: &str) -> Result<&'a str, ToolError> {
    input
        .arguments
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("'{field}' is required"),
        })
}

// ---------------------------------------------------------------------------
// WorkflowCreate
// ---------------------------------------------------------------------------

/// Tool that lets the agent create a new workflow template.
pub struct WorkflowCreateTool {
    def: ToolDefinition,
}

impl WorkflowCreateTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowCreate"),
            description: "Create a new workflow template. \
                Use Expression DSL for simple graph structure \
                (e.g. 'search >> analyze >> summarize') \
                or TOML for workflows with explicit tool bindings and parameters."
                .into(),
            help: Some(
                "Creates and persists a workflow template that can be executed or scheduled.\n\
                 \n\
                 ## Expression DSL format\n\
                 \n\
                 Task names are abstract step identifiers (letters, digits, hyphens, \
                 underscores only). They are NOT tool names or tool invocations.\n\
                 \n\
                 Operators:\n\
                 - >> : sequential (a >> b means b runs after a)\n\
                 - |  : parallel  (a | b means a and b run concurrently)\n\
                 - () : grouping  (a >> (b | c) >> d)\n\
                 \n\
                 CORRECT: \"check-disk >> fetch-weather >> generate-report\"\n\
                 WRONG:   \"ShellExec:df -h >> WebFetch:search(...)\" (parse error)\n\
                 \n\
                 ## TOML format\n\
                 \n\
                 Use TOML when you need to bind tools and parameters to steps:\n\
                 [workflow]\n\
                 name = \"my-workflow\"\n\
                 [[workflow.tasks]]\n\
                 id = \"step1\"\n\
                 name = \"Step 1\"\n\
                 type = \"tool_execution\"\n\
                 tool_name = \"ShellExec\"\n\
                 parameters = { command = \"df -h\" }\n\
                 \n\
                 ## Example (DSL)\n\
                 \n\
                 WorkflowCreate({\n\
                   \"name\": \"research-pipeline\",\n\
                   \"definition\": \"search >> (analyze | score) >> summarize\",\n\
                   \"format\": \"expression_dsl\",\n\
                   \"description\": \"Multi-source research and summarization\"\n\
                 })\n\
                 \n\
                 ## Example (TOML)\n\
                 \n\
                 WorkflowCreate({\n\
                   \"name\": \"disk-check\",\n\
                   \"definition\": \"[workflow]\\nname = \\\"disk-check\\\"\\n\
                 [[workflow.tasks]]\\nid = \\\"check\\\"\\nname = \\\"Check Disk\\\"\\n\
                 type = \\\"tool_execution\\\"\\ntool_name = \\\"ShellExec\\\"\\n\
                 parameters = { command = \\\"df -h\\\" }\",\n\
                   \"format\": \"toml\"\n\
                 })"
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique name for the workflow template"
                    },
                    "definition": {
                        "type": "string",
                        "description": "Workflow definition body. For expression_dsl: \
                            abstract step names joined by >> (sequential), | (parallel), \
                            () (grouping). Example: 'search >> (analyze | score) >> summarize'. \
                            Task names must only contain letters, digits, hyphens, underscores. \
                            For toml: full TOML content starting with [workflow]."
                    },
                    "format": {
                        "type": "string",
                        "enum": ["expression_dsl", "toml"],
                        "description": "Format of the definition"
                    },
                    "description": {
                        "type": "string",
                        "description": "Human-readable description of what this workflow does"
                    },
                    "tags": {
                        "type": "string",
                        "description": "Comma-separated tags for filtering"
                    }
                },
                "required": ["name", "definition"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for WorkflowCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowCreateTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let name = require_str(&input, "name")?;
        let definition = require_str(&input, "definition")?;

        let format = input
            .arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("expression_dsl");
        let description = input.arguments.get("description").and_then(|v| v.as_str());
        let tags = input.arguments.get("tags").and_then(|v| v.as_str());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowCreate",
                "name": name,
                "definition": definition,
                "format": format,
                "description": description,
                "tags": tags,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// WorkflowList
// ---------------------------------------------------------------------------

/// Tool that lets the agent list available workflow templates.
pub struct WorkflowListTool {
    def: ToolDefinition,
}

impl WorkflowListTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowList"),
            description: "List available workflow templates. \
                Optionally filter by tag."
                .into(),
            help: Some(
                "Returns available workflow templates for execution or scheduling.\n\
                 \n\
                 Example:\n\
                 WorkflowList({}) -- list all\n\
                 WorkflowList({\"tag\": \"research\"}) -- filter by tag"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "tag": {
                        "type": "string",
                        "description": "Optional tag to filter workflows"
                    }
                }
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for WorkflowListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowListTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let tag = input.arguments.get("tag").and_then(|v| v.as_str());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowList",
                "tag": tag,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// WorkflowGet
// ---------------------------------------------------------------------------

/// Tool that lets the agent get details of a specific workflow template.
pub struct WorkflowGetTool {
    def: ToolDefinition,
}

impl WorkflowGetTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowGet"),
            description: "Get details of a workflow template by ID, \
                including its definition, format, and metadata."
                .into(),
            help: Some(
                "Returns the full details of a workflow template.\n\
                 \n\
                 Example:\n\
                 WorkflowGet({\"id\": \"wf-abc123\"})"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The workflow template ID"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for WorkflowGetTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowGetTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowGet",
                "id": id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// WorkflowUpdate
// ---------------------------------------------------------------------------

/// Tool that lets the agent update an existing workflow template.
pub struct WorkflowUpdateTool {
    def: ToolDefinition,
}

impl WorkflowUpdateTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowUpdate"),
            description: "Update an existing workflow template. \
                Any provided field will overwrite the existing value."
                .into(),
            help: Some(
                "Updates a workflow template in place.\n\
                 \n\
                 Example:\n\
                 WorkflowUpdate({\n                   \"id\": \"wf-abc123\",\n\
                   \"definition\": \"search >> analyze >> report\",\n\
                   \"description\": \"Updated pipeline\"\n\
                 })"
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The workflow template ID to update"
                    },
                    "definition": {
                        "type": "string",
                        "description": "New workflow definition body"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["expression_dsl", "toml"],
                        "description": "New format of the definition"
                    },
                    "description": {
                        "type": "string",
                        "description": "New description"
                    },
                    "tags": {
                        "type": "string",
                        "description": "New comma-separated tags"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for WorkflowUpdateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowUpdateTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        let definition = input.arguments.get("definition").and_then(|v| v.as_str());
        let format = input.arguments.get("format").and_then(|v| v.as_str());
        let description = input.arguments.get("description").and_then(|v| v.as_str());
        let tags = input.arguments.get("tags").and_then(|v| v.as_str());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowUpdate",
                "id": id,
                "definition": definition,
                "format": format,
                "description": description,
                "tags": tags,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// WorkflowDelete
// ---------------------------------------------------------------------------

/// Tool that lets the agent delete a workflow template.
pub struct WorkflowDeleteTool {
    def: ToolDefinition,
}

impl WorkflowDeleteTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowDelete"),
            description: "Delete a workflow template by ID. \
                This also removes any schedules linked to this workflow."
                .into(),
            help: Some(
                "Permanently deletes a workflow template.\n\
                 \n\
                 Example:\n\
                 WorkflowDelete({\"id\": \"wf-abc123\"})"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The workflow template ID to delete"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true,
        }
    }
}

impl Default for WorkflowDeleteTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowDeleteTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowDelete",
                "id": id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// WorkflowValidate
// ---------------------------------------------------------------------------

/// Tool that lets the agent validate a workflow definition without saving it.
pub struct WorkflowValidateTool {
    def: ToolDefinition,
}

impl WorkflowValidateTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WorkflowValidate"),
            description: "Validate a workflow definition without saving it. \
                Returns validation results including any errors."
                .into(),
            help: Some(
                "Checks a workflow definition for correctness.\n\
                 \n\
                 Example:\n\
                 WorkflowValidate({\n\
                   \"definition\": \"search >> analyze >> summarize\",\n\
                   \"format\": \"expression_dsl\"\n\
                 })"
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "definition": {
                        "type": "string",
                        "description": "The workflow definition to validate"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["expression_dsl", "toml"],
                        "description": "Format of the definition"
                    }
                },
                "required": ["definition"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for WorkflowValidateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkflowValidateTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let definition = require_str(&input, "definition")?;
        let format = input
            .arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("expression_dsl");

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowValidate",
                "definition": definition,
                "format": format,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// ScheduleCreate
// ---------------------------------------------------------------------------

/// Tool that lets the agent create a scheduled Task.
pub struct ScheduleCreateTool {
    def: ToolDefinition,
}

impl ScheduleCreateTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ScheduleCreate"),
            description: "Create a scheduled Task that runs a workflow on a trigger. \
                Supports cron expressions, fixed intervals, and one-time delays."
                .into(),
            help: Some(
                "Creates a scheduled Task linked to an existing workflow.\n\
                 \n\
                 Trigger types:\n\
                 - cron: cron expression (e.g. '0 9 * * 1-5' for weekdays at 9am)\n\
                 - interval: seconds between runs (e.g. 3600 for hourly)\n\
                 - onetime: seconds from now until execution\n\
                 \n\
                 Example:\n\
                 ScheduleCreate({\n\
                   \"name\": \"daily-research\",\n\
                   \"trigger_type\": \"cron\",\n\
                   \"trigger_value\": \"0 9 * * *\",\n\
                   \"workflow_id\": \"wf-123\"\n\
                 })"
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Human-readable name for the schedule"
                    },
                    "trigger_type": {
                        "type": "string",
                        "enum": ["cron", "interval", "onetime"],
                        "description": "Type of trigger"
                    },
                    "trigger_value": {
                        "type": "string",
                        "description": "Cron expression, interval in seconds, or delay in seconds"
                    },
                    "workflow_id": {
                        "type": "string",
                        "description": "ID of the workflow template to execute"
                    },
                    "description": {
                        "type": "string",
                        "description": "Human-readable description"
                    }
                },
                "required": ["name", "trigger_type", "trigger_value", "workflow_id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for ScheduleCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let name = require_str(&input, "name")?;
        let trigger_type = require_str(&input, "trigger_type")?;
        let trigger_value = require_str(&input, "trigger_value")?;
        let workflow_id = require_str(&input, "workflow_id")?;
        let description = input.arguments.get("description").and_then(|v| v.as_str());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "ScheduleCreate",
                "name": name,
                "trigger_type": trigger_type,
                "trigger_value": trigger_value,
                "workflow_id": workflow_id,
                "description": description,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// ScheduleList
// ---------------------------------------------------------------------------

/// Tool that lets the agent list scheduled tasks.
pub struct ScheduleListTool {
    def: ToolDefinition,
}

impl ScheduleListTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ScheduleList"),
            description: "List scheduled tasks. \
                Optionally filter by workflow ID."
                .into(),
            help: Some(
                "Returns all scheduled tasks, with optional filtering.\n\
                 \n\
                 Example:\n\
                 ScheduleList({}) -- list all\n\
                 ScheduleList({\"workflow_id\": \"wf-123\"}) -- filter by workflow"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "workflow_id": {
                        "type": "string",
                        "description": "Optional workflow ID to filter schedules"
                    }
                }
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for ScheduleListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScheduleListTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let workflow_id = input.arguments.get("workflow_id").and_then(|v| v.as_str());

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "ScheduleList",
                "workflow_id": workflow_id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// SchedulePause
// ---------------------------------------------------------------------------

/// Tool that lets the agent pause a scheduled Task.
pub struct SchedulePauseTool {
    def: ToolDefinition,
}

impl SchedulePauseTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("SchedulePause"),
            description: "Pause a scheduled Task. The schedule retains its \
                configuration and can be resumed later."
                .into(),
            help: Some(
                "Pauses a schedule without deleting it.\n\
                 \n\
                 Example:\n\
                 SchedulePause({\"id\": \"sched-abc123\"})"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The schedule ID to pause"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for SchedulePauseTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SchedulePauseTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "SchedulePause",
                "id": id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// ScheduleResume
// ---------------------------------------------------------------------------

/// Tool that lets the agent resume a paused schedule.
pub struct ScheduleResumeTool {
    def: ToolDefinition,
}

impl ScheduleResumeTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ScheduleResume"),
            description: "Resume a previously paused schedule.".into(),
            help: Some(
                "Resumes a paused schedule so it fires on its next trigger.\n\
                 \n\
                 Example:\n\
                 ScheduleResume({\"id\": \"sched-abc123\"})"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The schedule ID to resume"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for ScheduleResumeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScheduleResumeTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "ScheduleResume",
                "id": id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// ScheduleDelete
// ---------------------------------------------------------------------------

/// Tool that lets the agent delete a scheduled Task.
pub struct ScheduleDeleteTool {
    def: ToolDefinition,
}

impl ScheduleDeleteTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ScheduleDelete"),
            description: "Delete a scheduled Task by ID. \
                This permanently removes the schedule."
                .into(),
            help: Some(
                "Permanently deletes a schedule.\n\
                 \n\
                 Example:\n\
                 ScheduleDelete({\"id\": \"sched-abc123\"})"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The schedule ID to delete"
                    }
                },
                "required": ["id"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true,
        }
    }
}

impl Default for ScheduleDeleteTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ScheduleDeleteTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let id = require_str(&input, "id")?;
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "ScheduleDelete",
                "id": id,
                "status": "pending"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use y_core::types::SessionId;

    use super::*;

    fn make_input(name: &str, args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string(name),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    // -- WorkflowCreate --

    #[tokio::test]
    async fn test_workflow_create_valid() {
        let tool = WorkflowCreateTool::new();
        let input = make_input(
            "WorkflowCreate",
            serde_json::json!({
                "name": "research-pipeline",
                "definition": "search >> analyze >> summarize",
                "format": "expression_dsl"
            }),
        );
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowCreate");
        assert_eq!(output.content["name"], "research-pipeline");
    }

    #[tokio::test]
    async fn test_workflow_create_missing_name() {
        let tool = WorkflowCreateTool::new();
        let input = make_input(
            "WorkflowCreate",
            serde_json::json!({"definition": "a >> b"}),
        );
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    // -- WorkflowList --

    #[tokio::test]
    async fn test_workflow_list_no_args() {
        let tool = WorkflowListTool::new();
        let input = make_input("WorkflowList", serde_json::json!({}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowList");
    }

    // -- WorkflowGet --

    #[tokio::test]
    async fn test_workflow_get_valid() {
        let tool = WorkflowGetTool::new();
        let input = make_input("WorkflowGet", serde_json::json!({"id": "wf-123"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowGet");
        assert_eq!(output.content["id"], "wf-123");
    }

    #[tokio::test]
    async fn test_workflow_get_missing_id() {
        let tool = WorkflowGetTool::new();
        let input = make_input("WorkflowGet", serde_json::json!({}));
        assert!(tool.execute(input).await.is_err());
    }

    // -- WorkflowUpdate --

    #[tokio::test]
    async fn test_workflow_update_valid() {
        let tool = WorkflowUpdateTool::new();
        let input = make_input(
            "WorkflowUpdate",
            serde_json::json!({
                "id": "wf-123",
                "definition": "a >> b >> c",
                "description": "updated"
            }),
        );
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowUpdate");
        assert_eq!(output.content["id"], "wf-123");
    }

    #[tokio::test]
    async fn test_workflow_update_missing_id() {
        let tool = WorkflowUpdateTool::new();
        let input = make_input(
            "WorkflowUpdate",
            serde_json::json!({"definition": "a >> b"}),
        );
        assert!(tool.execute(input).await.is_err());
    }

    // -- WorkflowDelete --

    #[tokio::test]
    async fn test_workflow_delete_valid() {
        let tool = WorkflowDeleteTool::new();
        let input = make_input("WorkflowDelete", serde_json::json!({"id": "wf-123"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowDelete");
    }

    #[tokio::test]
    async fn test_workflow_delete_is_dangerous() {
        let def = WorkflowDeleteTool::tool_definition();
        assert!(def.is_dangerous);
    }

    // -- WorkflowValidate --

    #[tokio::test]
    async fn test_workflow_validate_valid() {
        let tool = WorkflowValidateTool::new();
        let input = make_input(
            "WorkflowValidate",
            serde_json::json!({
                "definition": "search >> analyze",
                "format": "expression_dsl"
            }),
        );
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "WorkflowValidate");
    }

    #[tokio::test]
    async fn test_workflow_validate_missing_definition() {
        let tool = WorkflowValidateTool::new();
        let input = make_input("WorkflowValidate", serde_json::json!({}));
        assert!(tool.execute(input).await.is_err());
    }

    // -- ScheduleCreate --

    #[tokio::test]
    async fn test_schedule_create_valid() {
        let tool = ScheduleCreateTool::new();
        let input = make_input(
            "ScheduleCreate",
            serde_json::json!({
                "name": "daily-research",
                "trigger_type": "cron",
                "trigger_value": "0 9 * * *",
                "workflow_id": "wf-123"
            }),
        );
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "ScheduleCreate");
    }

    #[tokio::test]
    async fn test_schedule_create_missing_workflow_id() {
        let tool = ScheduleCreateTool::new();
        let input = make_input(
            "ScheduleCreate",
            serde_json::json!({
                "name": "test",
                "trigger_type": "interval",
                "trigger_value": "3600"
            }),
        );
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }

    // -- ScheduleList --

    #[tokio::test]
    async fn test_schedule_list_no_args() {
        let tool = ScheduleListTool::new();
        let input = make_input("ScheduleList", serde_json::json!({}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "ScheduleList");
    }

    #[tokio::test]
    async fn test_schedule_list_with_filter() {
        let tool = ScheduleListTool::new();
        let input = make_input("ScheduleList", serde_json::json!({"workflow_id": "wf-123"}));
        let output = tool.execute(input).await.unwrap();
        assert_eq!(output.content["workflow_id"], "wf-123");
    }

    // -- SchedulePause --

    #[tokio::test]
    async fn test_schedule_pause_valid() {
        let tool = SchedulePauseTool::new();
        let input = make_input("SchedulePause", serde_json::json!({"id": "sched-123"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "SchedulePause");
    }

    #[tokio::test]
    async fn test_schedule_pause_missing_id() {
        let tool = SchedulePauseTool::new();
        let input = make_input("SchedulePause", serde_json::json!({}));
        assert!(tool.execute(input).await.is_err());
    }

    // -- ScheduleResume --

    #[tokio::test]
    async fn test_schedule_resume_valid() {
        let tool = ScheduleResumeTool::new();
        let input = make_input("ScheduleResume", serde_json::json!({"id": "sched-123"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "ScheduleResume");
    }

    // -- ScheduleDelete --

    #[tokio::test]
    async fn test_schedule_delete_valid() {
        let tool = ScheduleDeleteTool::new();
        let input = make_input("ScheduleDelete", serde_json::json!({"id": "sched-123"}));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "ScheduleDelete");
    }

    #[tokio::test]
    async fn test_schedule_delete_is_dangerous() {
        let def = ScheduleDeleteTool::tool_definition();
        assert!(def.is_dangerous);
    }

    // -- definition checks --

    #[test]
    fn test_all_tool_definitions() {
        let tools: Vec<(&str, ToolDefinition)> = vec![
            ("WorkflowCreate", WorkflowCreateTool::tool_definition()),
            ("WorkflowList", WorkflowListTool::tool_definition()),
            ("WorkflowGet", WorkflowGetTool::tool_definition()),
            ("WorkflowUpdate", WorkflowUpdateTool::tool_definition()),
            ("WorkflowDelete", WorkflowDeleteTool::tool_definition()),
            ("WorkflowValidate", WorkflowValidateTool::tool_definition()),
            ("ScheduleCreate", ScheduleCreateTool::tool_definition()),
            ("ScheduleList", ScheduleListTool::tool_definition()),
            ("SchedulePause", SchedulePauseTool::tool_definition()),
            ("ScheduleResume", ScheduleResumeTool::tool_definition()),
            ("ScheduleDelete", ScheduleDeleteTool::tool_definition()),
        ];
        for (expected_name, def) in tools {
            assert_eq!(def.name.as_str(), expected_name);
            assert_eq!(def.category, ToolCategory::Agent);
        }
    }
}
