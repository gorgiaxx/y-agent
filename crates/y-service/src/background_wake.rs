//! Policy and orchestration for bounded background-task completion wake-ups.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use y_core::runtime::{ToolRuntimeEvent, ToolRuntimeEventKind};
use y_core::types::SessionId;

use crate::chat::TurnEvent;
use crate::chat::{ChatService, PrepareTurnRequest};
use crate::chat_types::TurnMeta;
use crate::container::ServiceContainer;
use crate::event_sink::EventSink;

const WAKE_EVENT_CHANNEL_CAPACITY: usize = 256;
const OBSERVATION_WAIT_TIMEOUT: StdDuration = StdDuration::from_secs(3);
const OBSERVATION_RECHECK_INTERVAL: StdDuration = StdDuration::from_millis(10);
const COMPLETION_PROMPT_TEMPLATE: &str =
    include_str!("../../../config/prompts/background-task-completion.md");

/// Configuration for automatic turns triggered by background-task completion.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackgroundWakeConfig {
    /// Master switch. The subsystem is inert unless explicitly enabled.
    pub enabled: bool,
    /// Maximum successfully started wake turns per session in a rolling hour.
    pub max_wakes_per_hour: usize,
    /// Minimum delay between successfully started wake turns in one session.
    pub cooldown_secs: u64,
    /// Whether a completion may wake while an explicit Plan execution is active.
    pub allow_during_orchestration: bool,
}

impl Default for BackgroundWakeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_wakes_per_hour: 2,
            cooldown_secs: 300,
            allow_during_orchestration: false,
        }
    }
}

/// Service-owned session activity evaluated at the moment a completion arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundWakeContext {
    pub turn_active: bool,
    pub orchestration_active: bool,
}

impl BackgroundWakeContext {
    pub fn idle() -> Self {
        Self {
            turn_active: false,
            orchestration_active: false,
        }
    }
}

/// Stable reason an event was not admitted for automatic delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundWakeSuppression {
    Disabled,
    IneligibleEvent,
    SessionBusy,
    OrchestrationActive,
    AlreadyConsumed,
    UserKilled,
    ResultObservationInProgress,
    AlreadyReservedOrDelivered,
    Cooldown,
    HourlyBudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskKey {
    session_id: String,
    task_id: String,
}

impl TaskKey {
    fn new(session_id: &SessionId, task_id: &str) -> Self {
        Self {
            session_id: session_id.as_str().to_string(),
            task_id: task_id.to_string(),
        }
    }
}

/// Atomic claim for one eligible runtime completion.
#[derive(Debug)]
pub struct BackgroundWakeReservation {
    key: TaskKey,
}

impl BackgroundWakeReservation {
    pub fn session_id(&self) -> &str {
        &self.key.session_id
    }

    pub fn task_id(&self) -> &str {
        &self.key.task_id
    }
}

#[derive(Debug, Default)]
struct BackgroundWakeState {
    consumed: HashSet<TaskKey>,
    killed: HashSet<TaskKey>,
    observing: HashSet<TaskKey>,
    reserved: HashSet<TaskKey>,
    delivered: HashSet<TaskKey>,
    successful_wakes: HashMap<String, VecDeque<DateTime<Utc>>>,
}

/// Thread-safe admission policy for completion-triggered turns.
pub struct BackgroundWakePolicy {
    config: BackgroundWakeConfig,
    state: Mutex<BackgroundWakeState>,
}

