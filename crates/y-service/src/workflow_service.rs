//! Workflow template management service.
//!
//! Provides CRUD operations for workflow templates, DSL/TOML validation,
//! and DAG visualization serialization for GUI rendering.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use y_agent::orchestrator::dag::{TaskDag, TaskNode};
use y_agent::orchestrator::expression_dsl;
use y_agent::orchestrator::toml_parser::{self, WorkflowDefinition};
use y_storage::workflow_store::WorkflowRow;
use y_storage::SqliteWorkflowStore;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from workflow service operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowServiceError {
    #[error("workflow not found: {id}")]
    NotFound { id: String },

    #[error("validation failed: {message}")]
    Validation { message: String },

    #[error("storage error: {0}")]
    Storage(#[from] y_storage::StorageError),
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request to create a new workflow template.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateWorkflowRequest {
    /// Unique name for the workflow.
    pub name: String,
    /// Workflow definition body (DSL expression or TOML content).
    pub definition: String,
    /// Format of the definition: "expression_dsl" or "toml".
    #[serde(default = "default_format")]
    pub format: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Comma-separated tags or JSON array.
    pub tags: Option<String>,
}

fn default_format() -> String {
    "expression_dsl".to_string()
}

/// Request to update an existing workflow template.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWorkflowRequest {
    /// Updated definition body.
    pub definition: Option<String>,
    /// Updated format (if definition is changed).
    pub format: Option<String>,
    /// Updated description.
    pub description: Option<String>,
    /// Updated tags.
    pub tags: Option<String>,
}

/// Result of validating a workflow definition.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    /// Whether the definition is valid.
    pub valid: bool,
    /// Error messages (empty if valid).
    pub errors: Vec<String>,
    /// AST display string (for DSL expressions).
    pub ast_display: Option<String>,
    /// DAG visualization (if parsing succeeded).
    pub dag: Option<DagVisualization>,
}

/// Serializable DAG structure for GUI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagVisualization {
    /// All nodes (tasks) in the DAG.
    pub nodes: Vec<DagNode>,
    /// All edges (dependencies) between tasks.
    pub edges: Vec<DagEdge>,
    /// Topological ordering of task IDs.
    pub topological_order: Vec<String>,
}

/// A single node in the DAG visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagNode {
    /// Unique task ID.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    /// Task type label (e.g., "llm_call", "tool_execution", "noop").
    pub task_type: String,
    /// Priority level.
    pub priority: String,
    /// IDs of tasks this depends on.
    pub dependencies: Vec<String>,
    /// Whether retry is configured.
    pub has_retry: bool,
    /// Failure strategy name.
    pub failure_strategy: String,
}

/// A dependency edge in the DAG visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagEdge {
    /// Source task ID.
    pub from: String,
    /// Target task ID (depends on `from`).
    pub to: String,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Workflow service: CRUD + validation + DAG visualization.
pub struct WorkflowService;

impl WorkflowService {
    /// List all workflow templates.
    pub async fn list(
        store: &SqliteWorkflowStore,
    ) -> Result<Vec<WorkflowRow>, WorkflowServiceError> {
        store.list().await.map_err(WorkflowServiceError::Storage)
    }

    /// Get a workflow template by ID (falls back to name lookup).
    pub async fn get(
        store: &SqliteWorkflowStore,
        identifier: &str,
    ) -> Result<WorkflowRow, WorkflowServiceError> {
        // Try by ID first.
        if let Some(row) = store
            .get(identifier)
            .await
            .map_err(WorkflowServiceError::Storage)?
        {
            return Ok(row);
        }
        // Fallback: by name.
        store
            .get_by_name(identifier)
            .await
            .map_err(WorkflowServiceError::Storage)?
            .ok_or_else(|| WorkflowServiceError::NotFound {
                id: identifier.to_string(),
            })
    }

