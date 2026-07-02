//! Skill evolution event subscribers.
//!
//! Provides two `EventSubscriber` implementations that feed the
//! self-evolution pipeline from the async event bus:
//!
//! B1: `SkillUsageAuditSubscriber` — listens for `LlmCallCompleted` events
//!     and updates per-skill `SkillMetrics` (injection/actual-usage counts)
//!     for the skills tracked as injected into the current context. The
//!     overlap heuristic is a placeholder pending a richer output-text event
//!     payload (see `y_skills::usage_audit::SkillUsageAudit` for the real
//!     keyword-overlap audit used in production).
//!
//! B2: `ExperienceCaptureSubscriber` — listens for custom
//!     `skill_execution_completed` events and appends `ExperienceRecord`
//!     entries to an in-memory ring buffer for later pattern extraction.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{debug, info};

use y_core::hook::{Event, EventCategory, EventFilter, EventSubscriber, LlmEvent};
use y_skills::{
    evolution::SkillMetrics, experience::ExperienceOutcome, ExperienceRecord, InjectedSkill,
    TokenUsage,
};

// ---------------------------------------------------------------------------
// B1: Skill Usage Audit Subscriber
// ---------------------------------------------------------------------------

/// Shared state for tracking injected skills across LLM calls.
///
/// This holds `InjectedSkill` records (the canonical y-skills type) for the
/// active context. It is session-level accumulation state — distinct from a
/// single `InjectedSkill` — and therefore not a shadow of any y-skills type.
#[derive(Debug, Default)]
pub struct SkillInjectionTracker {
    /// Skills currently injected in the active context.
    pub injected_skills: Vec<InjectedSkill>,
}

impl SkillInjectionTracker {
    /// Record that a skill was injected into the context.
    pub fn record_injection(&mut self, name: String, content: String) {
        self.injected_skills.push(InjectedSkill { name, content });
    }

    /// Clear tracked injections (e.g., after audit).
    pub fn clear(&mut self) {
        self.injected_skills.clear();
    }
}

/// Per-skill usage metrics accumulated over the session.
///
/// This is the same aggregation shape `y_skills::usage_audit::SkillUsageAudit`
/// uses (`HashMap<String, SkillMetrics>`); the subscriber updates
/// `SkillMetrics` directly rather than a parallel `SkillUsageCounts` struct.
pub type UsageMetrics = HashMap<String, SkillMetrics>;

/// Subscribes to `LlmCallCompleted` events and performs keyword-overlap-based
/// usage auditing against the tracked injected skills.
///
/// Integration: Register via `EventBus::subscribe_handler()`.
pub struct SkillUsageAuditSubscriber {
    /// Shared injection tracker (populated by context middleware).
    tracker: Arc<RwLock<SkillInjectionTracker>>,
    /// Accumulated usage metrics.
    metrics: Arc<RwLock<UsageMetrics>>,
    /// Overlap threshold to consider a skill "used".
    usage_threshold: f64,
}

impl SkillUsageAuditSubscriber {
    /// Create a new usage audit subscriber.
    pub fn new(
        tracker: Arc<RwLock<SkillInjectionTracker>>,
        metrics: Arc<RwLock<UsageMetrics>>,
    ) -> Self {
        Self {
            tracker,
            metrics,
            usage_threshold: 0.15,
        }
    }

    /// Get the accumulated metrics.
    pub fn metrics(&self) -> Arc<RwLock<UsageMetrics>> {
        Arc::clone(&self.metrics)
    }
}

#[async_trait]
impl EventSubscriber for SkillUsageAuditSubscriber {
    async fn on_event(&self, event: &Event) {
        if let Event::Llm(LlmEvent::CallCompleted {
            input_tokens,
            output_tokens,
            ..
        }) = event
        {
            let tracker = self.tracker.read().await;
            if tracker.injected_skills.is_empty() {
                return;
            }

            debug!(
                injected_count = tracker.injected_skills.len(),
                input_tokens, output_tokens, "Auditing skill usage after LLM call"
            );

            let mut metrics = self.metrics.write().await;
            for skill in &tracker.injected_skills {
                let m = metrics.entry(skill.name.clone()).or_default();
                m.record_injection();
                // Placeholder heuristic: estimate skill usage from token ratios.
                // The real keyword-overlap audit lives in
                // `y_skills::usage_audit::SkillUsageAudit::audit()`, which needs
                // the actual LLM output text — not yet carried by this event.
                if *output_tokens > 0 && !skill.content.is_empty() {
                    let estimated_overlap =
                        f64::from(*output_tokens) / f64::from(*input_tokens).max(1.0);
                    if estimated_overlap >= self.usage_threshold {
                        m.record_actual_usage();
                    }
                }
            }

            info!(
                skills_tracked = tracker.injected_skills.len(),
                "Skill usage audit completed"
            );
        }
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter::categories(vec![EventCategory::Llm])
    }
}

// ---------------------------------------------------------------------------
// B2: Experience Capture Subscriber
// ---------------------------------------------------------------------------

/// Subscribes to custom `skill_execution_completed` events and captures
/// structured experience records for the evolution pipeline.
///
/// Integration: Register via `EventBus::subscribe_handler()`.
pub struct ExperienceCaptureSubscriber {
    /// In-memory store for captured experiences.
    store: Arc<RwLock<Vec<ExperienceRecord>>>,
}