impl BackgroundWakePolicy {
    pub fn new(config: BackgroundWakeConfig) -> Self {
        Self {
            config,
            state: Mutex::new(BackgroundWakeState::default()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn mark_consumed(&self, session_id: &SessionId, task_id: &str) {
        let key = TaskKey::new(session_id, task_id);
        let mut state = self.lock_state();
        state.observing.remove(&key);
        state.reserved.remove(&key);
        state.consumed.insert(key);
    }

    pub fn mark_killed(&self, session_id: &SessionId, task_id: &str) {
        let key = TaskKey::new(session_id, task_id);
        let mut state = self.lock_state();
        state.observing.remove(&key);
        state.reserved.remove(&key);
        state.killed.insert(key);
    }

    pub fn begin_observation(&self, session_id: &SessionId, task_id: &str) {
        self.lock_state()
            .observing
            .insert(TaskKey::new(session_id, task_id));
    }

    pub fn finish_observation(&self, session_id: &SessionId, task_id: &str, consumed: bool) {
        let key = TaskKey::new(session_id, task_id);
        let mut state = self.lock_state();
        state.observing.remove(&key);
        if consumed {
            state.reserved.remove(&key);
            state.consumed.insert(key);
        }
    }

    pub fn is_observing(&self, session_id: &SessionId, task_id: &str) -> bool {
        self.lock_state()
            .observing
            .contains(&TaskKey::new(session_id, task_id))
    }

    pub fn try_reserve(
        &self,
        event: &ToolRuntimeEvent,
        context: BackgroundWakeContext,
        now: DateTime<Utc>,
    ) -> Result<BackgroundWakeReservation, BackgroundWakeSuppression> {
        if !self.config.enabled {
            return Err(BackgroundWakeSuppression::Disabled);
        }
        if !matches!(
            event.kind,
            ToolRuntimeEventKind::ProcessCompleted { .. }
                | ToolRuntimeEventKind::ProcessFailed { .. }
        ) {
            return Err(BackgroundWakeSuppression::IneligibleEvent);
        }
        if context.turn_active {
            return Err(BackgroundWakeSuppression::SessionBusy);
        }
        if context.orchestration_active && !self.config.allow_during_orchestration {
            return Err(BackgroundWakeSuppression::OrchestrationActive);
        }

        let key = TaskKey::new(&event.session_id, &event.task_id);
        let mut state = self.lock_state();
        if state.consumed.contains(&key) {
            return Err(BackgroundWakeSuppression::AlreadyConsumed);
        }
        if state.killed.contains(&key) {
            return Err(BackgroundWakeSuppression::UserKilled);
        }
        if state.observing.contains(&key) {
            return Err(BackgroundWakeSuppression::ResultObservationInProgress);
        }
        if state.reserved.contains(&key) || state.delivered.contains(&key) {
            return Err(BackgroundWakeSuppression::AlreadyReservedOrDelivered);
        }

        let history = state
            .successful_wakes
            .entry(key.session_id.clone())
            .or_default();
        let cutoff = now - Duration::hours(1);
        while history
            .front()
            .is_some_and(|timestamp| *timestamp <= cutoff)
        {
            history.pop_front();
        }
        if let Some(last_wake) = history.back() {
            let cooldown_secs = i64::try_from(self.config.cooldown_secs).unwrap_or(i64::MAX);
            let cooldown = Duration::seconds(cooldown_secs);
            if now.signed_duration_since(*last_wake) < cooldown {
                return Err(BackgroundWakeSuppression::Cooldown);
            }
        }
        if history.len() >= self.config.max_wakes_per_hour {
            return Err(BackgroundWakeSuppression::HourlyBudgetExhausted);
        }

        state.reserved.insert(key.clone());
        Ok(BackgroundWakeReservation { key })
    }

    pub fn commit(&self, reservation: &BackgroundWakeReservation, now: DateTime<Utc>) {
        let mut state = self.lock_state();
        if state.reserved.remove(&reservation.key) {
            state.delivered.insert(reservation.key.clone());
            state
                .successful_wakes
                .entry(reservation.key.session_id.clone())
                .or_default()
                .push_back(now);
        }
    }

    pub fn release(&self, reservation: &BackgroundWakeReservation) {
        self.lock_state().reserved.remove(&reservation.key);
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, BackgroundWakeState> {
        self.state
            .lock()
            .expect("background wake policy mutex poisoned")
    }
}

/// Chat lifecycle event emitted by an automatic wake turn.
#[derive(Debug, Clone)]
pub enum BackgroundWakeEvent {
    Started {
        run_id: String,
        session_id: String,
        event_id: Option<u64>,
    },
    Progress {
        run_id: String,
        session_id: String,
        event: TurnEvent,
        child_session_id: Option<String>,
        event_id: Option<u64>,
    },
    AskUser {
        run_id: String,
        session_id: String,
        interaction_id: String,
        questions: serde_json::Value,
        event_id: Option<u64>,
    },
    PermissionRequest {
        run_id: String,
        session_id: String,
        request_id: String,
        tool_name: String,
        action_description: String,
        reason: String,
        content_preview: Option<String>,
        event_id: Option<u64>,
    },
    PlanReviewRequest {
        run_id: String,
        session_id: String,
        review_id: String,
        plan: serde_json::Value,
        event_id: Option<u64>,
    },
    Complete {
        session_id: String,
        payload: serde_json::Value,
        event_id: Option<u64>,
    },
    Error {
        run_id: String,
        session_id: String,
        error: String,
        event_id: Option<u64>,
    },
    TitleUpdated {
        session_id: String,
        title: String,
        event_id: Option<u64>,
    },
}

impl BackgroundWakeEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Started { .. } => "chat:started",
            Self::Progress { .. } => "chat:progress",
            Self::AskUser { .. } => "chat:AskUser",
            Self::PermissionRequest { .. } => "chat:PermissionRequest",
            Self::PlanReviewRequest { .. } => "chat:PlanReview",
            Self::Complete { .. } => "chat:complete",
            Self::Error { .. } => "chat:error",
            Self::TitleUpdated { .. } => "session:title_updated",
        }
    }

    pub fn event_id(&self) -> Option<u64> {
        match self {
            Self::Started { event_id, .. }
            | Self::Progress { event_id, .. }
            | Self::AskUser { event_id, .. }
            | Self::PermissionRequest { event_id, .. }
            | Self::PlanReviewRequest { event_id, .. }
            | Self::Complete { event_id, .. }
            | Self::Error { event_id, .. }
            | Self::TitleUpdated { event_id, .. } => *event_id,
        }
    }

    pub fn session_id(&self) -> &str {
        match self {
            Self::Started { session_id, .. }
            | Self::Progress { session_id, .. }
            | Self::AskUser { session_id, .. }
            | Self::PermissionRequest { session_id, .. }
            | Self::PlanReviewRequest { session_id, .. }
            | Self::Complete { session_id, .. }
            | Self::Error { session_id, .. }
            | Self::TitleUpdated { session_id, .. } => session_id,
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        match self {
            Self::Started {
                run_id, session_id, ..
            } => serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "kind": "background_auto_wake",
            }),
            Self::Progress {
                run_id,
                event,
                child_session_id,
                ..
            } => serde_json::json!({
                "run_id": run_id,
                "event": event,
                "session_id": child_session_id,
            }),
            Self::AskUser {
                run_id,
                session_id,
                interaction_id,
                questions,
                ..
            } => serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "interaction_id": interaction_id,
                "questions": questions,
            }),
            Self::PermissionRequest {
                run_id,
                session_id,
                request_id,
                tool_name,
                action_description,
                reason,
                content_preview,
                ..
            } => serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "request_id": request_id,
                "tool_name": tool_name,
                "action_description": action_description,
                "reason": reason,
                "content_preview": content_preview,
            }),
            Self::PlanReviewRequest {
                run_id,
                session_id,
                review_id,
                plan,
                ..
            } => serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "review_id": review_id,
                "plan": plan,
            }),
            Self::Complete { payload, .. } => payload.clone(),
            Self::Error {
                run_id,
                session_id,
                error,
                ..
            } => serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "error": error,
            }),
            Self::TitleUpdated {
                session_id, title, ..
            } => serde_json::json!({
                "session_id": session_id,
                "title": title,
            }),
        }
    }
}