    /// Create a new workflow template.
    ///
    /// Validates the definition (DSL or TOML), compiles the DAG, and persists.
    pub async fn create(
        store: &SqliteWorkflowStore,
        req: &CreateWorkflowRequest,
    ) -> Result<WorkflowRow, WorkflowServiceError> {
        // Validate definition.
        let validation = Self::validate_definition(&req.definition, &req.format);
        if !validation.valid {
            return Err(WorkflowServiceError::Validation {
                message: validation.errors.join("; "),
            });
        }

        // Compile DAG to JSON for storage.
        let compiled_dag = Self::compile_dag_json(&req.definition, &req.format)?;

        // Parse tags.
        let tags_json = parse_tags_to_json(req.tags.as_deref());

        let id = uuid::Uuid::new_v4().to_string();
        let row = WorkflowRow {
            id: id.clone(),
            name: req.name.clone(),
            description: req.description.clone(),
            definition: req.definition.clone(),
            format: req.format.clone(),
            compiled_dag,
            parameter_schema: None,
            tags: tags_json,
            creator: "user".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
        };

        store
            .save(&row)
            .await
            .map_err(WorkflowServiceError::Storage)?;

        // Re-read to get server-set timestamps.
        store
            .get(&id)
            .await
            .map_err(WorkflowServiceError::Storage)?
            .ok_or_else(|| WorkflowServiceError::NotFound { id })
    }

    /// Update an existing workflow template.
    pub async fn update(
        store: &SqliteWorkflowStore,
        id: &str,
        req: &UpdateWorkflowRequest,
    ) -> Result<WorkflowRow, WorkflowServiceError> {
        let mut existing = Self::get(store, id).await?;

        if let Some(ref definition) = req.definition {
            let format = req
                .format
                .as_deref()
                .unwrap_or(&existing.format)
                .to_string();
            let validation = Self::validate_definition(definition, &format);
            if !validation.valid {
                return Err(WorkflowServiceError::Validation {
                    message: validation.errors.join("; "),
                });
            }
            existing.definition = definition.clone();
            existing.compiled_dag = Self::compile_dag_json(definition, &format)?;
            existing.format = format;
        }

        if let Some(ref description) = req.description {
            existing.description = Some(description.clone());
        }

        if let Some(ref tags) = req.tags {
            existing.tags = parse_tags_to_json(Some(tags.as_str()));
        }

        store
            .save(&existing)
            .await
            .map_err(WorkflowServiceError::Storage)?;

        Self::get(store, id).await
    }

    /// Delete a workflow template by ID.
    pub async fn delete(
        store: &SqliteWorkflowStore,
        id: &str,
    ) -> Result<bool, WorkflowServiceError> {
        store.delete(id).await.map_err(WorkflowServiceError::Storage)
    }

    /// Validate a workflow definition without persisting.
    ///
    /// Returns structured validation results including AST display and DAG info.
    pub fn validate_definition(definition: &str, format: &str) -> ValidationResult {
        match format {
            "expression_dsl" => Self::validate_dsl(definition),
            "toml" => Self::validate_toml(definition),
            other => ValidationResult {
                valid: false,
                errors: vec![format!("unsupported format: {other}")],
                ast_display: None,
                dag: None,
            },
        }
    }

