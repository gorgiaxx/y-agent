//! Interrupt/resume protocol for human-in-the-loop workflows.

use serde::{Deserialize, Serialize};

/// A workflow interrupt raised by a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowInterrupt {
    /// Request human approval for a high-risk operation.
    HumanApproval {
        prompt: String,
        options: Vec<String>,
    },
    /// Request confirmation before proceeding.
    Confirmation { action: String },
    /// Request additional input from the user.
    InputRequired {
        prompt: String,
        /// JSON Schema describing the expected input.
        schema: Option<serde_json::Value>,
    },
}

/// Command to resume an interrupted workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ResumeCommand {
    /// Approve the pending action.
    Approve { selected: serde_json::Value },
    /// Reject the pending action.
    Reject { reason: String },
    /// Provide requested data.
    Provide { data: serde_json::Value },
    /// Cancel the workflow.
    Cancel,
}

/// Persisted interrupt state.
#[derive(Debug, Clone)]
pub struct InterruptState {
    /// Workflow execution ID.
    pub execution_id: String,
    /// Task that raised the interrupt.
    pub task_id: String,
    /// The interrupt details.
    pub interrupt: WorkflowInterrupt,
    /// Whether the interrupt has been resolved.
    pub resolved: bool,
    /// Resume command (if resolved).
    pub resume_command: Option<ResumeCommand>,
}

/// Interrupt manager.
pub struct InterruptManager {
    interrupts: Vec<InterruptState>,
}

impl InterruptManager {
    /// Create a new interrupt manager.
    pub fn new() -> Self {
        Self {
            interrupts: Vec::new(),
        }
    }

    /// Raise an interrupt.
    pub fn raise(
        &mut self,
        execution_id: &str,
        task_id: &str,
        interrupt: WorkflowInterrupt,
    ) -> usize {
        let idx = self.interrupts.len();
        self.interrupts.push(InterruptState {
            execution_id: execution_id.to_string(),
            task_id: task_id.to_string(),
            interrupt,
            resolved: false,
            resume_command: None,
        });
        idx
    }

    /// Resolve an interrupt with a resume command.
    pub fn resolve(&mut self, index: usize, command: ResumeCommand) -> bool {
        if let Some(state) = self.interrupts.get_mut(index) {
            if !state.resolved {
                state.resolved = true;
                state.resume_command = Some(command);
                return true;
            }
        }
        false
    }

    /// Get pending (unresolved) interrupts for an execution.
    pub fn pending(&self, execution_id: &str) -> Vec<&InterruptState> {
        self.interrupts
            .iter()
            .filter(|i| i.execution_id == execution_id && !i.resolved)
            .collect()
    }

    /// Whether an execution is currently interrupted.
    pub fn is_interrupted(&self, execution_id: &str) -> bool {
        !self.pending(execution_id).is_empty()
    }
}

impl Default for InterruptManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raise_and_resolve_interrupt() {
        let mut mgr = InterruptManager::new();
        let idx = mgr.raise(
            "exec-1",
            "task-1",
            WorkflowInterrupt::HumanApproval {
                prompt: "Deploy to prod?".into(),
                options: vec!["yes".into(), "no".into()],
            },
        );

        assert!(mgr.is_interrupted("exec-1"));
        assert_eq!(mgr.pending("exec-1").len(), 1);

        mgr.resolve(
            idx,
            ResumeCommand::Approve {
                selected: serde_json::json!("yes"),
            },
        );

        assert!(!mgr.is_interrupted("exec-1"));
    }

    #[test]
    fn test_multiple_interrupts() {
        let mut mgr = InterruptManager::new();
        mgr.raise(
            "exec-1",
            "task-1",
            WorkflowInterrupt::Confirmation {
                action: "delete files".into(),
            },
        );
        mgr.raise(
            "exec-1",
            "task-2",
            WorkflowInterrupt::InputRequired {
                prompt: "Enter API key".into(),
                schema: None,
            },
        );

        assert_eq!(mgr.pending("exec-1").len(), 2);
        assert_eq!(mgr.pending("exec-2").len(), 0);
    }

    #[test]
    fn test_cannot_resolve_twice() {
        let mut mgr = InterruptManager::new();
        let idx = mgr.raise(
            "exec-1",
            "task-1",
            WorkflowInterrupt::Confirmation {
                action: "test".into(),
            },
        );
        assert!(mgr.resolve(idx, ResumeCommand::Cancel));
        assert!(!mgr.resolve(idx, ResumeCommand::Cancel));
    }
}