/// Service-owned state and live channel for automatic wake turns.
#[derive(Clone)]
pub struct BackgroundWakeService {
    policy: Arc<BackgroundWakePolicy>,
    event_tx: broadcast::Sender<BackgroundWakeEvent>,
    pending_runs: Arc<Mutex<HashMap<String, CancellationToken>>>,
    turn_meta_cache: Arc<Mutex<HashMap<String, TurnMeta>>>,
    started: Arc<AtomicBool>,
}

impl BackgroundWakeService {
    pub fn new(config: BackgroundWakeConfig) -> Self {
        let (event_tx, _) = broadcast::channel(WAKE_EVENT_CHANNEL_CAPACITY);
        Self {
            policy: Arc::new(BackgroundWakePolicy::new(config)),
            event_tx,
            pending_runs: Arc::new(Mutex::new(HashMap::new())),
            turn_meta_cache: Arc::new(Mutex::new(HashMap::new())),
            started: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(&self, container: Arc<ServiceContainer>) {
        if !self.policy.is_enabled() || self.started.swap(true, Ordering::AcqRel) {
            return;
        }

        let mut receiver = container.tool_runtime_event_service.subscribe();
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(published) => service.handle_runtime_event(&container, published.event).await,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => tracing::warn!(
                        skipped,
                        "background wake runtime channel lagged; completions were conservatively skipped"
                    ),
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BackgroundWakeEvent> {
        self.event_tx.subscribe()
    }

    pub fn is_enabled(&self) -> bool {
        self.policy.is_enabled()
    }

    pub fn event_sink(&self) -> impl EventSink {
        BackgroundWakeEventSink {
            tx: self.event_tx.clone(),
            root_session_id: Arc::new(Mutex::new(None)),
        }
    }

    pub fn mark_consumed(&self, session_id: &SessionId, task_id: &str) {
        self.policy.mark_consumed(session_id, task_id);
    }

    pub fn mark_killed(&self, session_id: &SessionId, task_id: &str) {
        self.policy.mark_killed(session_id, task_id);
    }

    pub fn begin_observation(&self, session_id: &SessionId, task_id: &str) {
        self.policy.begin_observation(session_id, task_id);
    }

    pub fn finish_observation(&self, session_id: &SessionId, task_id: &str, consumed: bool) {
        self.policy
            .finish_observation(session_id, task_id, consumed);
    }

    pub fn cancel(&self, run_id: &str) -> bool {
        let token = match self.pending_runs.lock() {
            Ok(mut pending_runs) => pending_runs.remove(run_id),
            Err(error) => {
                tracing::error!(%error, "background wake pending-runs mutex poisoned");
                return false;
            }
        };
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    async fn handle_runtime_event(
        &self,
        container: &Arc<ServiceContainer>,
        event: ToolRuntimeEvent,
    ) {
        if matches!(event.kind, ToolRuntimeEventKind::ProcessKilled { .. }) {
            self.mark_killed(&event.session_id, &event.task_id);
            return;
        }
        if !matches!(
            event.kind,
            ToolRuntimeEventKind::ProcessCompleted { .. }
                | ToolRuntimeEventKind::ProcessFailed { .. }
        ) {
            return;
        }

        match self.admit_terminal_event(container, &event).await {
            Ok(reservation) => self.spawn_wake_turn(Arc::clone(container), event, reservation),
            Err(BackgroundWakeSuppression::ResultObservationInProgress) => {
                self.defer_until_observation_finishes(Arc::clone(container), event);
            }
            Err(reason) => log_suppression(&event, reason),
        }
    }

    async fn admit_terminal_event(
        &self,
        container: &ServiceContainer,
        event: &ToolRuntimeEvent,
    ) -> Result<BackgroundWakeReservation, BackgroundWakeSuppression> {
        let context = BackgroundWakeContext {
            turn_active: container
                .session_state
                .is_turn_active(&event.session_id)
                .await,
            orchestration_active: container
                .session_state
                .is_orchestration_active(&event.session_id)
                .await,
        };
        self.policy.try_reserve(event, context, Utc::now())
    }

    fn spawn_wake_turn(
        &self,
        container: Arc<ServiceContainer>,
        event: ToolRuntimeEvent,
        reservation: BackgroundWakeReservation,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            service.start_wake_turn(container, event, reservation).await;
        });
    }

    fn defer_until_observation_finishes(
        &self,
        container: Arc<ServiceContainer>,
        event: ToolRuntimeEvent,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + OBSERVATION_WAIT_TIMEOUT;
            while service
                .policy
                .is_observing(&event.session_id, &event.task_id)
            {
                if tokio::time::Instant::now() >= deadline {
                    tracing::debug!(
                        session_id = %event.session_id,
                        task_id = %event.task_id,
                        "background completion observation did not finish before timeout"
                    );
                    return;
                }
                tokio::time::sleep(OBSERVATION_RECHECK_INTERVAL).await;
            }

            match service.admit_terminal_event(&container, &event).await {
                Ok(reservation) => service.spawn_wake_turn(container, event, reservation),
                Err(reason) => log_suppression(&event, reason),
            }
        });
    }

