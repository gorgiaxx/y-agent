//! `SchedulerManager`: top-level entry point that owns the async trigger loop.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::config::SchedulerConfig;
use crate::executor::ScheduleExecutor;
use crate::queue::{trigger_queue, TriggerReceiver, TriggerSender};
use crate::recovery;
use crate::store::{Schedule, ScheduleStore};
use crate::trigger::{evaluate_all, FiredTrigger};

/// The top-level scheduler service.
///
/// Owns the `ScheduleStore`, `ScheduleExecutor`, and runs an async trigger loop
/// that evaluates all active schedules on each tick.
pub struct SchedulerManager {
    store: Arc<Mutex<ScheduleStore>>,
    executor: Arc<Mutex<ScheduleExecutor>>,
    config: SchedulerConfig,
    /// Handle to the trigger evaluation loop task.
    eval_handle: Option<JoinHandle<()>>,
    /// Handle to the executor consumer loop task.
    exec_handle: Option<JoinHandle<()>>,
    /// Notification for shutdown.
    shutdown: Arc<Notify>,
    /// Sender for the trigger queue (kept to clone for external event injection).
    trigger_tx: Option<TriggerSender>,
}

impl SchedulerManager {
    /// Create a new scheduler manager with the given configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            store: Arc::new(Mutex::new(ScheduleStore::new())),
            executor: Arc::new(Mutex::new(ScheduleExecutor::new())),
            config,
            eval_handle: None,
            exec_handle: None,
            shutdown: Arc::new(Notify::new()),
            trigger_tx: None,
        }
    }

    /// Create a scheduler manager with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(SchedulerConfig::default())
    }

    /// Register a schedule.
    pub async fn register(&self, schedule: Schedule) {
        let mut store = self.store.lock().await;
        info!(schedule_id = %schedule.id, "Registering schedule");
        store.register(schedule);
    }

    /// Remove a schedule by ID. Returns `true` if found and removed.
    pub async fn remove(&self, id: &str) -> bool {
        let mut store = self.store.lock().await;
        info!(schedule_id = %id, "Removing schedule");
        store.remove(id)
    }

    /// Pause a schedule (set enabled = false).
    pub async fn pause(&self, id: &str) -> bool {
        let mut store = self.store.lock().await;
        store.set_enabled(id, false)
    }

    /// Resume a schedule (set enabled = true).
    pub async fn resume(&self, id: &str) -> bool {
        let mut store = self.store.lock().await;
        store.set_enabled(id, true)
    }

    /// Get a clone of a schedule by ID.
    pub async fn get_schedule(&self, id: &str) -> Option<Schedule> {
        let store = self.store.lock().await;
        store.get(id).cloned()
    }

    /// List all schedules.
    pub async fn list_schedules(&self) -> Vec<Schedule> {
        let store = self.store.lock().await;
        store.list().to_vec()
    }

    /// Get total execution count.
    pub async fn execution_count(&self) -> usize {
        let executor = self.executor.lock().await;
        executor.execution_count()
    }

    /// Get a reference to the trigger sender for external event injection.
    pub fn trigger_sender(&self) -> Option<&TriggerSender> {
        self.trigger_tx.as_ref()
    }

    /// Start the scheduler — spawns the trigger evaluation loop and executor loop.
    pub async fn start(&mut self, tick_interval: Duration) {
        if self.eval_handle.is_some() {
            warn!("Scheduler already running");
            return;
        }

        info!(
            "Starting scheduler with tick interval {:?}, max_concurrent={}",
            tick_interval, self.config.max_concurrent_executions
        );

        let (tx, rx) = trigger_queue();
        self.trigger_tx = Some(tx.clone());

        // Run missed-schedule recovery before starting the loop.
        {
            let store_guard = self.store.lock().await;
            let (recovery_triggers, result) = recovery::recover_missed(&store_guard, Utc::now());
            drop(store_guard);

            if !recovery_triggers.is_empty() {
                info!(
                    "Recovery: {} caught up, {} skipped, {} backfilled ({} total triggers)",
                    result.caught_up.len(),
                    result.skipped.len(),
                    result.backfilled.len(),
                    recovery_triggers.len(),
                );
                for trigger in recovery_triggers {
                    let _ = tx.send(trigger).await;
                }
            }
        }

        let shutdown = self.shutdown.clone();
        let store = self.store.clone();

        // Trigger evaluation loop.
        let eval_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_interval);
            loop {
                tokio::select! {
                    () = shutdown.notified() => {
                        info!("Trigger evaluation loop shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        let store_guard = store.lock().await;
                        let schedules: Vec<Schedule> = store_guard.list_enabled()
                            .into_iter()
                            .cloned()
                            .collect();
                        drop(store_guard);

                        let now = Utc::now();
                        let fired = evaluate_all(&schedules, now);

                        for trigger in fired {
                            debug!(schedule_id = %trigger.schedule_id, "Trigger fired");
                            if tx.send(trigger).await.is_err() {
                                warn!("Trigger queue closed, stopping evaluation");
                                return;
                            }
                        }
                    }
                }
            }
        });

        // Executor consumer loop.
        let exec_shutdown = self.shutdown.clone();
        let exec_store = self.store.clone();
        let exec_executor = self.executor.clone();
        let exec_handle = tokio::spawn(async move {
            Self::executor_loop(rx, exec_store, exec_executor, exec_shutdown).await;
        });

        self.eval_handle = Some(eval_handle);
        self.exec_handle = Some(exec_handle);
    }

    /// Internal executor consumer loop.
    async fn executor_loop(
        mut rx: TriggerReceiver,
        store: Arc<Mutex<ScheduleStore>>,
        executor: Arc<Mutex<ScheduleExecutor>>,
        shutdown: Arc<Notify>,
    ) {
        loop {
            tokio::select! {
                () = shutdown.notified() => {
                    info!("Executor loop shutting down");
                    break;
                }
                trigger = rx.recv() => {
                    if let Some(fired) = trigger {
                        Self::handle_fired_trigger(fired, &store, &executor).await;
                    } else {
                        info!("Trigger queue closed, executor stopping");
                        break;
                    }
                }
            }
        }
    }

    /// Handle a single fired trigger.
    async fn handle_fired_trigger(
        fired: FiredTrigger,
        store: &Arc<Mutex<ScheduleStore>>,
        executor: &Arc<Mutex<ScheduleExecutor>>,
    ) {
        let mut store_guard = store.lock().await;
        let schedule = if let Some(s) = store_guard.get(&fired.schedule_id) { s.clone() } else {
            warn!(schedule_id = %fired.schedule_id, "Schedule not found, skipping");
            return;
        };

        let mut exec_guard = executor.lock().await;
        let execution_id = exec_guard.trigger_execution(&schedule, &mut store_guard);
        debug!(execution_id = %execution_id, "Execution triggered");
    }

    /// Stop the scheduler gracefully.
    pub async fn stop(&mut self) {
        if self.eval_handle.is_none() {
            return;
        }

        info!("Stopping scheduler");

        // Notify both loops twice (once for each notified() call).
        self.shutdown.notify_waiters();

        if let Some(handle) = self.eval_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        if let Some(handle) = self.exec_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        self.trigger_tx = None;
        // Reset shutdown for potential restart.
        self.shutdown = Arc::new(Notify::new());

        info!("Scheduler stopped");
    }

    /// Whether the scheduler is currently running.
    pub fn is_running(&self) -> bool {
        self.eval_handle.is_some()
    }
}

