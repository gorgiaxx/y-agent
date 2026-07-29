//! Service-owned runtime instance identity and singleton lease supervision.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use y_storage::{RuntimeInstanceRegistration, RuntimeLease, SqliteCoordinationStore, StorageError};

const SINGLETON_RESOURCE_KIND: &str = "singleton";

/// Timing policy for process coordination leases.
#[derive(Debug, Clone, Copy)]
pub struct CoordinationPolicy {
    /// Duration for which an unrenewed lease remains owned.
    pub lease_ttl: Duration,
    /// Interval between lease renewals and acquisition attempts.
    pub renewal_interval: Duration,
}

impl Default for CoordinationPolicy {
    fn default() -> Self {
        Self {
            lease_ttl: Duration::from_secs(15),
            renewal_interval: Duration::from_secs(5),
        }
    }
}

impl CoordinationPolicy {
    fn validate(self) -> Result<Self, InstanceCoordinationError> {
        if self.renewal_interval.is_zero() {
            return Err(InstanceCoordinationError::InvalidPolicy(
                "renewal interval must be greater than zero".to_string(),
            ));
        }
        if self.lease_ttl <= self.renewal_interval {
            return Err(InstanceCoordinationError::InvalidPolicy(
                "lease ttl must be greater than the renewal interval".to_string(),
            ));
        }
        Ok(self)
    }
}

/// Errors produced while registering instances or managing leases.
#[derive(Debug, thiserror::Error)]
pub enum InstanceCoordinationError {
    /// `SQLite` coordination operation failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Lease timing policy cannot provide safe renewal behavior.
    #[error("invalid coordination policy: {0}")]
    InvalidPolicy(String),
    /// The singleton supervisor stopped before it could reconcile ownership.
    #[error("singleton lease supervisor is stopped")]
    SupervisorStopped,
}

/// Lifecycle boundary for a service that must have one process owner.
#[async_trait]
pub trait LeaseManagedService: Send + Sync + 'static {
    /// Start the process-local service after its lease is acquired.
    async fn start(&self);
    /// Stop the process-local service before its lease is released.
    async fn stop(&self);
}

/// Registered process identity and lease policy for one service container.
#[derive(Debug, Clone)]
pub struct InstanceCoordinator {
    store: SqliteCoordinationStore,
    instance_id: Arc<str>,
    policy: CoordinationPolicy,
}

impl InstanceCoordinator {
    /// Register a process with a generated instance identifier and default policy.
    pub async fn register(
        store: SqliteCoordinationStore,
        runtime_kind: impl Into<String>,
    ) -> Result<Self, InstanceCoordinationError> {
        let instance_id = uuid::Uuid::new_v4().to_string();
        Self::register_with(
            store,
            RuntimeInstanceRegistration {
                instance_id,
                process_id: std::process::id(),
                runtime_kind: runtime_kind.into(),
                metadata: serde_json::json!({}),
            },
            CoordinationPolicy::default(),
        )
        .await
    }

    /// Register an explicit process identity and timing policy.
    ///
    /// This constructor is intended for deterministic hosts and tests. Ordinary
    /// application startup should use [`Self::register`].
    pub async fn register_with(
        store: SqliteCoordinationStore,
        registration: RuntimeInstanceRegistration,
        policy: CoordinationPolicy,
    ) -> Result<Self, InstanceCoordinationError> {
        let policy = policy.validate()?;
        store.register_instance(&registration).await?;
        info!(
            instance_id = %registration.instance_id,
            runtime_kind = %registration.runtime_kind,
            process_id = registration.process_id,
            "runtime instance registered"
        );
        Ok(Self {
            store,
            instance_id: Arc::from(registration.instance_id),
            policy,
        })
    }

    /// Return the stable identifier for this process lifetime.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Start supervising one singleton service role.
    pub async fn supervise_singleton(
        &self,
        resource_id: impl Into<String>,
        service: Arc<dyn LeaseManagedService>,
    ) -> Result<SingletonLeaseHandle, InstanceCoordinationError> {
        let resource_id = resource_id.into();
        let lease = self
            .store
            .try_acquire_lease(
                SINGLETON_RESOURCE_KIND,
                &resource_id,
                &self.instance_id,
                self.policy.lease_ttl,
            )
            .await?;
        if lease.is_some() {
            service.start().await;
            info!(
                instance_id = %self.instance_id,
                resource_id,
                "singleton lease acquired"
            );
        } else {
            info!(
                instance_id = %self.instance_id,
                resource_id,
                "singleton lease owned by another runtime instance"
            );
        }

        let shutdown = CancellationToken::new();
        let (reconcile_tx, reconcile_rx) = mpsc::channel(1);
        let task = tokio::spawn(supervise_loop(
            self.clone(),
            resource_id,
            service,
            lease,
            shutdown.clone(),
            reconcile_rx,
        ));
        Ok(SingletonLeaseHandle {
            shutdown,
            task: Some(task),
            reconcile_tx,
        })
    }
}