    async fn start_wake_turn(
        &self,
        container: Arc<ServiceContainer>,
        event: ToolRuntimeEvent,
        reservation: BackgroundWakeReservation,
    ) {
        let request = PrepareTurnRequest {
            session_id: Some(event.session_id.clone()),
            user_input: render_completion_prompt(&event),
            skills: Some(Vec::new()),
            user_message_metadata: Some(serde_json::json!({
                "kind": "background_auto_wake",
                "task_id": event.task_id,
                "runtime_status": runtime_status(&event.kind),
                "runtime_occurred_at": event.occurred_at,
            })),
            ..PrepareTurnRequest::default()
        };
        let prepared = match ChatService::prepare_turn(&container, request).await {
            Ok(prepared) => prepared,
            Err(error) => {
                self.policy.release(&reservation);
                tracing::warn!(
                    session_id = %event.session_id,
                    task_id = %event.task_id,
                    %error,
                    "failed to prepare background completion wake turn"
                );
                return;
            }
        };

        let run_id = format!("background-wake-{}", uuid::Uuid::new_v4());
        let cancel_token = CancellationToken::new();
        self.pending_runs
            .lock()
            .expect("background wake pending-runs mutex poisoned")
            .insert(run_id.clone(), cancel_token.clone());
        let start_result = crate::chat_worker::spawn_llm_worker(
            self.event_sink(),
            Arc::clone(&container),
            prepared,
            run_id.clone(),
            Arc::clone(&self.turn_meta_cache),
            Arc::clone(&self.pending_runs),
            cancel_token,
            "background_auto_wake",
            false,
        )
        .await;

        match start_result {
            Ok(()) => {
                self.policy.commit(&reservation, Utc::now());
                tracing::info!(
                    session_id = %event.session_id,
                    task_id = %event.task_id,
                    %run_id,
                    "started background completion wake turn"
                );
            }
            Err(error) => {
                self.pending_runs
                    .lock()
                    .expect("background wake pending-runs mutex poisoned")
                    .remove(&run_id);
                self.policy.release(&reservation);
                tracing::error!(
                    session_id = %event.session_id,
                    task_id = %event.task_id,
                    %run_id,
                    %error,
                    "failed to start background completion wake turn"
                );
            }
        }
    }
}

