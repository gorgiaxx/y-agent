use std::sync::Arc;

use tempfile::tempdir;
use uuid::Uuid;
use y_diagnostics::{InMemoryTraceStore, Trace, TraceStatus, TraceStore};
use y_service::diagnostics::DiagnosticsService;
use y_service::evolution_feedback::{EvolutionFeedbackInput, EvolutionFeedbackService};
use y_service::skill_evolution_service::SkillEvolutionService;
use y_skills::experience::EvidenceProvenance;

#[tokio::test]
async fn feedback_is_durable_idempotent_and_updates_asset_evidence() {
    let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
    let mut trace = Trace::new(Uuid::new_v4(), "subagent:dyn-code-scout");
    trace.metadata = serde_json::json!({
        "orchestration": { "selected_skills": ["review-rust"] },
        "dynamic_agent": { "id": "dyn-code-scout", "version": 2 }
    });
    trace.user_input = Some("Review the ownership module".to_string());
    trace.complete();
    let trace_id = trace.id;
    trace_store.insert_trace(trace).await.unwrap();

    let temp = tempdir().unwrap();
    let skill_evolution = Arc::new(
        SkillEvolutionService::open(temp.path().join("skill-evolution"), None)
            .await
            .unwrap(),
    );
    let service =
        EvolutionFeedbackService::new(Arc::clone(&trace_store), Arc::clone(&skill_evolution));
    let feedback_id = Uuid::new_v4();
    let input = EvolutionFeedbackInput {
        feedback_id,
        trace_id,
        score: 0.0,
        comment: Some("The review missed the unsafe aliasing path".to_string()),
    };

    let first = service.record(input.clone()).await.unwrap();
    let repeated = service.record(input).await.unwrap();

    assert!(!first.duplicate);
    assert!(repeated.duplicate);
    assert_eq!(first.skill_experiences_recorded, 1);
    assert_eq!(repeated.skill_experiences_recorded, 0);
    let scores = trace_store.get_scores(trace_id).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].id, feedback_id);
    let trace = trace_store.get_trace(trace_id).await.unwrap();
    assert_eq!(trace.metadata["user_feedback"]["score"], 0.0);

    let experiences = skill_evolution.load_experiences().await.unwrap();
    assert_eq!(experiences.len(), 1);
    assert!(experiences[0]
        .evidence
        .iter()
        .any(|entry| entry.provenance == EvidenceProvenance::UserCorrection));

    let metrics = DiagnosticsService::dynamic_agent_version_metrics(trace_store, None, 100)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].total_runs, 1);
    assert_eq!(metrics[0].failed_runs, 1);
    assert_eq!(metrics[0].successful_runs, 0);
    assert_eq!(metrics[0].success_rate, 0.0);
}

#[tokio::test]
async fn negative_feedback_requires_an_actionable_correction() {
    let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
    let mut trace = Trace::new(Uuid::new_v4(), "chat-turn");
    trace.status = TraceStatus::Completed;
    let trace_id = trace.id;
    trace_store.insert_trace(trace).await.unwrap();
    let temp = tempdir().unwrap();
    let skill_evolution = Arc::new(
        SkillEvolutionService::open(temp.path().join("skill-evolution"), None)
            .await
            .unwrap(),
    );
    let service = EvolutionFeedbackService::new(trace_store, skill_evolution);

    let result = service
        .record(EvolutionFeedbackInput {
            feedback_id: Uuid::new_v4(),
            trace_id,
            score: 0.0,
            comment: None,
        })
        .await;

    assert!(result.unwrap_err().to_string().contains("comment"));
}

#[tokio::test]
async fn repeated_negative_feedback_can_trigger_a_dynamic_agent_regression() {
    let trace_store: Arc<dyn TraceStore> = Arc::new(InMemoryTraceStore::new());
    let temp = tempdir().unwrap();
    let skill_evolution = Arc::new(
        SkillEvolutionService::open(temp.path().join("skill-evolution"), None)
            .await
            .unwrap(),
    );
    let service =
        EvolutionFeedbackService::new(Arc::clone(&trace_store), Arc::clone(&skill_evolution));

    for version in [1_u64, 2] {
        for sample in 0..5 {
            let mut trace = Trace::new(Uuid::new_v4(), "subagent:dyn-code-scout");
            trace.metadata = serde_json::json!({
                "dynamic_agent": { "id": "dyn-code-scout", "version": version }
            });
            trace.complete();
            let trace_id = trace.id;
            trace_store.insert_trace(trace).await.unwrap();
            if version == 2 {
                service
                    .record(EvolutionFeedbackInput {
                        feedback_id: Uuid::new_v4(),
                        trace_id,
                        score: 0.0,
                        comment: Some(format!(
                            "Sample {sample} missed the required ownership invariant"
                        )),
                    })
                    .await
                    .unwrap();
            }
        }
    }

    let findings = DiagnosticsService::dynamic_agent_regressions(trace_store, None, 100, 5, 0.25)
        .await
        .unwrap();

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].baseline_version, 1);
    assert_eq!(findings[0].current_version, 2);
    assert_eq!(findings[0].success_rate_drop, 1.0);
}
