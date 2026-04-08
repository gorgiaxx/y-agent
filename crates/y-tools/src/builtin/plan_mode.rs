//! Plan mode tools: `EnterPlanMode`, `PlanWriter`, `ExitPlanMode`.
//!
//! These tools enable the LLM to autonomously enter a structured planning
//! phase for complex tasks. The agent decides when planning is warranted
//! based on task complexity.
//!
//! - `EnterPlanMode` -- signal tool; the orchestrator intercepts it and
//!   transitions the execution context to read-only exploration mode.
//! - `PlanWriter` -- writes the plan markdown to `~/.y-agent/plan/` with
//!   collision-avoidant filename generation.
//! - `ExitPlanMode` -- signal tool; the orchestrator intercepts it and
//!   triggers sequential phase execution.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

// ---------------------------------------------------------------------------
// Plan directory utilities
// ---------------------------------------------------------------------------

/// Default plan storage directory: `~/.y-agent/plan/`.
pub fn plan_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".y-agent").join("plan")
}

/// Generate a filesystem-safe slug from a title string.
///
/// Converts to lowercase, replaces non-alphanumeric characters with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive hyphens and trim.
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.trim_matches('-').to_string()
}

/// Generate a deduplicated plan file path.
///
/// Tries `<dir>/<slug>.md` first, then `<slug>-1.md`, `<slug>-2.md`, etc.
/// Returns the first path that does not exist on disk.
fn generate_plan_path(dir: &Path, title: &str) -> PathBuf {
    let slug = slugify(title);
    let slug = if slug.is_empty() {
        "untitled-plan".to_string()
    } else {
        slug
    };

    let base = dir.join(format!("{slug}.md"));
    if !base.exists() {
        return base;
    }

    for i in 1..1000 {
        let candidate = dir.join(format!("{slug}-{i}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }

    // Extremely unlikely fallback.
    dir.join(format!("{slug}-{}.md", uuid::Uuid::new_v4()))
}

// ---------------------------------------------------------------------------
// EnterPlanModeTool
// ---------------------------------------------------------------------------

/// Signal tool: the LLM calls this when it autonomously decides a task
/// requires structured planning before execution.
///
/// The tool itself does nothing -- the `PlanModeOrchestrator` in `y-service`
/// intercepts the call and transitions the execution context.
pub struct EnterPlanModeTool {
    def: ToolDefinition,
}

impl EnterPlanModeTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("EnterPlanMode"),
            description: "Enter plan mode for complex tasks that require structured \
                investigation and phased execution. Call this BEFORE making changes \
                when the task involves multiple files, architectural decisions, or \
                multi-step workflows. In plan mode you can only explore (read files, \
                search code) -- no modifications allowed."
                .into(),
            help: Some(
                "Triggers a read-only exploration phase. After investigating, \
                 write a plan with PlanWriter, then call ExitPlanMode to begin \
                 phased execution.\n\
                 \n\
                 When to use:\n\
                 - Task affects 3+ files across different modules\n\
                 - Task requires understanding existing architecture before changes\n\
                 - Task involves refactoring, migration, or redesign\n\
                 - Task has ambiguous requirements needing investigation\n\
                 - Task failure would be costly to undo"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why this task warrants structured planning"
                    },
                    "title": {
                        "type": "string",
                        "description": "Short descriptive title for the plan"
                    }
                },
                "required": ["reason", "title"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for EnterPlanModeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EnterPlanModeTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let reason = input
            .arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'reason' is required".into(),
            })?;

        let title = input
            .arguments
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'title' is required".into(),
            })?;

        // The orchestrator intercepts this before execute() is called in
        // normal flow. If we reach here, return a descriptor for the caller.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "enter_plan_mode",
                "reason": reason,
                "title": title,
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
// PlanWriterTool
// ---------------------------------------------------------------------------

/// Writes a structured plan to `~/.y-agent/plan/<slug>.md`.
///
/// Unlike `EnterPlanMode` and `ExitPlanMode`, this tool performs actual I/O.
/// It generates a collision-free filename from the title and writes the
/// markdown content to disk.
pub struct PlanWriterTool {
    def: ToolDefinition,
}