struct BackgroundWakeEventSink {
    tx: broadcast::Sender<BackgroundWakeEvent>,
    root_session_id: Arc<Mutex<Option<String>>>,
}

impl BackgroundWakeEventSink {
    fn send(&self, event: BackgroundWakeEvent) {
        let _ = self.tx.send(event);
    }

    fn root_session_id(&self) -> String {
        self.root_session_id
            .lock()
            .expect("background wake event-sink mutex poisoned")
            .clone()
            .unwrap_or_default()
    }
}

impl EventSink for BackgroundWakeEventSink {
    fn emit_started(&self, run_id: &str, session_id: &str, event_id: Option<u64>) {
        *self
            .root_session_id
            .lock()
            .expect("background wake event-sink mutex poisoned") = Some(session_id.to_string());
        self.send(BackgroundWakeEvent::Started {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            event_id,
        });
    }

    fn emit_progress(
        &self,
        run_id: &str,
        event: &TurnEvent,
        child_session_id: Option<&str>,
        event_id: Option<u64>,
    ) {
        self.send(BackgroundWakeEvent::Progress {
            run_id: run_id.to_string(),
            session_id: self.root_session_id(),
            event: event.clone(),
            child_session_id: child_session_id.map(str::to_string),
            event_id,
        });
    }

