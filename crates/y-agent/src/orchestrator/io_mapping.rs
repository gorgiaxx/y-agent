//! Input/output mapping: how data flows between workflow tasks.
//!
//! Design reference: orchestrator-design.md, Data Flow & Mapping
//!
//! Tasks declare their inputs and outputs as mappings. The executor resolves
//! inputs before dispatching a task and applies output mappings after the
//! task completes successfully.

use serde::{Deserialize, Serialize};

use crate::orchestrator::dag::TaskId;

// ---------------------------------------------------------------------------
// Input mapping
// ---------------------------------------------------------------------------

/// Declares how a task receives one named input.
///
/// The executor resolves each `InputMapping` before calling the
/// `TaskExecutor::execute()` method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum InputMapping {
    /// Read from the top-level workflow inputs supplied at execution time.
    WorkflowInput {
        /// Field name in the workflow's input map.
        field: String,
    },
    /// Read from a predecessor task's output.
    TaskOutput {
        /// ID of the predecessor task.
        task_id: TaskId,
        /// Field name in that task's output.
        field: String,
    },
    /// Read the current value of a typed channel.
    Context {
        /// Channel name.
        channel: String,
    },
    /// A literal constant value.
    Constant {
        /// The constant value.
        value: serde_json::Value,
    },
    /// A template expression resolved at runtime (e.g. `{{ workflow.input.query }}`).
    Expression {
        /// The expression string.
        expr: String,
    },
}

// ---------------------------------------------------------------------------
// Output mapping
// ---------------------------------------------------------------------------

/// Declares where a task's output is routed.
///
/// The executor applies each `OutputMapping` after a task completes
/// successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum OutputMapping {
    /// Write to the top-level workflow outputs.
    WorkflowOutput {
        /// Field name in the workflow's output map.
        field: String,
    },
    /// Write to a typed channel (applies the channel's reducer).
    Context {
        /// Channel name to write to.
        channel: String,
    },
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

/// Resolve input mappings into concrete values.
///
/// Callers provide lookup functions for each data source. Returns a map from
/// input name to resolved value.
pub fn resolve_inputs<S: ::std::hash::BuildHasher>(
    mappings: &[(&str, &InputMapping)],
    workflow_inputs: &serde_json::Map<String, serde_json::Value>,
    task_outputs: &std::collections::HashMap<TaskId, serde_json::Value, S>,
    ctx: &crate::orchestrator::channel::WorkflowContext,
) -> Result<std::collections::HashMap<String, serde_json::Value>, InputResolveError> {
    let mut resolved = std::collections::HashMap::new();

    for &(name, mapping) in mappings {
        let value = match mapping {
            InputMapping::WorkflowInput { field } => workflow_inputs
                .get(field)
                .cloned()
                .ok_or_else(|| InputResolveError::MissingWorkflowInput {
                    field: field.clone(),
                })?,
            InputMapping::TaskOutput { task_id, field } => {
                let output = task_outputs.get(task_id).ok_or_else(|| {
                    InputResolveError::MissingTaskOutput {
                        task_id: task_id.clone(),
                    }
                })?;
                output
                    .get(field)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
            }
            InputMapping::Context { channel } => ctx
                .read(channel)
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            InputMapping::Constant { value } => value.clone(),
            InputMapping::Expression { expr } => {
                // Simple template resolution: {{ workflow.input.X }}
                if let Some(field) = expr
                    .strip_prefix("{{ workflow.input.")
                    .and_then(|s| s.strip_suffix(" }}"))
                {
                    workflow_inputs
                        .get(field)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    // Unresolved expressions pass through as string
                    serde_json::Value::String(expr.clone())
                }
            }
        };
        resolved.insert(name.to_string(), value);
    }

    Ok(resolved)
}

/// Error from input resolution.
#[derive(Debug, thiserror::Error)]
pub enum InputResolveError {
    #[error("workflow input field '{field}' not provided")]
    MissingWorkflowInput { field: String },

