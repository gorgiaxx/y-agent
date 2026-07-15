//! Explicit user-feedback ingestion for diagnostics and governed evolution.

use std::sync::Arc;

use uuid::Uuid;
use y_diagnostics::{Score, ScoreSource, ScoreValue, TraceStore};

use crate::skill_evolution_service::SkillEvolutionService;

#[derive(Debug, Clone)]
pub struct EvolutionFeedbackInput {
    pub feedback_id: Uuid,
    pub trace_id: Uuid,
    pub score: f64,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DynamicAgentFeedbackTarget {
    pub id: String,
    pub version: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EvolutionFeedbackOutcome {
    pub feedback_id: Uuid,
    pub trace_id: Uuid,
    pub duplicate: bool,
    pub skill_experiences_recorded: usize,
    pub dynamic_agent: Option<DynamicAgentFeedbackTarget>,
}

pub struct EvolutionFeedbackService {
    trace_store: Arc<dyn TraceStore>,
    skill_evolution: Arc<SkillEvolutionService>,
}

impl EvolutionFeedbackService {
    pub fn new(
        trace_store: Arc<dyn TraceStore>,
        skill_evolution: Arc<SkillEvolutionService>,
    ) -> Self {
        Self {
            trace_store,
            skill_evolution,
        }
    }

    pub async fn record(
        &self,
        input: EvolutionFeedbackInput,
    ) -> Result<EvolutionFeedbackOutcome, EvolutionFeedbackError> {
        validate_input(&input)?;
        let comment = input
            .comment
            .as_deref()
            .map(str::trim)
            .filter(|comment| !comment.is_empty())
            .map(String::from);
        let mut trace = self
            .trace_store
            .get_trace(input.trace_id)
            .await
            .map_err(|_| EvolutionFeedbackError::TraceNotFound {
                trace_id: input.trace_id,
            })?;
        let scores = self
            .trace_store
            .get_scores(input.trace_id)
            .await
            .map_err(storage_error)?;
        let existing = scores.iter().find(|score| score.id == input.feedback_id);
        if let Some(existing) = existing {
            let same_score = matches!(
                existing.value,
                ScoreValue::Numeric(value) if (value - input.score).abs() < f64::EPSILON
            );
            if !same_score || existing.comment != comment {
                return Err(EvolutionFeedbackError::Validation {
                    message: "feedback_id is already associated with different feedback"
                        .to_string(),
                });
            }
        } else {
            let mut score = Score::numeric(
                input.trace_id,
                "user_feedback",
                input.score,
                ScoreSource::UserFeedback,
            );
            score.id = input.feedback_id;
            score.comment = comment.clone();
            self.trace_store
                .insert_score(score)
                .await
                .map_err(storage_error)?;
        }

        if !trace.metadata.is_object() {
            trace.metadata = serde_json::json!({});
        }
        trace.metadata["user_feedback"] = serde_json::json!({
            "feedback_id": input.feedback_id,
            "score": input.score,
            "comment": comment,
        });
        self.trace_store
            .update_trace(trace.clone())
            .await
            .map_err(storage_error)?;

        let skills = selected_skills(&trace.metadata);
        let task_description = trace.user_input.as_deref().unwrap_or(&trace.name);
        let evidence_comment = comment
            .as_deref()
            .unwrap_or("User confirmed this result without an additional comment");
        let skill_experiences_recorded = self
            .skill_evolution
            .record_user_feedback(
                input.feedback_id,
                input.trace_id,
                &skills,
                task_description,
                input.score,
                evidence_comment,
            )
            .await
            .map_err(|error| EvolutionFeedbackError::Storage {
                message: error.to_string(),
            })?;
        let dynamic_agent = dynamic_agent_target(&trace.metadata);

        Ok(EvolutionFeedbackOutcome {
            feedback_id: input.feedback_id,
            trace_id: input.trace_id,
            duplicate: existing.is_some() && skill_experiences_recorded == 0,
            skill_experiences_recorded,
            dynamic_agent,
        })
    }
}

fn validate_input(input: &EvolutionFeedbackInput) -> Result<(), EvolutionFeedbackError> {
    if !input.score.is_finite() || !(0.0..=1.0).contains(&input.score) {
        return Err(EvolutionFeedbackError::Validation {
            message: "feedback score must be between 0.0 and 1.0".to_string(),
        });
    }
    if input.score <= 0.25
        && input
            .comment
            .as_deref()
            .is_none_or(|comment| comment.trim().is_empty())
    {
        return Err(EvolutionFeedbackError::Validation {
            message: "negative feedback requires a non-blank correction comment".to_string(),
        });
    }
    Ok(())
}

fn selected_skills(metadata: &serde_json::Value) -> Vec<String> {
    metadata
        .pointer("/orchestration/selected_skills")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(String::from)
        .collect()
}

fn dynamic_agent_target(metadata: &serde_json::Value) -> Option<DynamicAgentFeedbackTarget> {
    Some(DynamicAgentFeedbackTarget {
        id: metadata.pointer("/dynamic_agent/id")?.as_str()?.to_string(),
        version: metadata.pointer("/dynamic_agent/version")?.as_u64()?,
    })
}

fn storage_error(error: impl std::fmt::Display) -> EvolutionFeedbackError {
    EvolutionFeedbackError::Storage {
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvolutionFeedbackError {
    #[error("{message}")]
    Validation { message: String },
    #[error("diagnostics trace not found: {trace_id}")]
    TraceNotFound { trace_id: Uuid },
    #[error("failed to persist evolution feedback: {message}")]
    Storage { message: String },
}
