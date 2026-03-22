//! Micro-agent pipeline: stateless multi-step execution with Working Memory.
//!
//! Design reference: micro-agent-pipeline-design.md
//!
//! Each step in the pipeline runs with a fresh session context containing
//! only the Working Memory slots from its declared input categories.
//! Steps produce output into a single cognitive category. This prevents
//! token accumulation across steps and enables weaker LLMs to handle
//! individual steps reliably.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::orchestrator::dag::{TaskDag, TaskNode, TaskPriority};

// ---------------------------------------------------------------------------
// Step definition
// ---------------------------------------------------------------------------

/// A single step in a micro-agent pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicroAgentStep {
    /// Unique identifier for this step.
    pub step_id: String,
    /// Human-readable step name.
    pub name: String,
    /// The cognitive category this step writes its output to.
    pub output_category: String,
    /// Cognitive categories this step reads as input.
    pub input_categories: Vec<String>,
    /// Maximum token budget for this step's context.
    pub token_budget: u32,
    /// Optional system prompt override for this step.
    pub system_prompt: Option<String>,
}

// ---------------------------------------------------------------------------
// Pipeline definition
// ---------------------------------------------------------------------------

/// Configuration for the micro-agent pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicroPipelineConfig {
    /// Maximum number of retry attempts per step.
    #[serde(default = "default_max_retries")]
    pub max_retries_per_step: u32,
    /// Whether to discard each step's session after completion.
    #[serde(default = "default_true")]
    pub discard_sessions: bool,
}

fn default_max_retries() -> u32 {
    2
}

fn default_true() -> bool {
    true
}

impl Default for MicroPipelineConfig {
    fn default() -> Self {
        Self {
            max_retries_per_step: default_max_retries(),
            discard_sessions: true,
        }
    }
}

/// A micro-agent pipeline: ordered sequence of steps communicating
/// through Working Memory slots.
#[derive(Debug, Clone)]
pub struct MicroAgentPipeline {
    /// Ordered steps.
    pub steps: Vec<MicroAgentStep>,
    /// Pipeline configuration.
    pub config: MicroPipelineConfig,
}

impl MicroAgentPipeline {
    /// Create a new pipeline with the given steps and default config.
    pub fn new(steps: Vec<MicroAgentStep>) -> Self {
        Self {
            steps,
            config: MicroPipelineConfig::default(),
        }
    }

    /// Create a pipeline with custom config.
    pub fn with_config(steps: Vec<MicroAgentStep>, config: MicroPipelineConfig) -> Self {
        Self { steps, config }
    }

    /// Number of steps in the pipeline.
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// Validate the pipeline definition.
    ///
    /// Checks that:
    /// - At least one step exists
    /// - Step IDs are unique
    /// - No circular input/output category dependencies
    /// - Each step's input categories are produced by earlier steps
    ///   (or are external inputs)
    pub fn validate(&self) -> Result<(), MicroPipelineError> {
        if self.steps.is_empty() {
            return Err(MicroPipelineError::EmptyPipeline);
        }

        let mut seen_ids = std::collections::HashSet::new();
        for step in &self.steps {
            if !seen_ids.insert(&step.step_id) {
                return Err(MicroPipelineError::DuplicateStepId {
                    step_id: step.step_id.clone(),
                });
            }
        }

        // Check that each step's inputs are produced by a preceding step
        // or are the first step (which reads external input).
        let mut available_categories = std::collections::HashSet::new();
        for step in &self.steps {
            for input_cat in &step.input_categories {
                // First step is allowed to read categories not yet produced
                // (they come from external input / user task).
                if !available_categories.contains(input_cat.as_str())
                    && step.step_id != self.steps[0].step_id
                {
                    return Err(MicroPipelineError::MissingInputCategory {
                        step_id: step.step_id.clone(),
                        category: input_cat.clone(),
                    });
                }
            }
            available_categories.insert(step.output_category.as_str());
        }

        Ok(())
    }

    /// Convert this pipeline to a `TaskDag` for execution by the Orchestrator.
    ///
    /// Each step becomes a `TaskNode`. Dependencies are derived from the
    /// input/output category relationships.
    ///
    /// # Panics
    ///
    /// Panics if any step has a duplicate ID. Call [`validate`](Self::validate)
    /// first to ensure step IDs are unique.
    pub fn to_task_dag(&self) -> TaskDag {
        // Map output_category → step_id for dependency resolution.
        let mut category_producers: HashMap<&str, &str> = HashMap::new();

        let mut dag = TaskDag::new();
        for step in &self.steps {
            // Find dependencies: steps that produce the categories this step reads.
            let deps: Vec<String> = step
                .input_categories
                .iter()
                .filter_map(|cat| {
                    category_producers
                        .get(cat.as_str())
                        .copied()
                        .map(str::to_string)
                })
                .collect();

            let node = TaskNode {
                id: step.step_id.clone(),
                name: step.name.clone(),
                priority: TaskPriority::Normal,
                dependencies: deps,
                ..TaskNode::default()
            };
            dag.add_task(node)
                .expect("MicroAgentPipeline steps should have unique IDs (validate first)");

            category_producers.insert(&step.output_category, &step.step_id);
        }

        dag
    }
}

// ---------------------------------------------------------------------------
// Step execution result
// ---------------------------------------------------------------------------

/// The result of executing a single pipeline step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// The step that was executed.
    pub step_id: String,
    /// Output category written.
    pub output_category: String,
    /// Output value.
    pub output: serde_json::Value,
    /// Tokens consumed by this step.
    pub tokens_used: u32,
    /// Whether retries were needed.
    pub retry_count: u32,
}

