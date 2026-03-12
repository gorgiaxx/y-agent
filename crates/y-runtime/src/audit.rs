//! Audit trail for tracking all tool/runtime operations.
//!
//! Provides a structured, append-only log of every action taken within
//! a runtime environment. Used for compliance, debugging, and replay.
//!
//! Design reference: runtime-design.md §Observability

use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Maximum audit entries to keep in memory (ring-buffer behavior).
const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// A single audit trail entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Type of event.
    pub event_type: AuditEventType,
    /// Who triggered the event.
    pub actor: String,
    /// Target resource or tool.
    pub target: String,
    /// Outcome: success or failure with reason.
    pub outcome: AuditOutcome,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Categories of audit events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Tool execution request.
    ToolExecution,
    /// File system operation.
    FileOperation,
    /// Network request.
    NetworkRequest,
    /// Container lifecycle event.
    ContainerLifecycle,
    /// Permission check.
    PermissionCheck,
    /// Configuration change.
    ConfigChange,
    /// Resource limit event (threshold reached or exceeded).
    ResourceLimit,
    /// Custom event type.
    Custom(String),
}

/// Outcome of an audited operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// Operation succeeded.
    Success,
    /// Operation was denied (permission, rate limit, etc.).
    Denied { reason: String },
    /// Operation failed with an error.
    Failed { error: String },
}

/// Thread-safe audit trail with ring-buffer storage.
#[derive(Debug, Clone)]
pub struct AuditTrail {
    inner: Arc<Mutex<AuditTrailInner>>,
}

#[derive(Debug)]
struct AuditTrailInner {
    entries: VecDeque<AuditEntry>,
    max_entries: usize,
    total_logged: u64,
}

impl AuditTrail {
    /// Create a new audit trail with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }

    /// Create a new audit trail with a custom capacity.
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AuditTrailInner {
                entries: VecDeque::with_capacity(max_entries.min(1024)),
                max_entries,
                total_logged: 0,
            })),
        }
    }

    /// Log a new audit entry.
    pub async fn log(&self, entry: AuditEntry) {
        let mut inner = self.inner.lock().await;
        inner.total_logged += 1;
        if inner.entries.len() >= inner.max_entries {
            inner.entries.pop_front();
        }
        inner.entries.push_back(entry);
    }

    /// Log a tool execution event.
    pub async fn log_tool_execution(
        &self,
        actor: &str,
        tool_name: &str,
        outcome: AuditOutcome,
        metadata: Option<serde_json::Value>,
    ) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            event_type: AuditEventType::ToolExecution,
            actor: actor.to_string(),
            target: tool_name.to_string(),
            outcome,
            metadata: metadata.unwrap_or(serde_json::Value::Null),
        })
        .await;
    }

    /// Log a permission check event.
    pub async fn log_permission_check(&self, actor: &str, resource: &str, outcome: AuditOutcome) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            event_type: AuditEventType::PermissionCheck,
            actor: actor.to_string(),
            target: resource.to_string(),
            outcome,
            metadata: serde_json::Value::Null,
        })
        .await;
    }

    /// Get the total number of entries logged (including evicted).
    pub async fn total_logged(&self) -> u64 {
        let inner = self.inner.lock().await;
        inner.total_logged
    }

    /// Get the number of entries currently in the buffer.
    pub async fn current_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.entries.len()
    }

    /// Get recent entries (up to `limit`), most recent first.
    pub async fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        let inner = self.inner.lock().await;
        inner.entries.iter().rev().take(limit).cloned().collect()
    }

    /// Query entries by event type.
    pub async fn query_by_type(&self, event_type: &AuditEventType) -> Vec<AuditEntry> {
        let inner = self.inner.lock().await;
        inner
            .entries
            .iter()
            .filter(|e| &e.event_type == event_type)
            .cloned()
            .collect()
    }

    /// Query entries by actor.
    pub async fn query_by_actor(&self, actor: &str) -> Vec<AuditEntry> {
        let inner = self.inner.lock().await;
        inner
            .entries
            .iter()
            .filter(|e| e.actor == actor)
            .cloned()
            .collect()
    }

    /// Clear all entries.
    pub async fn clear(&self) {
        let mut inner = self.inner.lock().await;
        inner.entries.clear();
    }
}

impl Default for AuditTrail {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audit_log_and_retrieve() {
        let audit = AuditTrail::new();
        audit
            .log_tool_execution("agent-1", "web_search", AuditOutcome::Success, None)
            .await;

        assert_eq!(audit.current_count().await, 1);
        assert_eq!(audit.total_logged().await, 1);

        let recent = audit.recent(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].actor, "agent-1");
        assert_eq!(recent[0].target, "web_search");
    }

    #[tokio::test]
    async fn test_audit_ring_buffer_eviction() {
        let audit = AuditTrail::with_capacity(3);

        for i in 0..5 {
            audit
                .log_tool_execution(&format!("agent-{i}"), "tool", AuditOutcome::Success, None)
                .await;
        }

        // Only the last 3 should be retained.
        assert_eq!(audit.current_count().await, 3);
        assert_eq!(audit.total_logged().await, 5);

        let recent = audit.recent(10).await;
        assert_eq!(recent[0].actor, "agent-4"); // Most recent.
        assert_eq!(recent[2].actor, "agent-2"); // Oldest still in buffer.
    }

    #[tokio::test]
    async fn test_audit_query_by_type() {
        let audit = AuditTrail::new();
        audit
            .log_tool_execution("agent", "tool1", AuditOutcome::Success, None)
            .await;
        audit
            .log_permission_check("agent", "file.txt", AuditOutcome::Success)
            .await;
        audit
            .log_tool_execution("agent", "tool2", AuditOutcome::Success, None)
            .await;

        let tool_events = audit.query_by_type(&AuditEventType::ToolExecution).await;
        assert_eq!(tool_events.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_query_by_actor() {
        let audit = AuditTrail::new();
        audit
            .log_tool_execution("alice", "tool1", AuditOutcome::Success, None)
            .await;
        audit
            .log_tool_execution("bob", "tool2", AuditOutcome::Success, None)
            .await;
        audit
            .log_tool_execution("alice", "tool3", AuditOutcome::Success, None)
            .await;

        let alice_events = audit.query_by_actor("alice").await;
        assert_eq!(alice_events.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_denied_outcome() {
        let audit = AuditTrail::new();
        audit
            .log_permission_check(
                "agent",
                "/etc/passwd",
                AuditOutcome::Denied {
                    reason: "filesystem access denied".into(),
                },
            )
            .await;

        let recent = audit.recent(1).await;
        assert_eq!(
            recent[0].outcome,
            AuditOutcome::Denied {
                reason: "filesystem access denied".into()
            }
        );
    }

    #[tokio::test]
    async fn test_audit_clear() {
        let audit = AuditTrail::new();
        audit
            .log_tool_execution("agent", "tool", AuditOutcome::Success, None)
            .await;
        assert_eq!(audit.current_count().await, 1);

        audit.clear().await;
        assert_eq!(audit.current_count().await, 0);
        // total_logged is not reset by clear.
        assert_eq!(audit.total_logged().await, 1);
    }

    #[test]
    fn test_audit_entry_serialization() {
        let entry = AuditEntry {
            timestamp: Utc::now(),
            event_type: AuditEventType::ToolExecution,
            actor: "agent-1".into(),
            target: "web_search".into(),
            outcome: AuditOutcome::Success,
            metadata: serde_json::json!({"query": "rust async"}),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let deserialized: AuditEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.actor, "agent-1");
        assert_eq!(deserialized.event_type, AuditEventType::ToolExecution);
    }
}