    /// Get DAG visualization data for a stored workflow.
    pub async fn get_dag_visualization(
        store: &SqliteWorkflowStore,
        id: &str,
    ) -> Result<DagVisualization, WorkflowServiceError> {
        let row = Self::get(store, id).await?;
        let validation = Self::validate_definition(&row.definition, &row.format);
        validation.dag.ok_or_else(|| WorkflowServiceError::Validation {
            message: validation.errors.join("; "),
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn validate_dsl(definition: &str) -> ValidationResult {
        let ast = match expression_dsl::parse(definition) {
            Ok(a) => a,
            Err(e) => {
                return ValidationResult {
                    valid: false,
                    errors: vec![e.to_string()],
                    ast_display: None,
                    dag: None,
                };
            }
        };

        let dag = match ast.to_task_dag() {
            Ok(d) => d,
            Err(e) => {
                return ValidationResult {
                    valid: false,
                    errors: vec![e.to_string()],
                    ast_display: Some(ast.to_string()),
                    dag: None,
                };
            }
        };

        if let Err(e) = dag.validate() {
            return ValidationResult {
                valid: false,
                errors: vec![e.to_string()],
                ast_display: Some(ast.to_string()),
                dag: None,
            };
        }

        ValidationResult {
            valid: true,
            errors: Vec::new(),
            ast_display: Some(ast.to_string()),
            dag: Some(build_dag_visualization(&dag)),
        }
    }

    fn validate_toml(definition: &str) -> ValidationResult {
        match toml_parser::parse_toml_workflow(definition) {
            Ok(parsed) => {
                if let Err(e) = parsed.dag.validate() {
                    return ValidationResult {
                        valid: false,
                        errors: vec![e.to_string()],
                        ast_display: None,
                        dag: None,
                    };
                }
                ValidationResult {
                    valid: true,
                    errors: Vec::new(),
                    ast_display: None,
                    dag: Some(build_dag_visualization(&parsed.dag)),
                }
            }
            Err(e) => ValidationResult {
                valid: false,
                errors: vec![e.to_string()],
                ast_display: None,
                dag: None,
            },
        }
    }

    fn compile_dag_json(
        definition: &str,
        format: &str,
    ) -> Result<String, WorkflowServiceError> {
        let def = match format {
            "expression_dsl" => WorkflowDefinition::Expression(definition.to_string()),
            "toml" => WorkflowDefinition::Toml(definition.to_string()),
            other => {
                return Err(WorkflowServiceError::Validation {
                    message: format!("unsupported format: {other}"),
                });
            }
        };

        let parsed = def
            .parse()
            .map_err(|e| WorkflowServiceError::Validation {
                message: e.to_string(),
            })?;

        let topo = parsed
            .dag
            .topological_order()
            .map_err(|e| WorkflowServiceError::Validation {
                message: e.to_string(),
            })?;

        serde_json::to_string(&topo).map_err(|e| WorkflowServiceError::Validation {
            message: format!("JSON serialization error: {e}"),
        })
    }
}

// ---------------------------------------------------------------------------
// DAG visualization builder
// ---------------------------------------------------------------------------

/// Build a `DagVisualization` from a validated `TaskDag`.
fn build_dag_visualization(dag: &TaskDag) -> DagVisualization {
    let topo = dag.topological_order().unwrap_or_default();

    // Collect all tasks via iterative readiness traversal.
    let mut visited = HashSet::new();
    let mut all_tasks: Vec<&TaskNode> = Vec::new();

    let mut ready = dag.ready_tasks(&visited);
    while !ready.is_empty() {
        for task in &ready {
            visited.insert(task.id.clone());
            all_tasks.push(task);
        }
        ready = dag.ready_tasks(&visited);
        ready.retain(|t| !visited.contains(&t.id));
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for task in &all_tasks {
        nodes.push(DagNode {
            id: task.id.clone(),
            name: task.name.clone(),
            task_type: format!("{:?}", task.task_type),
            priority: format!("{:?}", task.priority),
            dependencies: task.dependencies.clone(),
            has_retry: task.retry.is_some(),
            failure_strategy: format!("{:?}", task.failure_strategy),
        });

        for dep in &task.dependencies {
            edges.push(DagEdge {
                from: dep.clone(),
                to: task.id.clone(),
            });
        }
    }

    DagVisualization {
        nodes,
        edges,
        topological_order: topo,
    }
}

// ---------------------------------------------------------------------------
// Tag parsing helper
// ---------------------------------------------------------------------------

/// Parse comma-separated or JSON-array tags into a JSON array string.
fn parse_tags_to_json(tags: Option<&str>) -> String {
    match tags {
        Some(t) if t.starts_with('[') => {
            // Already JSON array; validate and pass through.
            if serde_json::from_str::<Vec<String>>(t).is_ok() {
                t.to_string()
            } else {
                "[]".to_string()
            }
        }
        Some(t) => {
            let tag_list: Vec<&str> = t.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
            serde_json::to_string(&tag_list).unwrap_or_else(|_| "[]".to_string())
        }
        None => "[]".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_dsl_valid() {
        let result = WorkflowService::validate_definition(
            "search >> (analyze | score) >> summarize",
            "expression_dsl",
        );
        assert!(result.valid);
        assert!(result.errors.is_empty());
        assert!(result.ast_display.is_some());
        assert!(result.dag.is_some());

        let dag = result.dag.unwrap();
        assert_eq!(dag.nodes.len(), 4);
        assert_eq!(dag.topological_order.len(), 4);
        // search is first, summarize is last.
        assert_eq!(dag.topological_order[0], "search");
        assert_eq!(*dag.topological_order.last().unwrap(), "summarize");
    }

    #[test]
    fn test_validate_dsl_invalid() {
        let result = WorkflowService::validate_definition(">> invalid", "expression_dsl");
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
        assert!(result.dag.is_none());
    }

    #[test]
    fn test_validate_dsl_empty() {
        let result = WorkflowService::validate_definition("", "expression_dsl");
        assert!(!result.valid);
    }

    #[test]
    fn test_validate_toml_valid() {
        let toml = r#"
[workflow]
name = "test"

[[workflow.tasks]]
id = "step1"
name = "Step 1"
type = "noop"

[[workflow.tasks]]
id = "step2"
name = "Step 2"
type = "noop"
depends_on = ["step1"]
"#;
        let result = WorkflowService::validate_definition(toml, "toml");
        assert!(result.valid);
        assert!(result.dag.is_some());
        let dag = result.dag.unwrap();
        assert_eq!(dag.nodes.len(), 2);
        assert_eq!(dag.edges.len(), 1);
        assert_eq!(dag.edges[0].from, "step1");
        assert_eq!(dag.edges[0].to, "step2");
    }

    #[test]
    fn test_validate_toml_invalid() {
        let result = WorkflowService::validate_definition("not valid toml {{{", "toml");
        assert!(!result.valid);
    }

    #[test]
    fn test_validate_unsupported_format() {
        let result = WorkflowService::validate_definition("anything", "yaml");
        assert!(!result.valid);
        assert!(result.errors[0].contains("unsupported"));
    }

    #[test]
    fn test_dag_visualization_edges() {
        // a >> (b | c) >> d has edges: a->b, a->c, b->d, c->d
        let result = WorkflowService::validate_definition(
            "a >> (b | c) >> d",
            "expression_dsl",
        );
        let dag = result.dag.unwrap();
        assert_eq!(dag.edges.len(), 4);

        let edge_set: HashSet<(String, String)> = dag
            .edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone()))
            .collect();
        assert!(edge_set.contains(&("a".into(), "b".into())));
        assert!(edge_set.contains(&("a".into(), "c".into())));
        assert!(edge_set.contains(&("b".into(), "d".into())));
        assert!(edge_set.contains(&("c".into(), "d".into())));
    }

    #[test]
    fn test_parse_tags_json() {
        assert_eq!(parse_tags_to_json(None), "[]");
        assert_eq!(
            parse_tags_to_json(Some("a, b, c")),
            r#"["a","b","c"]"#
        );
        assert_eq!(
            parse_tags_to_json(Some(r#"["x","y"]"#)),
            r#"["x","y"]"#
        );
        assert_eq!(parse_tags_to_json(Some("")), "[]");
    }

    #[test]
    fn test_compile_dag_json_dsl() {
        let json = WorkflowService::compile_dag_json("a >> b >> c", "expression_dsl").unwrap();
        let order: Vec<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_compile_dag_json_invalid() {
        let result = WorkflowService::compile_dag_json(">> bad", "expression_dsl");
        assert!(result.is_err());
    }
}
