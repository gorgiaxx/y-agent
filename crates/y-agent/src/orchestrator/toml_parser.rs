//! TOML workflow parser: complex workflow definitions via configuration.
//!
//! Design reference: orchestrator-design.md, Dual-Format Workflow Definition
//!
//! Workflows can be defined in two formats:
//! - **Expression DSL**: compact inline syntax (`search >> (analyze | score) >> summarize`)
//! - **TOML configuration**: detailed structured definitions with full I/O mapping
//!
//! This module handles the TOML format, parsing it into the same internal
//! representation (`TaskDag` + mappings) as the Expression DSL.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::orchestrator::dag::{TaskDag, TaskId, TaskNode, TaskPriority, TaskType};
use crate::orchestrator::failure::{FailureStrategy, RetryConfig};
use crate::orchestrator::io_mapping::{InputMapping, OutputMapping};

// ---------------------------------------------------------------------------
// TOML schema types
// ---------------------------------------------------------------------------

/// Top-level TOML workflow document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlWorkflow {
    /// Workflow metadata.
    pub workflow: WorkflowMeta,
}

/// Workflow metadata section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMeta {
    /// Unique name / ID.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Tags for categorisation.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Task definitions.
    pub tasks: Vec<TomlTask>,
}

/// A single task definition in TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TomlTask {
    /// Unique task ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Task type and its configuration.
    #[serde(flatten)]
    pub task_type: TomlTaskType,
    /// IDs of tasks this depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Priority level.
    #[serde(default)]
    pub priority: Option<TaskPriority>,
    /// Timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Retry configuration.
    #[serde(default)]
    pub retry: Option<RetryConfig>,
    /// Failure strategy.
    #[serde(default)]
    pub failure_strategy: Option<FailureStrategy>,
    /// Input mappings: `field_name` -> mapping.
    #[serde(default)]
    pub inputs: HashMap<String, InputMapping>,
    /// Output mappings.
    #[serde(default)]
    pub outputs: Vec<OutputMapping>,
}

/// TOML-friendly task type (tagged via `type` field).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TomlTaskType {
    LlmCall {
        #[serde(default)]
        provider_tag: Option<String>,
        #[serde(default)]
        system_prompt: Option<String>,
    },
    ToolExecution {
        tool_name: String,
        #[serde(default)]
        parameters: serde_json::Value,
    },
    SubAgent {
        agent_id: String,
    },
    SubWorkflow {
        workflow_id: String,
    },
    Script {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    HumanApproval,
    Noop,
}

// ---------------------------------------------------------------------------
// Parse result
// ---------------------------------------------------------------------------

/// Result of parsing a TOML workflow.
#[derive(Debug)]
pub struct ParsedWorkflow {
    /// The compiled task DAG.
    pub dag: TaskDag,
    /// Input mappings per task: `task_id` -> \[`(field_name, mapping)`\].
    pub input_mappings: HashMap<TaskId, Vec<(String, InputMapping)>>,
    /// Output mappings per task: `task_id` -> \[`mapping`\].
    pub output_mappings: HashMap<TaskId, Vec<OutputMapping>>,
    /// Workflow name.
    pub name: String,
    /// Workflow description.
    pub description: String,
    /// Workflow tags.
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parse errors
// ---------------------------------------------------------------------------

/// Error during TOML workflow parsing.
#[derive(Debug, thiserror::Error)]
pub enum TomlParseError {
    #[error("TOML syntax error: {0}")]
    Syntax(String),

