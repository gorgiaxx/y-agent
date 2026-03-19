//! Micro-agent pipeline: stateless steps with working memory slots.
//!
//! Design reference: multi-agent-design.md §Micro-Agent Pipeline
//!
//! Each pipeline step runs a lightweight agent that reads from and writes
//! to named working memory (WM) slots. The agent's session context is
//! discarded after each step completes — only the WM slot output persists.

use std::collections::HashMap;

use crate::agent::delegation::{DelegationProtocol, DelegationResult};
use crate::agent::error::MultiAgentError;

// ---------------------------------------------------------------------------
// Working memory
// ---------------------------------------------------------------------------

/// Working memory: named string slots shared between pipeline steps.
///
/// Each step can read from any slot and writes its output to a designated
/// output slot. Previous step outputs are available to subsequent steps.
#[derive(Debug, Clone, Default)]
pub struct WorkingMemory {
    slots: HashMap<String, String>,
}

impl WorkingMemory {
    /// Create empty working memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a slot value.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.slots.get(key).map(String::as_str)
    }

    /// Write a value to a slot.
    pub fn set(&mut self, key: &str, value: &str) {
        self.slots.insert(key.to_string(), value.to_string());
    }

    /// Check if a slot exists.
    pub fn has(&self, key: &str) -> bool {
        self.slots.contains_key(key)
    }

    /// Number of populated slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether working memory is empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// All slot names.
    pub fn slot_names(&self) -> Vec<&str> {
        self.slots.keys().map(String::as_str).collect()
    }
}

// ---------------------------------------------------------------------------
// Pipeline step
// ---------------------------------------------------------------------------

/// A single step in a micro-agent pipeline.
#[derive(Debug, Clone)]
pub struct PipelineStep {
    /// Agent ID to execute this step.
    pub agent_id: String,
    /// Task prompt for this step (may reference WM slots via `{slot_name}`).
    pub prompt_template: String,
    /// WM slot names this step reads from.
    pub input_slots: Vec<String>,
    /// WM slot name this step writes to.
    pub output_slot: String,
}

impl PipelineStep {
    /// Create a new pipeline step.
    pub fn new(
        agent_id: &str,
        prompt_template: &str,
        input_slots: Vec<String>,
        output_slot: &str,
    ) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            prompt_template: prompt_template.to_string(),
            input_slots,
            output_slot: output_slot.to_string(),
        }
    }

    /// Render the prompt by substituting WM slot values.
    fn render_prompt(&self, wm: &WorkingMemory) -> String {
        let mut prompt = self.prompt_template.clone();
        for slot in &self.input_slots {
            if let Some(value) = wm.get(slot) {
                prompt = prompt.replace(&format!("{{{slot}}}"), value);
            }
        }
        prompt
    }
}

// ---------------------------------------------------------------------------
// Pipeline executor
// ---------------------------------------------------------------------------

/// Result of a pipeline execution.
#[derive(Debug)]
pub struct PipelineResult {
    /// Working memory state after all steps complete.
    pub working_memory: WorkingMemory,
    /// Per-step delegation results (session contexts discarded).
    pub step_results: Vec<DelegationResult>,
}

/// Micro-agent pipeline executor.
///
/// Runs a sequence of lightweight agent steps. Each step:
/// 1. Reads input from WM slots
/// 2. Renders the prompt template
/// 3. Executes the agent
/// 4. Writes output to the designated WM slot
/// 5. Discards the agent's session context (stateless)
pub struct MicroPipeline;

impl MicroPipeline {
    /// Execute a pipeline of steps with shared working memory.
    pub fn execute(
        protocol: &DelegationProtocol,
        steps: &[PipelineStep],
        initial_wm: WorkingMemory,
    ) -> Result<PipelineResult, MultiAgentError> {
        let mut wm = initial_wm;
        let mut step_results = Vec::new();

        for step in steps {
            // Render prompt from WM
            let prompt = step.render_prompt(&wm);

            // Create and execute delegation task
            let task = protocol.create_task(&step.agent_id, &prompt);
            let result = protocol.execute_sync(&task)?;

            if !result.success {
                return Err(MultiAgentError::DelegationFailed {
                    message: format!(
                        "Pipeline step '{}' failed: {}",
                        step.agent_id, result.output
                    ),
                });
            }

            // Write output to WM slot (session context is discarded)
            wm.set(&step.output_slot, &result.output);

            step_results.push(result);
        }

        Ok(PipelineResult {
            working_memory: wm,
            step_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;

    /// T-MA-P7-09: Step writes WM slot, next step reads it.
    #[test]
    fn test_micro_pipeline_wm_slots() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());

        let mut initial_wm = WorkingMemory::new();
        initial_wm.set("input", "raw data to process");

        let steps = vec![
            PipelineStep::new(
                "analyzer",
                "Analyze: {input}",
                vec!["input".to_string()],
                "analysis",
            ),
            PipelineStep::new(
                "summarizer",
                "Summarize analysis: {analysis}",
                vec!["analysis".to_string()],
                "summary",
            ),
        ];

        let result = MicroPipeline::execute(&protocol, &steps, initial_wm).unwrap();

        // Both slots should be populated
        assert!(result.working_memory.has("input"));
        assert!(result.working_memory.has("analysis"));
        assert!(result.working_memory.has("summary"));
        assert_eq!(result.step_results.len(), 2);
    }

    /// T-MA-P7-10: Session context is discarded (only WM persists).
    #[test]
    fn test_micro_pipeline_session_discard() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());

        let steps = vec![
            PipelineStep::new("step-1", "Do task A", vec![], "out_a"),
            PipelineStep::new(
                "step-2",
                "Do task B with {out_a}",
                vec!["out_a".to_string()],
                "out_b",
            ),
        ];

        let result = MicroPipeline::execute(&protocol, &steps, WorkingMemory::new()).unwrap();

        // Each step result is independent (step 2 doesn't get step 1's session)
        assert_eq!(result.step_results.len(), 2);
        assert_eq!(result.step_results[0].agent_id, "step-1");
        assert_eq!(result.step_results[1].agent_id, "step-2");

        // WM should have the outputs
        assert!(result.working_memory.has("out_a"));
        assert!(result.working_memory.has("out_b"));
        assert_eq!(result.working_memory.len(), 2);
    }

    /// Working memory basic operations.
    #[test]
    fn test_working_memory_operations() {
        let mut wm = WorkingMemory::new();
        assert!(wm.is_empty());

        wm.set("key1", "value1");
        wm.set("key2", "value2");

        assert_eq!(wm.len(), 2);
        assert_eq!(wm.get("key1"), Some("value1"));
        assert!(wm.has("key1"));
        assert!(!wm.has("key3"));

        // Overwrite
        wm.set("key1", "updated");
        assert_eq!(wm.get("key1"), Some("updated"));
        assert_eq!(wm.len(), 2);
    }

    /// Prompt template rendering with WM slots.
    #[test]
    fn test_prompt_template_rendering() {
        let mut wm = WorkingMemory::new();
        wm.set("topic", "Rust ownership");
        wm.set("detail", "borrowing rules");

        let step = PipelineStep::new(
            "writer",
            "Write about {topic} focusing on {detail}",
            vec!["topic".to_string(), "detail".to_string()],
            "output",
        );

        let rendered = step.render_prompt(&wm);
        assert_eq!(
            rendered,
            "Write about Rust ownership focusing on borrowing rules"
        );
    }
}
