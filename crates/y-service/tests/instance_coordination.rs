#![cfg(feature = "instance_coordination")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use y_service::{CoordinationPolicy, InstanceCoordinator, LeaseManagedService};
use y_storage::{
    create_pool, migration, RuntimeInstanceRegistration, SqliteCoordinationStore, StorageConfig,
};

#[derive(Default)]
struct TestService {
    running: AtomicBool,
}

#[async_trait]
impl LeaseManagedService for TestService {
    async fn start(&self) {
        self.running.store(true, Ordering::SeqCst);
    }

    async fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

async fn coordinator(store: SqliteCoordinationStore, instance_id: &str) -> InstanceCoordinator {
    InstanceCoordinator::register_with(
        store,
        RuntimeInstanceRegistration {
            instance_id: instance_id.to_string(),
            process_id: std::process::id(),
            runtime_kind: "test".to_string(),
            metadata: json!({"test": true}),
        },
        CoordinationPolicy {
            lease_ttl: Duration::from_secs(30),
            renewal_interval: Duration::from_secs(5),
        },
    )
    .await
    .expect("register coordinator")
}

#[tokio::test]
async fn singleton_supervisor_runs_one_owner_and_transfers_after_release() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = StorageConfig {
        db_path: directory
            .path()
            .join("service-coordination.db")
            .to_string_lossy()
            .into_owned(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5_000,
        transcript_dir: directory.path().join("transcripts"),
    };
    let first_pool = create_pool(&config).await.expect("first pool");
    migration::run_embedded_migrations(&first_pool)
        .await
        .expect("schema");
    let second_pool = create_pool(&config).await.expect("second pool");
    let first = coordinator(SqliteCoordinationStore::new(first_pool), "instance-a").await;
    let second = coordinator(SqliteCoordinationStore::new(second_pool), "instance-b").await;
    let first_service = Arc::new(TestService::default());
    let second_service = Arc::new(TestService::default());

    let first_handle = first
        .supervise_singleton("scheduler", first_service.clone())
        .await
        .expect("first supervisor");
    let second_handle = second
        .supervise_singleton("scheduler", second_service.clone())
        .await
        .expect("second supervisor");

    assert!(first_service.running.load(Ordering::SeqCst));
    assert!(!second_service.running.load(Ordering::SeqCst));

    first_handle.shutdown().await;
    second_handle
        .reconcile_now()
        .await
        .expect("reconcile after release");

    assert!(second_service.running.load(Ordering::SeqCst));
    second_handle.shutdown().await;
}

#[tokio::test]
async fn stale_supervisor_stops_after_its_lease_is_replaced() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = StorageConfig {
        db_path: directory
            .path()
            .join("lease-loss.db")
            .to_string_lossy()
            .into_owned(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5_000,
        transcript_dir: directory.path().join("transcripts"),
    };
    let first_pool = create_pool(&config).await.expect("first pool");
    migration::run_embedded_migrations(&first_pool)
        .await
        .expect("schema");
    let second_pool = create_pool(&config).await.expect("second pool");
    let first_store = SqliteCoordinationStore::new(first_pool);
    let first = coordinator(first_store.clone(), "instance-a").await;
    let second = coordinator(SqliteCoordinationStore::new(second_pool), "instance-b").await;
    let first_service = Arc::new(TestService::default());
    let second_service = Arc::new(TestService::default());
    let first_handle = first
        .supervise_singleton("scheduler", first_service.clone())
        .await
        .expect("first supervisor");

    let first_lease = first_store
        .try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-a",
            Duration::from_secs(30),
        )
        .await
        .expect("read current ownership")
        .expect("first lease");
    assert!(
        first_store
            .release_lease(&first_lease)
            .await
            .expect("release behind supervisor"),
        "test setup should leave the first supervisor with a stale token"
    );
    let second_handle = second
        .supervise_singleton("scheduler", second_service.clone())
        .await
        .expect("second supervisor");

    assert!(second_service.running.load(Ordering::SeqCst));
    first_handle
        .reconcile_now()
        .await
        .expect("stale owner reconciliation");
    assert!(!first_service.running.load(Ordering::SeqCst));

    first_handle.shutdown().await;
    second_handle.shutdown().await;
}