    #[error("DAG error: {0}")]
    Dag(#[from] crate::orchestrator::dag::DagError),

    #[error("validation error: {message}")]
    Validation { message: String },
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a TOML string into a `ParsedWorkflow`.
pub fn parse_toml_workflow(input: &str) -> Result<ParsedWorkflow, TomlParseError> {
    let toml_wf: TomlWorkflow =
        toml::from_str(input).map_err(|e| TomlParseError::Syntax(e.to_string()))?;

    let meta = &toml_wf.workflow;

    if meta.tasks.is_empty() {
        return Err(TomlParseError::Validation {
            message: "workflow must contain at least one task".into(),
        });
    }

    let mut dag = TaskDag::new();
    let mut input_mappings: HashMap<TaskId, Vec<(String, InputMapping)>> = HashMap::new();
    let mut output_mappings: HashMap<TaskId, Vec<OutputMapping>> = HashMap::new();

    for t in &meta.tasks {
        let task_type = convert_task_type(&t.task_type);

        let node = TaskNode {
            id: t.id.clone(),
            name: t.name.clone(),
            priority: t.priority.unwrap_or_default(),
            dependencies: t.depends_on.clone(),
            task_type,
            timeout_ms: t.timeout_ms,
            retry: t.retry.clone(),
            failure_strategy: t.failure_strategy.clone().unwrap_or_default(),
        };

        dag.add_task(node)?;

        // Collect input mappings.
        if !t.inputs.is_empty() {
            let maps: Vec<(String, InputMapping)> = t
                .inputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            input_mappings.insert(t.id.clone(), maps);
        }

        // Collect output mappings.
        if !t.outputs.is_empty() {
            output_mappings.insert(t.id.clone(), t.outputs.clone());
        }
    }

    dag.validate()?;

    Ok(ParsedWorkflow {
        dag,
        input_mappings,
        output_mappings,
        name: meta.name.clone(),
        description: meta.description.clone(),
        tags: meta.tags.clone(),
    })
}

fn convert_task_type(tt: &TomlTaskType) -> TaskType {
    match tt {
        TomlTaskType::LlmCall {
            provider_tag,
            system_prompt,
        } => TaskType::LlmCall {
            provider_tag: provider_tag.clone(),
            system_prompt: system_prompt.clone(),
        },
        TomlTaskType::ToolExecution {
            tool_name,
            parameters,
        } => TaskType::ToolExecution {
            tool_name: tool_name.clone(),
            parameters: parameters.clone(),
        },
        TomlTaskType::SubAgent { agent_id } => TaskType::SubAgent {
            agent_id: agent_id.clone(),
        },
        TomlTaskType::SubWorkflow { workflow_id } => TaskType::SubWorkflow {
            workflow_id: workflow_id.clone(),
        },
        TomlTaskType::Script { command, args } => TaskType::Script {
            command: command.clone(),
            args: args.clone(),
        },
        TomlTaskType::HumanApproval => TaskType::HumanApproval,
        TomlTaskType::Noop => TaskType::Noop,
    }
}

// ---------------------------------------------------------------------------
// Unified WorkflowDefinition
// ---------------------------------------------------------------------------

/// Dual-format workflow definition: Expression DSL or TOML.
#[derive(Debug, Clone)]
pub enum WorkflowDefinition {
    /// Compact DSL expression (e.g. `search >> (analyze | score) >> summarize`).
    Expression(String),
    /// Full TOML configuration string.
    Toml(String),
}

impl WorkflowDefinition {
    /// Parse into a `ParsedWorkflow`.
    ///
    /// For `Expression` variants, input/output mappings will be empty.
    pub fn parse(&self) -> Result<ParsedWorkflow, TomlParseError> {
        match self {
            Self::Toml(toml_str) => parse_toml_workflow(toml_str),
            Self::Expression(expr) => {
                let ast = crate::orchestrator::expression_dsl::parse(expr)
                    .map_err(|e| TomlParseError::Syntax(e.to_string()))?;
                let dag = ast
                    .to_task_dag()
                    .map_err(|e| TomlParseError::Syntax(e.to_string()))?;

                Ok(ParsedWorkflow {
                    dag,
                    input_mappings: HashMap::new(),
                    output_mappings: HashMap::new(),
                    name: String::new(),
                    description: String::new(),
                    tags: Vec::new(),
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P4-01: Parse simple TOML workflow with 2 sequential tasks.
    #[test]
    fn test_parse_simple_sequential() {
        let toml = r#"
[workflow]
name = "two-step"
description = "Simple sequential workflow"

[[workflow.tasks]]
id = "step1"
name = "First Step"
type = "noop"

[[workflow.tasks]]
id = "step2"
name = "Second Step"
type = "noop"
depends_on = ["step1"]
"#;

        let result = parse_toml_workflow(toml).unwrap();
        assert_eq!(result.name, "two-step");
        assert_eq!(result.description, "Simple sequential workflow");

        let order = result.dag.topological_order().unwrap();
        assert_eq!(order[0], "step1");
        assert_eq!(order[1], "step2");
    }

    /// T-P4-02: Parse TOML workflow with parallel tasks.
    #[test]
    fn test_parse_parallel_tasks() {
        let toml = r#"
[workflow]
name = "parallel"

[[workflow.tasks]]
id = "search"
name = "Search"
type = "tool_execution"
tool_name = "web_search"

[[workflow.tasks]]
id = "analyze"
name = "Analyze"
type = "llm_call"
depends_on = ["search"]

[[workflow.tasks]]
id = "score"
name = "Score"
type = "llm_call"
depends_on = ["search"]

[[workflow.tasks]]
id = "summarize"
name = "Summarize"
type = "llm_call"
depends_on = ["analyze", "score"]
"#;

        let result = parse_toml_workflow(toml).unwrap();
        let order = result.dag.topological_order().unwrap();
        assert_eq!(order[0], "search");
        assert_eq!(*order.last().unwrap(), "summarize");
        // analyze and score are in the middle (order may vary)
        assert!(order.contains(&"analyze".to_string()));
        assert!(order.contains(&"score".to_string()));
    }

    /// T-P4-03: Parse TOML workflow with input/output mappings.
    #[test]
    fn test_parse_with_mappings() {
        let toml = r#"
[workflow]
name = "mapped"

[[workflow.tasks]]
id = "search"
name = "Search"
type = "tool_execution"
tool_name = "web_search"

[workflow.tasks.inputs]
query = { source = "workflow_input", field = "user_query" }

[[workflow.tasks.outputs]]
target = "context"
channel = "search_results"

[[workflow.tasks]]
id = "analyze"
name = "Analyze"
type = "llm_call"
depends_on = ["search"]

[workflow.tasks.inputs]
data = { source = "task_output", task_id = "search", field = "results" }
"#;

        let result = parse_toml_workflow(toml).unwrap();
        assert!(result.input_mappings.contains_key("search"));
        assert!(result.input_mappings.contains_key("analyze"));
        assert!(result.output_mappings.contains_key("search"));

        let search_inputs = &result.input_mappings["search"];
        assert_eq!(search_inputs.len(), 1);
        assert_eq!(search_inputs[0].0, "query");

        let search_outputs = &result.output_mappings["search"];
        assert_eq!(search_outputs.len(), 1);
    }

    /// T-P4-04: Invalid TOML produces clear error message.
    #[test]
    fn test_invalid_toml_error() {
        let result = parse_toml_workflow("not valid toml {{{}}}");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TomlParseError::Syntax(_)));
    }

    /// T-P4-04b: Empty workflow produces validation error.
    #[test]
    fn test_empty_workflow_error() {
        let toml = r#"
[workflow]
name = "empty"
tasks = []
"#;
        let result = parse_toml_workflow(toml);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TomlParseError::Validation { .. }
        ));
    }

    /// T-P4-05: TOML and DSL produce equivalent TaskDag for same workflow.
    #[test]
    fn test_toml_dsl_equivalence() {
        // DSL: a >> b >> c
        let dsl_def = WorkflowDefinition::Expression("a >> b >> c".into());
        let dsl_result = dsl_def.parse().unwrap();
        let dsl_order = dsl_result.dag.topological_order().unwrap();

        // TOML equivalent
        let toml = r#"
[workflow]
name = "abc"

[[workflow.tasks]]
id = "a"
name = "a"
type = "noop"

[[workflow.tasks]]
id = "b"
name = "b"
type = "noop"
depends_on = ["a"]

[[workflow.tasks]]
id = "c"
name = "c"
type = "noop"
depends_on = ["b"]
"#;

        let toml_def = WorkflowDefinition::Toml(toml.into());
        let toml_result = toml_def.parse().unwrap();
        let toml_order = toml_result.dag.topological_order().unwrap();

        assert_eq!(dsl_order, toml_order);
    }

    /// WorkflowDefinition::Expression round-trips through parse.
    #[test]
    fn test_workflow_definition_expression() {
        let def = WorkflowDefinition::Expression("a | b | c".into());
        let result = def.parse().unwrap();
        let order = result.dag.topological_order().unwrap();
        assert_eq!(order.len(), 3);
    }

    /// Parse TOML with retry and failure strategy.
    #[test]
    fn test_parse_with_retry() {
        let toml = r#"
[workflow]
name = "retry-test"

[[workflow.tasks]]
id = "flaky"
name = "Flaky Task"
type = "noop"
timeout_ms = 5000
failure_strategy = "continue_on_error"

[workflow.tasks.retry]
max_attempts = 5
delay_ms = 100
backoff = "exponential"
"#;

        let result = parse_toml_workflow(toml).unwrap();
        let tasks = result.dag.topological_order().unwrap();
        assert_eq!(tasks.len(), 1);
    }
}
