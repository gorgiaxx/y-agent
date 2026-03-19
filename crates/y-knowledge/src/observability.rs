//! Knowledge observability — tracing spans and event definitions.
//!
//! Provides structured events and tracing instrumentation for
//! knowledge ingestion, retrieval, and context injection operations.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Knowledge Events (for Hook / Event Bus)
// ---------------------------------------------------------------------------

/// Events emitted by the knowledge subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum KnowledgeEvent {
    /// Emitted when ingestion completes.
    IngestionCompleted {
        /// Entry ID of the ingested document.
        entry_id: String,
        /// Number of chunks created.
        chunk_count: usize,
        /// Classified domains.
        domains: Vec<String>,
        /// Quality score.
        quality_score: f32,
        /// Source URI.
        source_uri: String,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Emitted when knowledge is retrieved for a query.
    KnowledgeRetrieved {
        /// Query that triggered retrieval.
        query: String,
        /// Number of results returned.
        result_count: usize,
        /// Search strategy used.
        strategy: String,
        /// Top relevance score.
        top_relevance: f32,
        /// Duration in milliseconds.
        duration_ms: u64,
    },

    /// Emitted when knowledge is injected into context.
    KnowledgeInjected {
        /// Number of chunks injected.
        chunk_count: usize,
        /// Total tokens consumed.
        tokens_used: u32,
        /// Token budget available.
        token_budget: u32,
        /// Domain hint used (if any).
        domain_hint: Option<String>,
    },

    /// Emitted when a staleness check finds entries needing attention.
    MaintenanceAlert {
        /// Number of stale entries found.
        stale_count: usize,
        /// Number of expired entries.
        expired_count: usize,
        /// Entry IDs needing re-ingestion.
        re_ingest_ids: Vec<String>,
    },
}

/// Hook point names for the knowledge subsystem.
///
/// These match event bus / hooks naming convention for integration.
pub mod hook_points {
    /// Fired after successful ingestion.
    pub const KB_INGESTION_COMPLETED: &str = "kb_ingestion_completed";
    /// Fired after successful retrieval.
    pub const KB_KNOWLEDGE_RETRIEVED: &str = "kb_knowledge_retrieved";
    /// Fired after context injection.
    pub const KB_KNOWLEDGE_INJECTED: &str = "kb_knowledge_injected";
    /// Fired when maintenance detects stale entries.
    pub const KB_MAINTENANCE_ALERT: &str = "kb_maintenance_alert";
}

// ---------------------------------------------------------------------------
// Metrics summary
// ---------------------------------------------------------------------------

/// Knowledge subsystem metrics snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeMetrics {
    /// Total ingestion operations.
    pub total_ingestions: u64,
    /// Total retrieval operations.
    pub total_retrievals: u64,
    /// Total context injections.
    pub total_injections: u64,
    /// Average retrieval latency (ms).
    pub avg_retrieval_latency_ms: f64,
    /// Total chunks indexed.
    pub total_chunks_indexed: u64,
    /// Total hits across all chunks.
    pub total_hits: u64,
}

/// Metrics collector for the knowledge subsystem.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    /// Current metrics.
    pub metrics: KnowledgeMetrics,
    /// Running sum of retrieval latencies for average calculation.
    retrieval_latency_sum: f64,
}

impl MetricsCollector {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an ingestion event.
    pub fn record_ingestion(&mut self, chunk_count: u64) {
        self.metrics.total_ingestions += 1;
        self.metrics.total_chunks_indexed += chunk_count;
    }

    /// Record a retrieval event.
    pub fn record_retrieval(&mut self, latency_ms: f64) {
        self.metrics.total_retrievals += 1;
        self.retrieval_latency_sum += latency_ms;
        #[allow(clippy::cast_precision_loss)]
        {
            self.metrics.avg_retrieval_latency_ms =
                self.retrieval_latency_sum / self.metrics.total_retrievals as f64;
        }
    }

    /// Record a context injection event.
    pub fn record_injection(&mut self) {
        self.metrics.total_injections += 1;
    }

    /// Record hits.
    pub fn record_hits(&mut self, count: u64) {
        self.metrics.total_hits += count;
    }

    /// Get current metrics snapshot.
    pub fn snapshot(&self) -> KnowledgeMetrics {
        self.metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_event_serialization() {
        let event = KnowledgeEvent::IngestionCompleted {
            entry_id: "e1".to_string(),
            chunk_count: 5,
            domains: vec!["rust".to_string()],
            quality_score: 0.85,
            source_uri: "/path/to/doc.md".to_string(),
            duration_ms: 120,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("ingestion_completed"));
        assert!(json.contains("e1"));
    }

    #[test]
    fn test_retrieved_event_serialization() {
        let event = KnowledgeEvent::KnowledgeRetrieved {
            query: "rust error".to_string(),
            result_count: 3,
            strategy: "hybrid".to_string(),
            top_relevance: 0.95,
            duration_ms: 45,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("knowledge_retrieved"));
    }

    #[test]
    fn test_hook_point_names() {
        assert_eq!(
            hook_points::KB_INGESTION_COMPLETED,
            "kb_ingestion_completed"
        );
        assert_eq!(
            hook_points::KB_KNOWLEDGE_RETRIEVED,
            "kb_knowledge_retrieved"
        );
    }

    #[test]
    fn test_metrics_collector_ingestion() {
        let mut collector = MetricsCollector::new();
        collector.record_ingestion(10);
        collector.record_ingestion(5);

        let snap = collector.snapshot();
        assert_eq!(snap.total_ingestions, 2);
        assert_eq!(snap.total_chunks_indexed, 15);
    }

    #[test]
    fn test_metrics_collector_retrieval() {
        let mut collector = MetricsCollector::new();
        collector.record_retrieval(100.0);
        collector.record_retrieval(200.0);

        let snap = collector.snapshot();
        assert_eq!(snap.total_retrievals, 2);
        assert!((snap.avg_retrieval_latency_ms - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_collector_injection() {
        let mut collector = MetricsCollector::new();
        collector.record_injection();
        collector.record_injection();

        assert_eq!(collector.snapshot().total_injections, 2);
    }

    #[test]
    fn test_metrics_collector_hits() {
        let mut collector = MetricsCollector::new();
        collector.record_hits(5);
        collector.record_hits(3);

        assert_eq!(collector.snapshot().total_hits, 8);
    }
}