impl PlanWriterTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("PlanWriter"),
            description: "Write an execution plan to disk. The plan must include \
                an Overview section and numbered Phases, each with Objective, \
                Key Files, Steps, and Acceptance Criteria."
                .into(),
            help: Some(
                "Creates a markdown plan file at ~/.y-agent/plan/<slug>.md.\n\
                 Filename is generated from the title with collision avoidance.\n\
                 \n\
                 The content must be valid markdown with:\n\
                 - YAML frontmatter (title, status, total_phases)\n\
                 - ## Overview section\n\
                 - ## Phase N: <title> sections with subsections"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Plan title (used for filename generation)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full markdown content of the plan including \
                            YAML frontmatter and all Phase sections"
                    }
                },
                "required": ["title", "content"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for PlanWriterTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PlanWriterTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let title = input
            .arguments
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'title' is required".into(),
            })?;

        let content = input
            .arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'content' is required".into(),
            })?;

        let dir = plan_dir();

        // Ensure directory exists.
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            return Err(ToolError::Other {
                message: format!("failed to create plan directory {}: {e}", dir.display()),
            });
        }

        let path = generate_plan_path(&dir, title);

        if let Err(e) = tokio::fs::write(&path, content).await {
            return Err(ToolError::Other {
                message: format!("failed to write plan file {}: {e}", path.display()),
            });
        }

        tracing::info!(
            path = %path.display(),
            title = %title,
            "plan file written"
        );

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "plan_written",
                "path": path.display().to_string(),
                "title": title,
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
// ExitPlanModeTool
// ---------------------------------------------------------------------------

/// Signal tool: called by the LLM after writing a plan to indicate it is
/// ready for phased execution.
///
/// The `PlanModeOrchestrator` intercepts this call, parses the plan file,
/// and begins sequential phase execution.
pub struct ExitPlanModeTool {
    def: ToolDefinition,
}

impl ExitPlanModeTool {
    pub fn new() -> Self {
        Self {
            def: Self::tool_definition(),
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("ExitPlanMode"),
            description: "Exit plan mode and begin phased execution. Call this \
                after writing a plan with PlanWriter. The system will execute \
                each phase sequentially."
                .into(),
            help: Some(
                "Signals that the plan is complete and ready for execution.\n\
                 Requires the path to the plan file written by PlanWriter.\n\
                 \n\
                 The system will:\n\
                 1. Parse the plan file into phases\n\
                 2. Execute each phase as a separate sub-agent run\n\
                 3. Update the plan file with phase status\n\
                 4. Return a consolidated summary"
                    .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "plan_file": {
                        "type": "string",
                        "description": "Absolute path to the plan file written by PlanWriter"
                    },
                    "total_phases": {
                        "type": "integer",
                        "description": "Total number of phases in the plan"
                    }
                },
                "required": ["plan_file", "total_phases"]
            }),
            result_schema: None,
            category: ToolCategory::Agent,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }
}