const MAX_EXPERIENCES: usize = 500;

impl ExperienceCaptureSubscriber {
    /// Create a new experience capture subscriber.
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get the captured experiences.
    pub fn store(&self) -> Arc<RwLock<Vec<ExperienceRecord>>> {
        Arc::clone(&self.store)
    }
}

impl Default for ExperienceCaptureSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventSubscriber for ExperienceCaptureSubscriber {
    async fn on_event(&self, event: &Event) {
        if let Event::Custom { name, payload } = event {
            if name == "skill_execution_completed" {
                let skill_id = payload
                    .get("skill_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let success = payload
                    .get("success")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let duration_ms = payload
                    .get("duration_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let tokens_used = u32::try_from(
                    payload
                        .get("tokens_used")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                )
                .unwrap_or(0);

                let experience = ExperienceRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    skill_id: Some(skill_id.clone()),
                    skill_version: None,
                    task_description: String::new(),
                    outcome: if success {
                        ExperienceOutcome::Success
                    } else {
                        ExperienceOutcome::Failure
                    },
                    trajectory_summary: String::new(),
                    key_decisions: Vec::new(),
                    evidence: Vec::new(),
                    tool_calls: Vec::new(),
                    error_messages: Vec::new(),
                    duration_ms,
                    token_usage: TokenUsage::new(0, tokens_used),
                };

                let mut store = self.store.write().await;
                store.push(experience);
                if store.len() > MAX_EXPERIENCES {
                    let drain_count = store.len() - MAX_EXPERIENCES / 2;
                    store.drain(..drain_count);
                }

                info!(
                    skill_id = %skill_id,
                    success,
                    duration_ms,
                    "Captured skill execution experience"
                );
            }
        }
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter::categories(vec![EventCategory::Custom])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-B1-01: Usage metrics track injection and usage counts.
    #[test]
    fn test_usage_metrics_tracking() {
        let mut metrics = UsageMetrics::default();
        metrics
            .entry("skill-a".to_string())
            .or_default()
            .record_injection();
        metrics
            .entry("skill-a".to_string())
            .or_default()
            .record_injection();
        metrics
            .entry("skill-b".to_string())
            .or_default()
            .record_injection();
        metrics
            .entry("skill-a".to_string())
            .or_default()
            .record_actual_usage();

        assert_eq!(metrics["skill-a"].injection_count, 2);
        assert_eq!(metrics["skill-a"].actual_usage_count, 1);
        assert_eq!(metrics["skill-b"].injection_count, 1);
        assert_eq!(metrics["skill-b"].actual_usage_count, 0);

        assert!((metrics["skill-a"].usage_rate() - 0.5).abs() < f64::EPSILON);
        assert!((metrics["skill-b"].usage_rate()).abs() < f64::EPSILON);
    }

    /// T-SK-B1-02: Injection tracker records and clears skills.
    #[test]
    fn test_injection_tracker() {
        let mut tracker = SkillInjectionTracker::default();
        tracker.record_injection("skill-1".into(), "content 1".into());
        tracker.record_injection("skill-2".into(), "content 2".into());

        assert_eq!(tracker.injected_skills.len(), 2);
        assert_eq!(tracker.injected_skills[0].name, "skill-1");
        assert_eq!(tracker.injected_skills[0].content, "content 1");

        tracker.clear();
        assert!(tracker.injected_skills.is_empty());
    }

    /// T-SK-B1-03: Usage audit subscriber filters for LLM events.
    #[tokio::test]
    async fn test_usage_audit_subscriber_event_filter() {
        let tracker = Arc::new(RwLock::new(SkillInjectionTracker::default()));
        let metrics = Arc::new(RwLock::new(UsageMetrics::default()));
        let subscriber = SkillUsageAuditSubscriber::new(tracker, metrics);

        let filter = subscriber.event_filter();
        assert_eq!(filter.categories, vec![EventCategory::Llm]);
    }

    /// T-SK-B2-01: Experience capture subscriber records custom events.
    #[tokio::test]
    async fn test_experience_capture_subscriber() {
        let subscriber = ExperienceCaptureSubscriber::new();

        let event = Event::Custom {
            name: "skill_execution_completed".into(),
            payload: serde_json::json!({
                "skill_id": "humanizer",
                "success": true,
                "duration_ms": 1500,
                "tokens_used": 800
            }),
        };

        subscriber.on_event(&event).await;

        let store = subscriber.store();
        let experiences = store.read().await;
        assert_eq!(experiences.len(), 1);
        assert_eq!(experiences[0].skill_id.as_deref(), Some("humanizer"));
        assert_eq!(experiences[0].outcome, ExperienceOutcome::Success);
        assert_eq!(experiences[0].duration_ms, 1500);
        assert_eq!(experiences[0].token_usage.total, 800);
    }

    /// T-SK-B2-02: Experience capture ignores non-matching custom events.
    #[tokio::test]
    async fn test_experience_capture_ignores_other_events() {
        let subscriber = ExperienceCaptureSubscriber::new();

        let event = Event::Custom {
            name: "something_else".into(),
            payload: serde_json::json!({}),
        };

        subscriber.on_event(&event).await;

        let store = subscriber.store();
        let experiences = store.read().await;
        assert!(experiences.is_empty());
    }
}