    #[error("task output for '{task_id}' not available (task may not have run yet)")]
    MissingTaskOutput { task_id: String },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P2-01: `InputMapping::WorkflowInput` resolves from initial inputs.
    #[test]
    fn test_resolve_workflow_input() {
        let mut wf_inputs = serde_json::Map::new();
        wf_inputs.insert("query".into(), serde_json::json!("rust async"));

        let mapping = InputMapping::WorkflowInput {
            field: "query".into(),
        };
        let ctx = crate::orchestrator::channel::WorkflowContext::new();
        let task_outputs = std::collections::HashMap::new();

        let result =
            resolve_inputs(&[("query", &mapping)], &wf_inputs, &task_outputs, &ctx).unwrap();
        assert_eq!(result["query"], serde_json::json!("rust async"));
    }

    /// T-P2-02: `InputMapping::TaskOutput` resolves from predecessor output.
    #[test]
    fn test_resolve_task_output() {
        let wf_inputs = serde_json::Map::new();
        let mut task_outputs = std::collections::HashMap::new();
        task_outputs.insert(
            "search".to_string(),
            serde_json::json!({"results": ["a", "b"]}),
        );

        let mapping = InputMapping::TaskOutput {
            task_id: "search".into(),
            field: "results".into(),
        };
        let ctx = crate::orchestrator::channel::WorkflowContext::new();

        let result =
            resolve_inputs(&[("data", &mapping)], &wf_inputs, &task_outputs, &ctx).unwrap();
        assert_eq!(result["data"], serde_json::json!(["a", "b"]));
    }

    /// T-P2-03: `InputMapping::Context` resolves from channel.
    #[test]
    fn test_resolve_context_channel() {
        let wf_inputs = serde_json::Map::new();
        let task_outputs = std::collections::HashMap::new();
        let mut ctx = crate::orchestrator::channel::WorkflowContext::new();
        ctx.write("state", serde_json::json!(42));

        let mapping = InputMapping::Context {
            channel: "state".into(),
        };

        let result = resolve_inputs(&[("val", &mapping)], &wf_inputs, &task_outputs, &ctx).unwrap();
        assert_eq!(result["val"], serde_json::json!(42));
    }

    /// `InputMapping::Constant` resolves to the literal value.
    #[test]
    fn test_resolve_constant() {
        let wf_inputs = serde_json::Map::new();
        let task_outputs = std::collections::HashMap::new();
        let ctx = crate::orchestrator::channel::WorkflowContext::new();

        let mapping = InputMapping::Constant {
            value: serde_json::json!("fixed"),
        };

        let result = resolve_inputs(&[("c", &mapping)], &wf_inputs, &task_outputs, &ctx).unwrap();
        assert_eq!(result["c"], serde_json::json!("fixed"));
    }

    /// `InputMapping::Expression` resolves simple template expressions.
    #[test]
    fn test_resolve_expression() {
        let mut wf_inputs = serde_json::Map::new();
        wf_inputs.insert("topic".into(), serde_json::json!("AI"));
        let task_outputs = std::collections::HashMap::new();
        let ctx = crate::orchestrator::channel::WorkflowContext::new();

        let mapping = InputMapping::Expression {
            expr: "{{ workflow.input.topic }}".into(),
        };

        let result = resolve_inputs(&[("t", &mapping)], &wf_inputs, &task_outputs, &ctx).unwrap();
        assert_eq!(result["t"], serde_json::json!("AI"));
    }

    /// Missing workflow input produces an error.
    #[test]
    fn test_resolve_missing_workflow_input() {
        let wf_inputs = serde_json::Map::new();
        let task_outputs = std::collections::HashMap::new();
        let ctx = crate::orchestrator::channel::WorkflowContext::new();

        let mapping = InputMapping::WorkflowInput {
            field: "nonexistent".into(),
        };

        let result = resolve_inputs(&[("x", &mapping)], &wf_inputs, &task_outputs, &ctx);
        assert!(result.is_err());
    }

    /// `InputMapping` serialization round-trip.
    #[test]
    fn test_input_mapping_serialization() {
        let mapping = InputMapping::TaskOutput {
            task_id: "step-1".into(),
            field: "result".into(),
        };
        let json = serde_json::to_string(&mapping).unwrap();
        let deserialized: InputMapping = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, InputMapping::TaskOutput { .. }));
    }

    /// `OutputMapping` serialization round-trip.
    #[test]
    fn test_output_mapping_serialization() {
        let mapping = OutputMapping::Context {
            channel: "state".into(),
        };
        let json = serde_json::to_string(&mapping).unwrap();
        let deserialized: OutputMapping = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, OutputMapping::Context { .. }));
    }
}
