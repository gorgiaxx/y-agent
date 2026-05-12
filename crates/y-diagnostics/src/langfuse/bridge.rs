//! Background task that bridges diagnostics events to Langfuse via the
//! native REST ingestion API (`POST /api/public/ingestion`).
//!
//! Events are flushed incrementally on a configurable timer so that
//! interrupted traces still appear in Langfuse.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::events::DiagnosticsEvent;
use crate::trace_store::TraceStore;

use super::config::LangfuseConfig;
use super::mapper::LangfuseIngestionMapper;
use super::sender::LangfuseHttpSender;
use super::types::IngestionBatchRequest;

const TRACE_TTL: Duration = Duration::from_secs(600);
const REAPER_INTERVAL: Duration = Duration::from_secs(60);

struct TrackedTrace {
    trace_create_sent: bool,
    sent_observation_ids: HashSet<Uuid>,
    last_activity: Instant,
    sampling_decision: Option<bool>,
    dirty: bool,
}

impl TrackedTrace {
    fn new() -> Self {
        Self {
            trace_create_sent: false,
            sent_observation_ids: HashSet::new(),
            last_activity: Instant::now(),
            sampling_decision: None,
            dirty: true,
        }
    }

    fn touch(&mut self) {
        self.last_activity = Instant::now();
        self.dirty = true;
    }
}

pub struct LangfuseExportBridge {
    rx: broadcast::Receiver<DiagnosticsEvent>,
    store: Arc<dyn TraceStore>,
    config: LangfuseConfig,
    mapper: LangfuseIngestionMapper,
    sender: LangfuseHttpSender,
    shutdown: CancellationToken,
}

impl LangfuseExportBridge {
    pub fn new(
        rx: broadcast::Receiver<DiagnosticsEvent>,
        store: Arc<dyn TraceStore>,
        config: LangfuseConfig,
        shutdown: CancellationToken,
    ) -> Self {
        let mapper = LangfuseIngestionMapper::new(config.clone());
        let sender = LangfuseHttpSender::new(&config);
        Self {
            rx,
            store,
            config,
            mapper,
            sender,
            shutdown,
        }
    }