    fn emit_ask_user(
        &self,
        run_id: &str,
        session_id: &str,
        interaction_id: &str,
        questions: &serde_json::Value,
        event_id: Option<u64>,
    ) {
        self.send(BackgroundWakeEvent::AskUser {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            interaction_id: interaction_id.to_string(),
            questions: questions.clone(),
            event_id,
        });
    }

    fn emit_permission_request(
        &self,
        run_id: &str,
        session_id: &str,
        request_id: &str,
        tool_name: &str,
        action_description: &str,
        reason: &str,
        content_preview: Option<&str>,
        event_id: Option<u64>,
    ) {
        self.send(BackgroundWakeEvent::PermissionRequest {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            action_description: action_description.to_string(),
            reason: reason.to_string(),
            content_preview: content_preview.map(str::to_string),
            event_id,
        });
    }

    fn emit_plan_review_request(
        &self,
        run_id: &str,
        session_id: &str,
        review_id: &str,
        plan: &serde_json::Value,
        event_id: Option<u64>,
    ) {
        self.send(BackgroundWakeEvent::PlanReviewRequest {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            review_id: review_id.to_string(),
            plan: plan.clone(),
            event_id,
        });
    }

    fn emit_complete(&self, payload: &serde_json::Value, event_id: Option<u64>) {
        self.send(BackgroundWakeEvent::Complete {
            session_id: self.root_session_id(),
            payload: payload.clone(),
            event_id,
        });
    }

    fn emit_error(&self, run_id: &str, session_id: &str, error: &str, event_id: Option<u64>) {
        self.send(BackgroundWakeEvent::Error {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            error: error.to_string(),
            event_id,
        });
    }

    fn emit_title_updated(&self, session_id: &str, title: &str, event_id: Option<u64>) {
        self.send(BackgroundWakeEvent::TitleUpdated {
            session_id: session_id.to_string(),
            title: title.to_string(),
            event_id,
        });
    }
}

fn render_completion_prompt(event: &ToolRuntimeEvent) -> String {
    let (status, detail) = match &event.kind {
        ToolRuntimeEventKind::ProcessCompleted {
            exit_code,
            duration_ms,
        } => (
            "completed",
            format!("exit_code={exit_code}, duration_ms={duration_ms}"),
        ),
        ToolRuntimeEventKind::ProcessFailed { error, duration_ms } => (
            "failed",
            format!("error={error}, duration_ms={duration_ms}"),
        ),
        _ => ("ineligible", String::new()),
    };
    COMPLETION_PROMPT_TEMPLATE
        .replace("{{task_id}}", &event.task_id)
        .replace("{{tool_name}}", &event.tool_name)
        .replace("{{status}}", status)
        .replace("{{detail}}", &detail)
}

