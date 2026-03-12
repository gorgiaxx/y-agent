//! DAG-based task scheduler with topological ordering.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

/// Unique task identifier.
pub type TaskId = String;

/// Task priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TaskPriority {
    Critical = 0,
    High = 1,
    Normal = 2,
    Low = 3,
    Background = 4,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// A task node in the DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// Unique identifier.
    pub id: TaskId,
    /// Human-readable name.
    pub name: String,
    /// Task priority.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Dependencies (task IDs that must complete before this task).
    pub dependencies: Vec<TaskId>,
}

/// Task DAG for scheduling.
#[derive(Debug)]
pub struct TaskDag {
    nodes: HashMap<TaskId, TaskNode>,
    // Reverse adjacency: task_id → set of tasks that depend on it.
    dependents: HashMap<TaskId, HashSet<TaskId>>,
}

impl TaskDag {
    /// Create a new empty DAG.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Add a task to the DAG.
    ///
    /// Returns an error if a task with the same ID already exists.
    pub fn add_task(&mut self, node: TaskNode) -> Result<(), DagError> {
        if self.nodes.contains_key(&node.id) {
            return Err(DagError::DuplicateTask {
                task: node.id.clone(),
            });
        }
        let id = node.id.clone();
        for dep in &node.dependencies {
            self.dependents
                .entry(dep.clone())
                .or_default()
                .insert(id.clone());
        }
        self.nodes.insert(id, node);
        Ok(())
    }

    /// Validate the DAG: check for cycles and missing dependencies.
    pub fn validate(&self) -> Result<(), DagError> {
        // Check for missing dependencies.
        for node in self.nodes.values() {
            for dep in &node.dependencies {
                if !self.nodes.contains_key(dep) {
                    return Err(DagError::MissingDependency {
                        task: node.id.clone(),
                        dependency: dep.clone(),
                    });
                }
            }
        }

        // Topological sort to detect cycles.
        self.topological_order().map(|_| ())
    }

    /// Get tasks ready to execute (all dependencies satisfied).
    pub fn ready_tasks(&self, completed: &HashSet<TaskId>) -> Vec<&TaskNode> {
        let mut ready: Vec<&TaskNode> = self
            .nodes
            .values()
            .filter(|n| {
                !completed.contains(&n.id) && n.dependencies.iter().all(|d| completed.contains(d))
            })
            .collect();
        // Sort by priority (Critical first) then name for determinism.
        ready.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.id.cmp(&b.id)));
        ready
    }

    /// Compute topological ordering of tasks.
    pub fn topological_order(&self) -> Result<Vec<TaskId>, DagError> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for node in self.nodes.values() {
            in_degree.insert(&node.id, node.dependencies.len());
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut result = Vec::new();
        while let Some(id) = queue.pop_front() {
            result.push(id.to_string());
            if let Some(deps) = self.dependents.get(id) {
                for dep_id in deps {
                    if let Some(deg) = in_degree.get_mut(dep_id.as_str()) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(dep_id);
                        }
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err(DagError::CycleDetected);
        }

        Ok(result)
    }

    /// Number of tasks.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Default for TaskDag {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors in DAG operations.
#[derive(Debug, thiserror::Error)]
pub enum DagError {
    #[error("cycle detected in task DAG")]
    CycleDetected,
    #[error("task '{task}' depends on missing task '{dependency}'")]
    MissingDependency { task: TaskId, dependency: TaskId },
    #[error("duplicate task ID '{task}'")]
    DuplicateTask { task: TaskId },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str]) -> TaskNode {
        TaskNode {
            id: id.into(),
            name: id.into(),
            priority: TaskPriority::Normal,
            dependencies: deps.iter().map(|d| (*d).to_string()).collect(),
        }
    }

    #[test]
    fn test_topological_order() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();
        dag.add_task(task("b", &["a"])).unwrap();
        dag.add_task(task("c", &["a"])).unwrap();
        dag.add_task(task("d", &["b", "c"])).unwrap();

        let order = dag.topological_order().unwrap();
        assert_eq!(order[0], "a"); // a must be first.
        assert_eq!(*order.last().unwrap(), "d"); // d must be last.
    }

    #[test]
    fn test_cycle_detection() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &["b"])).unwrap();
        dag.add_task(task("b", &["a"])).unwrap();

        assert!(matches!(
            dag.topological_order(),
            Err(DagError::CycleDetected)
        ));
    }

    #[test]
    fn test_ready_tasks() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();
        dag.add_task(task("b", &["a"])).unwrap();
        dag.add_task(task("c", &[])).unwrap();

        let completed = HashSet::new();
        let ready: Vec<&str> = dag
            .ready_tasks(&completed)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert!(ready.contains(&"a"));
        assert!(ready.contains(&"c"));
        assert!(!ready.contains(&"b"));

        let mut completed = HashSet::new();
        completed.insert("a".to_string());
        let ready: Vec<&str> = dag
            .ready_tasks(&completed)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert!(ready.contains(&"b"));
        assert!(ready.contains(&"c"));
    }

    #[test]
    fn test_missing_dependency() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &["missing"])).unwrap();

        assert!(matches!(
            dag.validate(),
            Err(DagError::MissingDependency { .. })
        ));
    }

    #[test]
    fn test_priority_ordering() {
        let mut dag = TaskDag::new();
        dag.add_task(TaskNode {
            id: "low".into(),
            name: "low".into(),
            priority: TaskPriority::Low,
            dependencies: vec![],
        })
        .unwrap();
        dag.add_task(TaskNode {
            id: "critical".into(),
            name: "critical".into(),
            priority: TaskPriority::Critical,
            dependencies: vec![],
        })
        .unwrap();

        let ready = dag.ready_tasks(&HashSet::new());
        assert_eq!(ready[0].id, "critical");
        assert_eq!(ready[1].id, "low");
    }

    #[test]
    fn test_duplicate_task_id() {
        let mut dag = TaskDag::new();
        dag.add_task(task("a", &[])).unwrap();
        let result = dag.add_task(task("a", &[]));
        assert!(matches!(result, Err(DagError::DuplicateTask { .. })));
    }
}