/// The result of executing the full pipeline.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Results from each step in execution order.
    pub step_results: Vec<StepResult>,
    /// Total tokens consumed across all steps.
    pub total_tokens: u32,
}

impl PipelineResult {
    /// Get the result of a specific step.
    pub fn get_step(&self, step_id: &str) -> Option<&StepResult> {
        self.step_results.iter().find(|r| r.step_id == step_id)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from micro-agent pipeline operations.
#[derive(Debug, thiserror::Error)]
pub enum MicroPipelineError {
    #[error("pipeline has no steps")]
    EmptyPipeline,
    #[error("duplicate step ID: {step_id}")]
    DuplicateStepId { step_id: String },
    #[error("step {step_id} reads category '{category}' not produced by any preceding step")]
    MissingInputCategory { step_id: String, category: String },
    #[error("step {step_id} failed after {attempts} attempts: {message}")]
    StepFailed {
        step_id: String,
        attempts: u32,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect_step() -> MicroAgentStep {
        MicroAgentStep {
            step_id: "inspect".into(),
            name: "Inspect".into(),
            output_category: "perception".into(),
            input_categories: vec![],
            token_budget: 2000,
            system_prompt: None,
        }
    }

    fn locate_step() -> MicroAgentStep {
        MicroAgentStep {
            step_id: "locate".into(),
            name: "Locate".into(),
            output_category: "structure".into(),
            input_categories: vec!["perception".into()],
            token_budget: 2000,
            system_prompt: None,
        }
    }

    fn analyze_step() -> MicroAgentStep {
        MicroAgentStep {
            step_id: "analyze".into(),
            name: "Analyze".into(),
            output_category: "analysis".into(),
            input_categories: vec!["perception".into(), "structure".into()],
            token_budget: 2000,
            system_prompt: None,
        }
    }

    fn execute_step() -> MicroAgentStep {
        MicroAgentStep {
            step_id: "execute".into(),
            name: "Execute".into(),
            output_category: "execution".into(),
            input_categories: vec!["analysis".into()],
            token_budget: 2000,
            system_prompt: Some("Apply the analysis to produce code changes.".into()),
        }
    }

    /// T-P3-36-07: A valid 4-step pipeline passes validation.
    #[test]
    fn test_pipeline_validation_valid() {
        let pipeline = MicroAgentPipeline::new(vec![
            inspect_step(),
            locate_step(),
            analyze_step(),
            execute_step(),
        ]);
        assert!(pipeline.validate().is_ok());
        assert_eq!(pipeline.step_count(), 4);
    }

    /// T-P3-36-08: Empty pipeline is rejected.
    #[test]
    fn test_pipeline_validation_empty() {
        let pipeline = MicroAgentPipeline::new(vec![]);
        let err = pipeline.validate().unwrap_err();
        assert!(matches!(err, MicroPipelineError::EmptyPipeline));
    }

    /// T-P3-36-09: Duplicate step IDs are rejected.
    #[test]
    fn test_pipeline_validation_duplicate_ids() {
        let pipeline = MicroAgentPipeline::new(vec![inspect_step(), inspect_step()]);
        let err = pipeline.validate().unwrap_err();
        assert!(matches!(err, MicroPipelineError::DuplicateStepId { .. }));
    }

    /// T-P3-36-10: Step reading non-existent category is rejected.
    #[test]
    fn test_pipeline_validation_missing_input() {
        let bad_step = MicroAgentStep {
            step_id: "bad".into(),
            name: "Bad".into(),
            output_category: "output".into(),
            input_categories: vec!["nonexistent".into()],
            token_budget: 2000,
            system_prompt: None,
        };
        let pipeline = MicroAgentPipeline::new(vec![inspect_step(), bad_step]);
        let err = pipeline.validate().unwrap_err();
        assert!(matches!(
            err,
            MicroPipelineError::MissingInputCategory { .. }
        ));
    }

    /// T-P3-36-11: Pipeline converts to `TaskDag` with correct dependencies.
    #[test]
    fn test_pipeline_to_task_dag() {
        let pipeline = MicroAgentPipeline::new(vec![
            inspect_step(),
            locate_step(),
            analyze_step(),
            execute_step(),
        ]);

        let dag = pipeline.to_task_dag();
        assert!(dag.validate().is_ok());

        // Inspect has no deps → ready first
        let completed = std::collections::HashSet::new();
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "inspect");

        // After inspect, locate should be ready
        let mut completed = std::collections::HashSet::new();
        completed.insert("inspect".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "locate");

        // After inspect + locate, analyze should be ready
        completed.insert("locate".to_string());
        let ready = dag.ready_tasks(&completed);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "analyze");
    }

    /// T-P3-36-12: Pipeline config defaults are correct.
    #[test]
    fn test_pipeline_config_defaults() {
        let config = MicroPipelineConfig::default();
        assert_eq!(config.max_retries_per_step, 2);
        assert!(config.discard_sessions);
    }

    /// T-P3-36-13: `PipelineResult` lookup by `step_id`.
    #[test]
    fn test_pipeline_result_get_step() {
        let result = PipelineResult {
            step_results: vec![
                StepResult {
                    step_id: "step-1".into(),
                    output_category: "perception".into(),
                    output: serde_json::json!({"files": 3}),
                    tokens_used: 500,
                    retry_count: 0,
                },
                StepResult {
                    step_id: "step-2".into(),
                    output_category: "analysis".into(),
                    output: serde_json::json!({"conclusion": "ok"}),
                    tokens_used: 800,
                    retry_count: 1,
                },
            ],
            total_tokens: 1300,
        };

        assert!(result.get_step("step-1").is_some());
        assert!(result.get_step("step-2").is_some());
        assert!(result.get_step("step-3").is_none());
        assert_eq!(result.total_tokens, 1300);
    }
}