    pub async fn run(mut self) {
        info!("Langfuse export bridge started (incremental flush)");
        let mut tracked: HashMap<Uuid, TrackedTrace> = HashMap::new();
        let mut last_reap = Instant::now();
        let flush_interval = Duration::from_secs(self.config.flush_interval_secs.max(1));

        loop {
            let event = tokio::select! {
                () = self.shutdown.cancelled() => {
                    self.flush_all(&mut tracked).await;
                    info!("Langfuse export bridge shutting down");
                    break;
                }
                event = tokio::time::timeout(flush_interval, self.rx.recv()) => event,
            };

            match event {
                Ok(Ok(ref ev)) => self.handle_event(ev, &mut tracked).await,
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    warn!(skipped = n, "Langfuse bridge lagged, skipped events");
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    self.flush_all(&mut tracked).await;
                    info!("Broadcast channel closed, shutting down Langfuse bridge");
                    break;
                }
                Err(_) => {}
            }

            self.flush_dirty(&mut tracked).await;

            if last_reap.elapsed() >= REAPER_INTERVAL {
                self.reap_abandoned(&mut tracked).await;
                last_reap = Instant::now();
            }
        }
    }

    async fn handle_event(
        &self,
        event: &DiagnosticsEvent,
        tracked: &mut HashMap<Uuid, TrackedTrace>,
    ) {
        let Some(trace_id) = extract_trace_id(event) else {
            return;
        };

        let entry = tracked.entry(trace_id).or_insert_with(TrackedTrace::new);
        entry.touch();

        if let DiagnosticsEvent::TraceCompleted {
            trace_id,
            success,
            agent_name,
            ..
        } = event
        {
            self.flush_trace(*trace_id, entry).await;
            self.send_trace_update(*trace_id, *success, agent_name)
                .await;
            tracked.remove(trace_id);
        }
    }

    async fn flush_dirty(&self, tracked: &mut HashMap<Uuid, TrackedTrace>) {
        let dirty_ids: Vec<Uuid> = tracked
            .iter()
            .filter(|(_, t)| t.dirty)
            .map(|(id, _)| *id)
            .collect();

        if dirty_ids.is_empty() {
            return;
        }

        let mut batch = Vec::new();

        for trace_id in &dirty_ids {
            let Some(entry) = tracked.get_mut(trace_id) else {
                continue;
            };

            if entry.sampling_decision.is_none() {
                entry.sampling_decision = Some(self.should_export(*trace_id));
            }
            if entry.sampling_decision == Some(false) {
                entry.dirty = false;
                continue;
            }

            self.collect_incremental_events(*trace_id, entry, &mut batch)
                .await;
            entry.dirty = false;
        }

        if !batch.is_empty() {
            let request = IngestionBatchRequest { batch };
            if let Err(e) = self.sender.send_batch(&request).await {
                warn!(%e, "Incremental flush to Langfuse failed");
            }
        }
    }

    async fn flush_trace(&self, trace_id: Uuid, entry: &mut TrackedTrace) {
        if entry.sampling_decision.is_none() {
            entry.sampling_decision = Some(self.should_export(trace_id));
        }
        if entry.sampling_decision == Some(false) {
            return;
        }

        let mut batch = Vec::new();
        self.collect_incremental_events(trace_id, entry, &mut batch)
            .await;
        if !batch.is_empty() {
            let request = IngestionBatchRequest { batch };
            if let Err(e) = self.sender.send_batch(&request).await {
                warn!(%trace_id, %e, "Final flush to Langfuse failed");
            }
        }
    }

    async fn flush_all(&self, tracked: &mut HashMap<Uuid, TrackedTrace>) {
        let ids: Vec<Uuid> = tracked.keys().copied().collect();
        for trace_id in ids {
            if let Some(entry) = tracked.get_mut(&trace_id) {
                self.flush_trace(trace_id, entry).await;
                if entry.trace_create_sent {
                    self.send_trace_update(trace_id, false, "").await;
                }
            }
        }
        tracked.clear();
    }

    async fn collect_incremental_events(
        &self,
        trace_id: Uuid,
        entry: &mut TrackedTrace,
        batch: &mut Vec<super::types::IngestionEvent>,
    ) {
        if !entry.trace_create_sent {
            match self.store.get_trace(trace_id).await {
                Ok(trace) => {
                    if !self.passes_tag_filter(&trace.tags) {
                        entry.sampling_decision = Some(false);
                        return;
                    }
                    batch.push(self.mapper.map_trace_create(&trace));
                    entry.trace_create_sent = true;
                }
                Err(e) => {
                    warn!(%trace_id, %e, "Failed to load trace for incremental export");
                    return;
                }
            }
        }

        let Ok(trace) = self.store.get_trace(trace_id).await else {
            return;
        };

        let observations = self
            .store
            .get_observations(trace_id)
            .await
            .unwrap_or_default();

        for obs in &observations {
            if entry.sent_observation_ids.contains(&obs.id) {
                continue;
            }
            batch.push(self.mapper.map_observation(&trace, obs));
            entry.sent_observation_ids.insert(obs.id);
        }
    }

    async fn send_trace_update(&self, trace_id: Uuid, _success: bool, _agent_name: &str) {
        let trace = match self.store.get_trace(trace_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(%trace_id, %e, "Failed to load trace for update export");
                return;
            }
        };

        let event = self.mapper.map_trace_update(&trace);
        let request = IngestionBatchRequest { batch: vec![event] };
        if let Err(e) = self.sender.send_batch(&request).await {
            warn!(%trace_id, %e, "Failed to send trace update to Langfuse");
        } else {
            debug!(%trace_id, "Exported trace update to Langfuse");
        }
    }

    async fn reap_abandoned(&self, tracked: &mut HashMap<Uuid, TrackedTrace>) {
        let expired: Vec<Uuid> = tracked
            .iter()
            .filter(|(_, t)| t.last_activity.elapsed() >= TRACE_TTL)
            .map(|(id, _)| *id)
            .collect();

        for id in expired {
            if let Some(entry) = tracked.get_mut(&id) {
                self.flush_trace(id, entry).await;
                if entry.trace_create_sent {
                    self.send_trace_update(id, false, "").await;
                }
            }
            tracked.remove(&id);
            debug!(%id, "Reaped abandoned pending trace");
        }
    }

    fn should_export(&self, trace_id: Uuid) -> bool {
        let rate = self.config.sampling.rate;
        if rate >= 1.0 {
            return true;
        }
        if rate <= 0.0 {
            return false;
        }
        let hash = trace_id.as_u128();
        let threshold = (rate * f64::from(u32::MAX)) as u128;
        (hash % u128::from(u32::MAX)) < threshold
    }

    fn passes_tag_filter(&self, tags: &[String]) -> bool {
        if !self.config.sampling.include_tags.is_empty()
            && !tags
                .iter()
                .any(|t| self.config.sampling.include_tags.contains(t))
        {
            return false;
        }
        !tags
            .iter()
            .any(|t| self.config.sampling.exclude_tags.contains(t))
    }
}

fn extract_trace_id(event: &DiagnosticsEvent) -> Option<Uuid> {
    match event {
        DiagnosticsEvent::LlmCallStarted { trace_id, .. }
        | DiagnosticsEvent::LlmCallCompleted { trace_id, .. }
        | DiagnosticsEvent::LlmCallFailed { trace_id, .. }
        | DiagnosticsEvent::ToolCallCompleted { trace_id, .. }
        | DiagnosticsEvent::SubagentCompleted { trace_id, .. }
        | DiagnosticsEvent::TraceCompleted { trace_id, .. } => Some(*trace_id),
        DiagnosticsEvent::StreamDelta { .. } | DiagnosticsEvent::StreamReasoningDelta { .. } => {
            None
        }
    }
}