fn runtime_status(kind: &ToolRuntimeEventKind) -> &'static str {
    match kind {
        ToolRuntimeEventKind::ProcessStarted { .. } => "started",
        ToolRuntimeEventKind::OutputChunk { .. } => "output",
        ToolRuntimeEventKind::ProcessCompleted { .. } => "completed",
        ToolRuntimeEventKind::ProcessFailed { .. } => "failed",
        ToolRuntimeEventKind::ProcessKilled { .. } => "killed",
    }
}

fn log_suppression(event: &ToolRuntimeEvent, reason: BackgroundWakeSuppression) {
    tracing::debug!(
        session_id = %event.session_id,
        task_id = %event.task_id,
        ?reason,
        "background completion did not trigger an automatic turn"
    );
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use y_core::runtime::{ToolRuntimeEvent, ToolRuntimeEventKind};
    use y_core::types::SessionId;

    use crate::EventSink;

    use super::{
        render_completion_prompt, BackgroundWakeConfig, BackgroundWakeContext,
        BackgroundWakePolicy, BackgroundWakeService, BackgroundWakeSuppression,
    };

    fn completed(session_id: &str, task_id: &str) -> ToolRuntimeEvent {
        ToolRuntimeEvent {
            session_id: SessionId(session_id.to_string()),
            task_id: task_id.to_string(),
            tool_name: "ShellExec".to_string(),
            backend: None,
            occurred_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            kind: ToolRuntimeEventKind::ProcessCompleted {
                exit_code: 0,
                duration_ms: 1_000,
            },
        }
    }

    fn enabled_config() -> BackgroundWakeConfig {
        BackgroundWakeConfig {
            enabled: true,
            max_wakes_per_hour: 2,
            cooldown_secs: 300,
            allow_during_orchestration: false,
        }
    }

    #[test]
    fn disabled_policy_never_reserves() {
        let policy = BackgroundWakePolicy::new(BackgroundWakeConfig::default());
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();

        let result = policy.try_reserve(
            &completed("session-a", "task-1"),
            BackgroundWakeContext::idle(),
            now,
        );

        assert_eq!(result.unwrap_err(), BackgroundWakeSuppression::Disabled);
    }

    #[test]
    fn busy_session_and_active_orchestration_are_suppressed() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let event = completed("session-a", "task-1");
        let policy = BackgroundWakePolicy::new(enabled_config());

        assert_eq!(
            policy
                .try_reserve(
                    &event,
                    BackgroundWakeContext {
                        turn_active: true,
                        orchestration_active: false,
                    },
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::SessionBusy
        );
        assert_eq!(
            policy
                .try_reserve(
                    &event,
                    BackgroundWakeContext {
                        turn_active: false,
                        orchestration_active: true,
                    },
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::OrchestrationActive
        );
    }

    #[test]
    fn consumed_and_user_killed_tasks_are_suppressed() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let policy = BackgroundWakePolicy::new(enabled_config());
        policy.mark_consumed(&SessionId("session-a".into()), "task-consumed");
        policy.mark_killed(&SessionId("session-a".into()), "task-killed");

        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-consumed"),
                    BackgroundWakeContext::idle(),
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::AlreadyConsumed
        );
        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-killed"),
                    BackgroundWakeContext::idle(),
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::UserKilled
        );
    }

    #[test]
    fn reservation_is_atomic_and_release_does_not_spend_budget() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let event = completed("session-a", "task-1");
        let policy = BackgroundWakePolicy::new(enabled_config());
        let reservation = policy
            .try_reserve(&event, BackgroundWakeContext::idle(), now)
            .unwrap();

        assert_eq!(
            policy
                .try_reserve(&event, BackgroundWakeContext::idle(), now)
                .unwrap_err(),
            BackgroundWakeSuppression::AlreadyReservedOrDelivered
        );

        policy.release(&reservation);
        let retry = policy
            .try_reserve(&event, BackgroundWakeContext::idle(), now)
            .unwrap();
        policy.commit(&retry, now);

        assert_eq!(
            policy
                .try_reserve(&event, BackgroundWakeContext::idle(), now)
                .unwrap_err(),
            BackgroundWakeSuppression::AlreadyReservedOrDelivered
        );
    }

    #[test]
    fn successful_wakes_enforce_cooldown_and_hourly_budget_per_session() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let policy = BackgroundWakePolicy::new(enabled_config());

        let first = policy
            .try_reserve(
                &completed("session-a", "task-1"),
                BackgroundWakeContext::idle(),
                now,
            )
            .unwrap();
        policy.commit(&first, now);

        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-2"),
                    BackgroundWakeContext::idle(),
                    now + Duration::seconds(299),
                )
                .unwrap_err(),
            BackgroundWakeSuppression::Cooldown
        );

        let second = policy
            .try_reserve(
                &completed("session-a", "task-2"),
                BackgroundWakeContext::idle(),
                now + Duration::seconds(300),
            )
            .unwrap();
        policy.commit(&second, now + Duration::seconds(300));

        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-3"),
                    BackgroundWakeContext::idle(),
                    now + Duration::seconds(600),
                )
                .unwrap_err(),
            BackgroundWakeSuppression::HourlyBudgetExhausted
        );

        assert!(policy
            .try_reserve(
                &completed("session-b", "task-4"),
                BackgroundWakeContext::idle(),
                now + Duration::seconds(600),
            )
            .is_ok());
    }

    #[test]
    fn orchestration_can_be_explicitly_allowed() {
        let mut config = enabled_config();
        config.allow_during_orchestration = true;
        let policy = BackgroundWakePolicy::new(config);
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();

        assert!(policy
            .try_reserve(
                &completed("session-a", "task-1"),
                BackgroundWakeContext {
                    turn_active: false,
                    orchestration_active: true,
                },
                now,
            )
            .is_ok());
    }

    #[test]
    fn in_flight_result_observation_blocks_only_the_racing_completion() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let session_id = SessionId("session-a".into());
        let policy = BackgroundWakePolicy::new(enabled_config());

        policy.begin_observation(&session_id, "task-running");
        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-running"),
                    BackgroundWakeContext::idle(),
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::ResultObservationInProgress
        );
        policy.finish_observation(&session_id, "task-running", false);
        assert!(policy
            .try_reserve(
                &completed("session-a", "task-running"),
                BackgroundWakeContext::idle(),
                now,
            )
            .is_ok());

        policy.begin_observation(&session_id, "task-complete");
        policy.finish_observation(&session_id, "task-complete", true);
        assert_eq!(
            policy
                .try_reserve(
                    &completed("session-a", "task-complete"),
                    BackgroundWakeContext::idle(),
                    now,
                )
                .unwrap_err(),
            BackgroundWakeSuppression::AlreadyConsumed
        );
    }

    #[test]
    fn completion_prompt_requires_polling_the_exact_task() {
        let prompt = render_completion_prompt(&completed("session-a", "task-42"));

        assert!(prompt.contains("task-42"));
        assert!(prompt.contains("completed"));
        assert!(prompt.contains("ShellExec"));
        assert!(prompt.contains("poll"));
    }

    #[tokio::test]
    async fn service_event_sink_broadcasts_standard_chat_payloads() {
        let service = BackgroundWakeService::new(enabled_config());
        let mut receiver = service.subscribe();
        let sink = service.event_sink();

        sink.emit_started("wake-run", "session-a", Some(7));

        let event = receiver.recv().await.unwrap();
        assert_eq!(event.event_name(), "chat:started");
        assert_eq!(event.event_id(), Some(7));
        assert_eq!(event.session_id(), "session-a");
        assert_eq!(
            event.payload(),
            serde_json::json!({
                "run_id": "wake-run",
                "session_id": "session-a",
                "kind": "background_auto_wake",
            })
        );
    }
}
