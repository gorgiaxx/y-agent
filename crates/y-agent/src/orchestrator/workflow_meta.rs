//! Workflow meta-tools: self-orchestration protocol for workflow CRUD.
//!
//! Design reference: agent-autonomy-design.md §Self-Orchestration Protocol
//!
//! The `WorkflowStore` persists reusable workflow templates (TOML or DSL).
//! Agents can create, list, get, and delete templates at runtime.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Workflow template
// ---------------------------------------------------------------------------

/// A reusable workflow template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    /// Unique template ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Description of what this workflow does.
    pub description: String,
    /// The workflow definition (TOML string or Expression DSL string).
    pub definition: String,
    /// Whether this is a TOML definition or an Expression DSL.
    pub format: WorkflowFormat,
    /// JSON Schema for parameters (if parameterized).
    pub parameter_schema: Option<serde_json::Value>,
    /// Tags for discovery.
    pub tags: Vec<String>,
    /// Version number.
    pub version: u32,
}

/// Format of the workflow definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFormat {
    /// TOML-based workflow definition.
    Toml,
    /// Expression DSL (e.g., `a >> (b | c) >> d`).
    ExpressionDsl,
}

/// Error from workflow store operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowStoreError {
    #[error("template '{id}' already exists")]
    AlreadyExists { id: String },
    #[error("template '{id}' not found")]
    NotFound { id: String },
    #[error("template name is empty")]
    EmptyName,
}

// ---------------------------------------------------------------------------
// Workflow store
// ---------------------------------------------------------------------------

/// In-memory store for workflow templates.
#[derive(Debug, Default)]
pub struct WorkflowStore {
    templates: HashMap<String, WorkflowTemplate>,
}

impl WorkflowStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new workflow template.
    ///
    /// # Panics
    ///
    /// This function will not panic under normal usage; the internal
    /// `expect` is guarded by the preceding `insert`.
    pub fn register(
        &mut self,
        template: WorkflowTemplate,
    ) -> Result<&WorkflowTemplate, WorkflowStoreError> {
        if template.name.is_empty() {
            return Err(WorkflowStoreError::EmptyName);
        }
        if self.templates.contains_key(&template.id) {
            return Err(WorkflowStoreError::AlreadyExists {
                id: template.id.clone(),
            });
        }
        let id = template.id.clone();
        self.templates.insert(id.clone(), template);
        Ok(self.templates.get(&id).expect("just inserted"))
    }

    /// Get a template by ID.
    pub fn get(&self, id: &str) -> Option<&WorkflowTemplate> {
        self.templates.get(id)
    }

    /// List all templates.
    pub fn list(&self) -> Vec<&WorkflowTemplate> {
        self.templates.values().collect()
    }

    /// List templates matching a tag.
    pub fn list_by_tag(&self, tag: &str) -> Vec<&WorkflowTemplate> {
        self.templates
            .values()
            .filter(|t| t.tags.iter().any(|tg| tg == tag))
            .collect()
    }

    /// Delete a template by ID.
    pub fn delete(&mut self, id: &str) -> Result<WorkflowTemplate, WorkflowStoreError> {
        self.templates
            .remove(id)
            .ok_or_else(|| WorkflowStoreError::NotFound { id: id.to_string() })
    }

    /// Number of templates.
    pub fn count(&self) -> usize {
        self.templates.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_template(id: &str) -> WorkflowTemplate {
        WorkflowTemplate {
            id: id.to_string(),
            name: format!("Test Workflow {id}"),
            description: "A test workflow".to_string(),
            definition: "step_a >> step_b".to_string(),
            format: WorkflowFormat::ExpressionDsl,
            parameter_schema: None,
            tags: vec!["test".to_string()],
            version: 1,
        }
    }

    /// T-P3-38-08: Register and retrieve a workflow template.
    #[test]
    fn test_workflow_register_get() {
        let mut store = WorkflowStore::new();
        store.register(test_template("wf-1")).unwrap();

        let t = store.get("wf-1").unwrap();
        assert_eq!(t.name, "Test Workflow wf-1");
        assert_eq!(t.format, WorkflowFormat::ExpressionDsl);
    }

    /// T-P3-38-09: Duplicate template ID is rejected.
    #[test]
    fn test_workflow_duplicate_rejected() {
        let mut store = WorkflowStore::new();
        store.register(test_template("wf-1")).unwrap();
        assert!(store.register(test_template("wf-1")).is_err());
    }

    /// T-P3-38-10: Delete a workflow template.
    #[test]
    fn test_workflow_delete() {
        let mut store = WorkflowStore::new();
        store.register(test_template("wf-1")).unwrap();
        let deleted = store.delete("wf-1").unwrap();
        assert_eq!(deleted.id, "wf-1");
        assert!(store.get("wf-1").is_none());
    }

    /// T-P3-38-11: List templates by tag.
    #[test]
    fn test_workflow_list_by_tag() {
        let mut store = WorkflowStore::new();
        store.register(test_template("wf-1")).unwrap();

        let mut t2 = test_template("wf-2");
        t2.tags = vec!["production".to_string()];
        store.register(t2).unwrap();

        let test_wfs = store.list_by_tag("test");
        assert_eq!(test_wfs.len(), 1);
        let prod_wfs = store.list_by_tag("production");
        assert_eq!(prod_wfs.len(), 1);
    }
}
