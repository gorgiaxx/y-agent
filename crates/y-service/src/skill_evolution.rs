//! Skill evolution event subscribers — wires `SkillUsageAudit` and
//! `ExperienceStore` into the async event bus for automatic invocation.
//!
//! Design reference: skills-knowledge-design.md §Evolution Pipeline
//!
//! B1: `SkillUsageAuditSubscriber` — listens for `LlmCallCompleted` and
//!     audits which injected skills were actually used based on output.
//!
//! B2: `ExperienceCaptureSubscriber` — listens for custom `skill_execution`
//!     events and builds `ExperienceRecord` entries.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{debug, info};

use y_core::hook::{Event, EventCategory, EventFilter, EventSubscriber};

// ---------------------------------------------------------------------------
// B1: Skill Usage Audit Subscriber
// ---------------------------------------------------------------------------

/// Shared state for tracking injected skills across LLM calls.
#[derive(Debug, Default)]
pub struct SkillInjectionTracker {
    /// Skills currently injected in the active context, keyed by skill name.
    pub injected_skills: Vec<TrackedSkill>,
}

/// A skill that was injected into the current context.
#[derive(Debug, Clone)]
pub struct TrackedSkill {
    pub name: String,
    pub content_hash: u64,
    /// Excerpt of the skill content for overlap analysis.
    pub content_excerpt: String,
}

impl SkillInjectionTracker {
    /// Record that a skill was injected into the context.
    pub fn record_injection(&mut self, name: String, content_excerpt: String) {
        let content_hash = simple_hash(&content_excerpt);
        self.injected_skills.push(TrackedSkill {
            name,
            content_hash,
            content_excerpt,
        });
    }

    /// Clear tracked injections (e.g., after audit).
    pub fn clear(&mut self) {
        self.injected_skills.clear();
    }
}

fn simple_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Usage audit metrics accumulated over the session.
#[derive(Debug, Default)]
pub struct UsageMetrics {
    /// Per-skill injection and usage counts.
    pub counts: HashMap<String, SkillUsageCounts>,
}

/// Individual skill usage counts.
#[derive(Debug, Default, Clone)]
pub struct SkillUsageCounts {
    pub injection_count: u32,
    pub usage_count: u32,
}

impl UsageMetrics {
    /// Record an injection for a skill.
    pub fn record_injection(&mut self, skill_name: &str) {
        self.counts
            .entry(skill_name.to_string())
            .or_default()
            .injection_count += 1;
    }

    /// Record that a skill was actually used.
    pub fn record_usage(&mut self, skill_name: &str) {
        self.counts
            .entry(skill_name.to_string())
            .or_default()
            .usage_count += 1;
    }

    /// Get usage rate for a skill (0.0–1.0).
    pub fn usage_rate(&self, skill_name: &str) -> Option<f64> {
        self.counts.get(skill_name).map(|c| {
            if c.injection_count == 0 {
                0.0
            } else {
                f64::from(c.usage_count) / f64::from(c.injection_count)
            }
        })
    }
}

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
        if let Event::LlmCallCompleted {
            input_tokens,
            output_tokens,
            ..
        } = event
        {
            let tracker = self.tracker.read().await;
            if tracker.injected_skills.is_empty() {
                return;
            }

            debug!(
                injected_count = tracker.injected_skills.len(),
                input_tokens,
                output_tokens,
                "Auditing skill usage after LLM call"
            );

            let mut metrics = self.metrics.write().await;
            for skill in &tracker.injected_skills {
                metrics.record_injection(&skill.name);
                // Simple heuristic: if the LLM used significant output tokens
                // relative to the skill content, consider it "used".
                // In production, this would use the actual LLM output text for
                // keyword overlap analysis via SkillUsageAudit.audit().
                if *output_tokens > 0 && !skill.content_excerpt.is_empty() {
                    // Placeholder: mark as used if output_tokens > threshold
                    // Real implementation will receive the actual output text
                    // via a richer event payload.
                    let estimated_overlap =
                        f64::from(*output_tokens) / f64::from(*input_tokens).max(1.0);
                    if estimated_overlap >= self.usage_threshold {
                        metrics.record_usage(&skill.name);
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
    store: Arc<RwLock<Vec<CapturedExperience>>>,
}

/// A simplified captured experience record (stored in-memory for now).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapturedExperience {
    /// Skill ID that was executed.
    pub skill_id: String,
    /// Whether the execution was successful.
    pub success: bool,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Token usage.
    pub tokens_used: u32,
    /// Timestamp of capture.
    pub timestamp: String,
}

impl ExperienceCaptureSubscriber {
    /// Create a new experience capture subscriber.
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get the captured experiences.
    pub fn store(&self) -> Arc<RwLock<Vec<CapturedExperience>>> {
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
                let tokens_used = payload
                    .get("tokens_used")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as u32;

                let experience = CapturedExperience {
                    skill_id: skill_id.clone(),
                    success,
                    duration_ms,
                    tokens_used,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };

                let mut store = self.store.write().await;
                store.push(experience);

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
        metrics.record_injection("skill-a");
        metrics.record_injection("skill-a");
        metrics.record_injection("skill-b");
        metrics.record_usage("skill-a");

        assert_eq!(metrics.counts["skill-a"].injection_count, 2);
        assert_eq!(metrics.counts["skill-a"].usage_count, 1);
        assert_eq!(metrics.counts["skill-b"].injection_count, 1);
        assert_eq!(metrics.counts["skill-b"].usage_count, 0);

        assert!((metrics.usage_rate("skill-a").unwrap() - 0.5).abs() < f64::EPSILON);
        assert!((metrics.usage_rate("skill-b").unwrap()).abs() < f64::EPSILON);
    }

    /// T-SK-B1-02: Injection tracker records and clears skills.
    #[test]
    fn test_injection_tracker() {
        let mut tracker = SkillInjectionTracker::default();
        tracker.record_injection("skill-1".into(), "content 1".into());
        tracker.record_injection("skill-2".into(), "content 2".into());

        assert_eq!(tracker.injected_skills.len(), 2);
        assert_eq!(tracker.injected_skills[0].name, "skill-1");

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
        assert_eq!(experiences[0].skill_id, "humanizer");
        assert!(experiences[0].success);
        assert_eq!(experiences[0].duration_ms, 1500);
        assert_eq!(experiences[0].tokens_used, 800);
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
