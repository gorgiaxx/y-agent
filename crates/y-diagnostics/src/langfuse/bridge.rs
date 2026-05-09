//! Background task that bridges diagnostics events to Langfuse via OTLP.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::events::DiagnosticsEvent;
use crate::trace_store::TraceStore;

use super::config::LangfuseConfig;
use super::mapper::OtelSpanMapper;
use super::sender::{OtlpHttpSender, ScorePayload};

struct PendingTrace {
    first_seen: Instant,
}

const TRACE_TTL: Duration = Duration::from_secs(600);
const REAPER_INTERVAL: Duration = Duration::from_secs(60);

pub struct LangfuseExportBridge {
    rx: broadcast::Receiver<DiagnosticsEvent>,
    store: Arc<dyn TraceStore>,
    config: LangfuseConfig,
    mapper: OtelSpanMapper,
    sender: OtlpHttpSender,
}

impl LangfuseExportBridge {
    pub fn new(
        rx: broadcast::Receiver<DiagnosticsEvent>,
        store: Arc<dyn TraceStore>,
        config: LangfuseConfig,
    ) -> Self {
        let mapper = OtelSpanMapper::new(config.clone());
        let sender = OtlpHttpSender::new(&config);
        Self {
            rx,
            store,
            config,
            mapper,
            sender,
        }
    }

    pub async fn run(mut self) {
        info!("Langfuse export bridge started");
        let mut pending: HashMap<Uuid, PendingTrace> = HashMap::new();
        let mut last_reap = Instant::now();

        loop {
            let event = tokio::time::timeout(REAPER_INTERVAL, self.rx.recv()).await;

            match event {
                Ok(Ok(DiagnosticsEvent::TraceCompleted {
                    trace_id,
                    success,
                    agent_name,
                    ..
                })) => {
                    pending.remove(&trace_id);
                    if self.should_export(trace_id, &agent_name) {
                        self.export_trace(trace_id, success).await;
                    }
                }
                Ok(Ok(DiagnosticsEvent::LlmCallStarted { trace_id, .. })) => {
                    pending.entry(trace_id).or_insert(PendingTrace {
                        first_seen: Instant::now(),
                    });
                }
                Ok(Ok(_)) | Err(_) => {}
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    warn!(skipped = n, "Langfuse bridge lagged, skipped events");
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    info!("Broadcast channel closed, shutting down Langfuse bridge");
                    break;
                }
            }

            // Reap abandoned traces.
            if last_reap.elapsed() >= REAPER_INTERVAL {
                let expired: Vec<Uuid> = pending
                    .iter()
                    .filter(|(_, p)| p.first_seen.elapsed() >= TRACE_TTL)
                    .map(|(id, _)| *id)
                    .collect();
                for id in expired {
                    pending.remove(&id);
                    debug!(%id, "Reaped abandoned pending trace");
                }
                last_reap = Instant::now();
            }
        }
    }

    fn should_export(&self, trace_id: Uuid, _agent_name: &str) -> bool {
        let rate = self.config.sampling.rate;
        if rate >= 1.0 {
            return true;
        }
        if rate <= 0.0 {
            return false;
        }
        // Deterministic hash-based sampling.
        let hash = trace_id.as_u128();
        let threshold = (rate * f64::from(u32::MAX)) as u128;
        (hash % u128::from(u32::MAX)) < threshold
    }

    async fn export_trace(&self, trace_id: Uuid, _success: bool) {
        let trace = match self.store.get_trace(trace_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(%trace_id, %e, "Failed to load trace for export");
                return;
            }
        };

        // Check tag-based filtering.
        if !self.config.sampling.include_tags.is_empty()
            && !trace
                .tags
                .iter()
                .any(|t| self.config.sampling.include_tags.contains(t))
        {
            return;
        }
        if trace
            .tags
            .iter()
            .any(|t| self.config.sampling.exclude_tags.contains(t))
        {
            return;
        }

        let observations = self.store.get_observations(trace_id).await.unwrap_or_default();
        let scores = self.store.get_scores(trace_id).await.unwrap_or_default();

        let otlp_request = self.mapper.map_trace(&trace, &observations, &scores);

        if let Err(e) = self.sender.send_traces(&otlp_request).await {
            warn!(%trace_id, %e, "Failed to export trace to Langfuse");
            return;
        }

        debug!(%trace_id, observations = observations.len(), "Exported trace to Langfuse");

        // Push scores via REST API (OTLP doesn't support scores).
        if !scores.is_empty() {
            let payloads: Vec<ScorePayload> = scores
                .iter()
                .map(|s| ScorePayload {
                    id: Some(s.id.to_string()),
                    trace_id: trace_id.to_string(),
                    observation_id: s.observation_id.map(|id| id.to_string()),
                    name: s.name.clone(),
                    value: match &s.value {
                        crate::types::ScoreValue::Numeric(v) => serde_json::json!(*v),
                        crate::types::ScoreValue::Boolean(v) => serde_json::json!(*v),
                        crate::types::ScoreValue::Categorical(v) => serde_json::json!(v),
                    },
                    comment: s.comment.clone(),
                    source: format!("{:?}", s.source),
                })
                .collect();

            if let Err(e) = self.sender.send_scores(&payloads).await {
                warn!(%trace_id, %e, "Failed to export scores to Langfuse");
            }
        }
    }
}