impl Default for SchedulerManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TriggerConfig;

    #[tokio::test]
    async fn test_manager_register_and_get() {
        let mgr = SchedulerManager::with_defaults();

        let schedule = Schedule::new(
            "test-schedule",
            "Test Schedule",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf-1",
        );
        mgr.register(schedule).await;

        let retrieved = mgr.get_schedule("test-schedule").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Schedule");
    }

    #[tokio::test]
    async fn test_manager_remove() {
        let mgr = SchedulerManager::with_defaults();
        mgr.register(Schedule::new("s1", "S1", TriggerConfig::Interval { interval_secs: 60 }, "wf"))
            .await;
        assert!(mgr.remove("s1").await);
        assert!(mgr.get_schedule("s1").await.is_none());
    }

    #[tokio::test]
    async fn test_manager_pause_resume() {
        let mgr = SchedulerManager::with_defaults();
        mgr.register(Schedule::new("s1", "S1", TriggerConfig::Interval { interval_secs: 60 }, "wf"))
            .await;

        mgr.pause("s1").await;
        assert!(!mgr.get_schedule("s1").await.unwrap().enabled);

        mgr.resume("s1").await;
        assert!(mgr.get_schedule("s1").await.unwrap().enabled);
    }

    #[tokio::test]
    async fn test_manager_start_stop() {
        let mut mgr = SchedulerManager::with_defaults();
        assert!(!mgr.is_running());

        mgr.start(Duration::from_millis(50)).await;
        assert!(mgr.is_running());

        mgr.stop().await;
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn test_manager_executes_interval_schedule() {
        let mut mgr = SchedulerManager::with_defaults();

        // Register a schedule with a very short interval.
        let schedule = Schedule::new(
            "fast-interval",
            "Fast Interval",
            TriggerConfig::Interval { interval_secs: 0 }, // fires immediately
            "wf",
        );
        mgr.register(schedule).await;

        // Start with a short tick.
        mgr.start(Duration::from_millis(20)).await;

        // Wait enough for at least one tick + execution.
        tokio::time::sleep(Duration::from_millis(100)).await;

        mgr.stop().await;

        // Should have fired at least once.
        let count = mgr.execution_count().await;
        assert!(count >= 1, "Expected at least 1 execution, got {count}");
    }

    #[tokio::test]
    async fn test_manager_list_schedules() {
        let mgr = SchedulerManager::with_defaults();
        mgr.register(Schedule::new("s1", "S1", TriggerConfig::Interval { interval_secs: 60 }, "wf"))
            .await;
        mgr.register(Schedule::new("s2", "S2", TriggerConfig::Interval { interval_secs: 120 }, "wf"))
            .await;

        let list = mgr.list_schedules().await;
        assert_eq!(list.len(), 2);
    }
}