/// Handle for graceful shutdown of a supervised singleton role.
pub struct SingletonLeaseHandle {
    shutdown: CancellationToken,
    task: Option<JoinHandle<()>>,
    reconcile_tx: mpsc::Sender<oneshot::Sender<()>>,
}

impl SingletonLeaseHandle {
    /// Request an immediate renewal or acquisition attempt and wait for it.
    pub async fn reconcile_now(&self) -> Result<(), InstanceCoordinationError> {
        let (completed_tx, completed_rx) = oneshot::channel();
        self.reconcile_tx
            .send(completed_tx)
            .await
            .map_err(|_| InstanceCoordinationError::SupervisorStopped)?;
        completed_rx
            .await
            .map_err(|_| InstanceCoordinationError::SupervisorStopped)
    }

    /// Stop the local service, release its lease, and join the supervisor task.
    pub async fn shutdown(mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for SingletonLeaseHandle {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn supervise_loop(
    coordinator: InstanceCoordinator,
    resource_id: String,
    service: Arc<dyn LeaseManagedService>,
    mut lease: Option<RuntimeLease>,
    shutdown: CancellationToken,
    mut reconcile_rx: mpsc::Receiver<oneshot::Sender<()>>,
) {
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + coordinator.policy.renewal_interval,
        coordinator.policy.renewal_interval,
    );

    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = interval.tick() => {
                lease = reconcile_ownership(&coordinator, &resource_id, &service, lease).await;
            }
            request = reconcile_rx.recv() => {
                let Some(completed) = request else {
                    break;
                };
                lease = reconcile_ownership(&coordinator, &resource_id, &service, lease).await;
                let _ = completed.send(());
            }
        }
    }

    if let Some(owned) = lease {
        service.stop().await;
        match coordinator.store.release_lease(&owned).await {
            Ok(true) => info!(
                instance_id = %coordinator.instance_id,
                resource_id,
                fencing_token = owned.fencing_token,
                "singleton lease released"
            ),
            Ok(false) => warn!(
                instance_id = %coordinator.instance_id,
                resource_id,
                fencing_token = owned.fencing_token,
                "singleton lease was no longer owned during shutdown"
            ),
            Err(storage_error) => error!(
                instance_id = %coordinator.instance_id,
                resource_id,
                fencing_token = owned.fencing_token,
                error = %storage_error,
                "failed to release singleton lease"
            ),
        }
    }
}

async fn reconcile_ownership(
    coordinator: &InstanceCoordinator,
    resource_id: &str,
    service: &Arc<dyn LeaseManagedService>,
    lease: Option<RuntimeLease>,
) -> Option<RuntimeLease> {
    if let Err(storage_error) = coordinator
        .store
        .heartbeat_instance(&coordinator.instance_id)
        .await
    {
        warn!(
            instance_id = %coordinator.instance_id,
            error = %storage_error,
            "runtime instance heartbeat failed"
        );
    }

    supervise_tick(coordinator, resource_id, service, lease).await
}

async fn supervise_tick(
    coordinator: &InstanceCoordinator,
    resource_id: &str,
    service: &Arc<dyn LeaseManagedService>,
    lease: Option<RuntimeLease>,
) -> Option<RuntimeLease> {
    if let Some(owned) = lease {
        return match coordinator
            .store
            .renew_lease(&owned, coordinator.policy.lease_ttl)
            .await
        {
            Ok(Some(renewed)) => Some(renewed),
            Ok(None) => {
                warn!(
                    instance_id = %coordinator.instance_id,
                    resource_id,
                    fencing_token = owned.fencing_token,
                    "singleton lease lost; stopping local service"
                );
                service.stop().await;
                None
            }
            Err(storage_error) => {
                error!(
                    instance_id = %coordinator.instance_id,
                    resource_id,
                    fencing_token = owned.fencing_token,
                    error = %storage_error,
                    "singleton lease renewal failed; stopping local service"
                );
                service.stop().await;
                None
            }
        };
    }

    match coordinator
        .store
        .try_acquire_lease(
            SINGLETON_RESOURCE_KIND,
            resource_id,
            &coordinator.instance_id,
            coordinator.policy.lease_ttl,
        )
        .await
    {
        Ok(Some(acquired)) => {
            service.start().await;
            info!(
                instance_id = %coordinator.instance_id,
                resource_id,
                fencing_token = acquired.fencing_token,
                "singleton lease acquired"
            );
            Some(acquired)
        }
        Ok(None) => None,
        Err(storage_error) => {
            error!(
                instance_id = %coordinator.instance_id,
                resource_id,
                error = %storage_error,
                "singleton lease acquisition failed"
            );
            None
        }
    }
}