impl Default for ExitPlanModeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let plan_file = input
            .arguments
            .get("plan_file")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'plan_file' is required".into(),
            })?;

        let total_phases = input
            .arguments
            .get("total_phases")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ToolError::ValidationError {
                message: "'total_phases' is required and must be a positive integer".into(),
            })?;

        // Validate the plan file exists.
        if !Path::new(plan_file).exists() {
            return Err(ToolError::ValidationError {
                message: format!("plan file does not exist: {plan_file}"),
            });
        }

        // The orchestrator intercepts this before execute() is called in
        // normal flow. If we reach here, return a descriptor.
        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "exit_plan_mode",
                "plan_file": plan_file,
                "total_phases": total_phases,
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

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_plan".into(),
            name: ToolName::from_string("test"),
            arguments: args,
            session_id: SessionId::new(),
            command_runner: None,
        }
    }

    // -- slugify --

    #[test]
    fn test_slugify_basic() {
        assert_eq!(
            slugify("Refactor Skill Ingestion"),
            "refactor-skill-ingestion"
        );
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("fix: parser (v2) -- final!"), "fix-parser-v2-final");
    }

    #[test]
    fn test_slugify_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn test_slugify_all_special() {
        assert_eq!(slugify("---!!!---"), "");
    }

    // -- generate_plan_path --

    #[test]
    fn test_generate_plan_path_no_collision() {
        let dir = std::env::temp_dir().join("y-agent-test-plan-no-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = generate_plan_path(&dir, "My Test Plan");
        assert_eq!(path.file_name().unwrap(), "my-test-plan.md");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_plan_path_with_collision() {
        let dir = std::env::temp_dir().join("y-agent-test-plan-collision");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create an existing file to trigger collision.
        std::fs::write(dir.join("my-plan.md"), "existing").unwrap();

        let path = generate_plan_path(&dir, "My Plan");
        assert_eq!(path.file_name().unwrap(), "my-plan-1.md");

        // Two collisions.
        std::fs::write(dir.join("my-plan-1.md"), "existing").unwrap();
        let path2 = generate_plan_path(&dir, "My Plan");
        assert_eq!(path2.file_name().unwrap(), "my-plan-2.md");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_generate_plan_path_empty_title() {
        let dir = std::env::temp_dir().join("y-agent-test-plan-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = generate_plan_path(&dir, "");
        assert_eq!(path.file_name().unwrap(), "untitled-plan.md");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- EnterPlanModeTool --

    #[tokio::test]
    async fn test_enter_plan_mode_valid() {
        let tool = EnterPlanModeTool::new();
        let input = make_input(serde_json::json!({
            "reason": "Task affects 5 files across 3 modules",
            "title": "Refactor skill parser"
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "enter_plan_mode");
        assert_eq!(output.content["status"], "pending");
    }

    #[tokio::test]
    async fn test_enter_plan_mode_missing_reason() {
        let tool = EnterPlanModeTool::new();
        let input = make_input(serde_json::json!({"title": "test"}));
        assert!(tool.execute(input).await.is_err());
    }

    #[tokio::test]
    async fn test_enter_plan_mode_missing_title() {
        let tool = EnterPlanModeTool::new();
        let input = make_input(serde_json::json!({"reason": "complex"}));
        assert!(tool.execute(input).await.is_err());
    }

    // -- PlanWriterTool --

    #[tokio::test]
    async fn test_plan_writer_writes_file() {
        let dir = std::env::temp_dir().join("y-agent-test-plan-writer");
        let _ = std::fs::remove_dir_all(&dir);

        // Override plan_dir by writing directly to a known path.
        let tool = PlanWriterTool::new();
        let content = "---\ntitle: Test\nstatus: pending\n---\n## Overview\nA test plan.";
        let input = make_input(serde_json::json!({
            "title": "Test Plan",
            "content": content
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "plan_written");

        let path_str = output.content["path"].as_str().unwrap();
        let written = std::fs::read_to_string(path_str).unwrap();
        assert!(written.contains("## Overview"));

        // Cleanup.
        let _ = std::fs::remove_file(path_str);
    }

    #[tokio::test]
    async fn test_plan_writer_missing_content() {
        let tool = PlanWriterTool::new();
        let input = make_input(serde_json::json!({"title": "No Content"}));
        assert!(tool.execute(input).await.is_err());
    }

    // -- ExitPlanModeTool --

    #[tokio::test]
    async fn test_exit_plan_mode_valid() {
        // Create a temp plan file for validation.
        let path = std::env::temp_dir().join("y-agent-test-exit-plan.md");
        std::fs::write(&path, "plan content").unwrap();

        let tool = ExitPlanModeTool::new();
        let input = make_input(serde_json::json!({
            "plan_file": path.display().to_string(),
            "total_phases": 3
        }));
        let output = tool.execute(input).await.unwrap();
        assert!(output.success);
        assert_eq!(output.content["action"], "exit_plan_mode");
        assert_eq!(output.content["total_phases"], 3);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn test_exit_plan_mode_missing_file() {
        let tool = ExitPlanModeTool::new();
        let input = make_input(serde_json::json!({
            "plan_file": "/nonexistent/plan.md",
            "total_phases": 1
        }));
        assert!(tool.execute(input).await.is_err());
    }

    // -- Definitions --

    #[test]
    fn test_enter_plan_mode_definition() {
        let def = EnterPlanModeTool::tool_definition();
        assert_eq!(def.name.as_str(), "EnterPlanMode");
        assert_eq!(def.category, ToolCategory::Agent);
        assert!(!def.is_dangerous);
    }

    #[test]
    fn test_plan_writer_definition() {
        let def = PlanWriterTool::tool_definition();
        assert_eq!(def.name.as_str(), "PlanWriter");
        assert_eq!(def.category, ToolCategory::Agent);
    }

    #[test]
    fn test_exit_plan_mode_definition() {
        let def = ExitPlanModeTool::tool_definition();
        assert_eq!(def.name.as_str(), "ExitPlanMode");
        assert_eq!(def.category, ToolCategory::Agent);
    }
}
