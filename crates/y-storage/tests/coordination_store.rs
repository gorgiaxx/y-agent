#![cfg(feature = "instance_coordination")]

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use y_storage::{
    create_pool, migration, RuntimeInstanceRegistration, SqliteCoordinationStore, StorageConfig,
};

async fn shared_stores() -> (
    tempfile::TempDir,
    sqlx::SqlitePool,
    SqliteCoordinationStore,
    SqliteCoordinationStore,
) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = StorageConfig {
        db_path: directory
            .path()
            .join("coordination.db")
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
    let first = SqliteCoordinationStore::new(first_pool.clone());
    let second = SqliteCoordinationStore::new(second_pool);
    (directory, first_pool, first, second)
}

async fn register(store: &SqliteCoordinationStore, instance_id: &str) {
    store
        .register_instance(&RuntimeInstanceRegistration {
            instance_id: instance_id.to_string(),
            process_id: std::process::id(),
            runtime_kind: "test".to_string(),
            metadata: json!({"test": true}),
        })
        .await
        .expect("register instance");
}

async fn prepare_and_initialize(config: StorageConfig, barrier: Arc<tokio::sync::Barrier>) {
    barrier.wait().await;
    migration::prepare_database(&config)
        .await
        .expect("prepare database");
    let pool = create_pool(&config).await.expect("startup pool");
    migration::run_embedded_migrations(&pool)
        .await
        .expect("initialize schema");
    pool.close().await;
}

async fn prepare_after_barrier(
    config: StorageConfig,
    barrier: Arc<tokio::sync::Barrier>,
) -> Result<(), y_storage::StorageError> {
    barrier.wait().await;
    migration::prepare_database(&config).await
}

#[tokio::test]
async fn concurrent_pools_allow_only_one_live_lease_owner() {
    let (_directory, _pool, first, second) = shared_stores().await;
    register(&first, "instance-a").await;
    register(&second, "instance-b").await;

    let (first_result, second_result) = tokio::join!(
        first.try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-a",
            Duration::from_secs(30),
        ),
        second.try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-b",
            Duration::from_secs(30),
        ),
    );

    let leases = [
        first_result.expect("first acquire"),
        second_result.expect("second acquire"),
    ];
    assert_eq!(
        leases.iter().filter(|lease| lease.is_some()).count(),
        1,
        "exactly one process may own a live singleton lease"
    );
}

#[tokio::test]
async fn takeover_fences_the_previous_owner() {
    let (_directory, pool, first, second) = shared_stores().await;
    register(&first, "instance-a").await;
    register(&second, "instance-b").await;

    let original = first
        .try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-a",
            Duration::from_secs(30),
        )
        .await
        .expect("initial acquire")
        .expect("initial owner");
    let renewed = first
        .renew_lease(&original, Duration::from_secs(30))
        .await
        .expect("renew")
        .expect("lease remains owned");
    assert_eq!(renewed.fencing_token, original.fencing_token);

    sqlx::query(
        "UPDATE runtime_leases SET expires_at = '2000-01-01T00:00:00.000Z' \
         WHERE resource_kind = 'singleton' AND resource_id = 'scheduler'",
    )
    .execute(&pool)
    .await
    .expect("expire lease");

    let replacement = second
        .try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-b",
            Duration::from_secs(30),
        )
        .await
        .expect("takeover")
        .expect("replacement owner");
    assert!(replacement.fencing_token > original.fencing_token);
    assert!(
        first
            .renew_lease(&original, Duration::from_secs(30))
            .await
            .expect("stale renew result")
            .is_none(),
        "the old fencing token must not renew a replacement lease"
    );
    assert!(
        !first
            .release_lease(&original)
            .await
            .expect("stale release result"),
        "the old fencing token must not release a replacement lease"
    );
    assert!(
        second
            .release_lease(&replacement)
            .await
            .expect("owner release"),
        "the current owner should release its lease"
    );
}

#[tokio::test]
async fn subsecond_lease_ttl_is_not_rounded_to_one_second() {
    let (_directory, pool, first, _second) = shared_stores().await;
    register(&first, "instance-a").await;

    first
        .try_acquire_lease(
            "singleton",
            "scheduler",
            "instance-a",
            Duration::from_millis(250),
        )
        .await
        .expect("acquire")
        .expect("lease owner");

    let remaining_ms: f64 = sqlx::query_scalar(
        "SELECT (julianday(expires_at) - julianday('now')) * 86400000.0 \
         FROM runtime_leases \
         WHERE resource_kind = 'singleton' AND resource_id = 'scheduler'",
    )
    .fetch_one(&pool)
    .await
    .expect("lease duration");
    assert!(remaining_ms > 0.0, "lease should still be live");
    assert!(
        remaining_ms < 750.0,
        "250ms lease was unexpectedly rounded to {remaining_ms}ms"
    );
}

#[tokio::test]
async fn stale_instance_pruning_preserves_a_live_lease_owner() {
    let (_directory, pool, first, second) = shared_stores().await;
    register(&first, "live-owner").await;
    register(&second, "stale-observer").await;
    register(&second, "expired-owner").await;
    first
        .try_acquire_lease(
            "singleton",
            "scheduler",
            "live-owner",
            Duration::from_secs(30),
        )
        .await
        .expect("acquire")
        .expect("lease owner");
    second
        .try_acquire_lease(
            "singleton",
            "maintenance",
            "expired-owner",
            Duration::from_secs(30),
        )
        .await
        .expect("acquire expiring lease")
        .expect("expiring lease owner");
    sqlx::raw_sql(
        "UPDATE runtime_instances SET heartbeat_at = '2000-01-01T00:00:00.000Z' \
         WHERE instance_id IN ('live-owner', 'stale-observer', 'expired-owner'); \
         UPDATE runtime_leases SET expires_at = '2000-01-01T00:00:00.000Z' \
         WHERE resource_kind = 'singleton' AND resource_id = 'maintenance'",
    )
    .execute(&pool)
    .await
    .expect("age registrations");

    let removed = first
        .prune_stale_instances(Duration::from_secs(60))
        .await
        .expect("prune stale registrations");

    assert_eq!(removed, 2);
    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT instance_id FROM runtime_instances ORDER BY instance_id")
            .fetch_all(&pool)
            .await
            .expect("remaining registrations");
    assert_eq!(remaining, vec!["live-owner"]);
}

#[tokio::test]
async fn version_three_database_upgrades_without_losing_state() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let db_path = directory.path().join("v3.db");
    let config = StorageConfig {
        db_path: db_path.to_string_lossy().into_owned(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5_000,
        transcript_dir: directory.path().join("transcripts"),
    };
    let pool = create_pool(&config).await.expect("pool");
    migration::run_embedded_migrations(&pool)
        .await
        .expect("fresh schema");
    sqlx::query(
        r"INSERT INTO session_metadata
             (id, root_id, path, session_type, transcript_path)
           VALUES ('preserved', 'preserved', '/preserved', 'main', '/tmp/preserved.jsonl')",
    )
    .execute(&pool)
    .await
    .expect("fixture session");
    sqlx::query("DROP TABLE runtime_leases")
        .execute(&pool)
        .await
        .expect("drop leases");
    sqlx::query("DROP TABLE runtime_instances")
        .execute(&pool)
        .await
        .expect("drop instances");
    sqlx::query("PRAGMA user_version = 3")
        .execute(&pool)
        .await
        .expect("mark v3");
    pool.close().await;

    migration::prepare_database(&config)
        .await
        .expect("prepare v3");
    let upgraded = create_pool(&config).await.expect("upgraded pool");
    migration::run_embedded_migrations(&upgraded)
        .await
        .expect("initialize upgraded schema");

    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&upgraded)
        .await
        .expect("schema version");
    let preserved: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_metadata WHERE id = 'preserved'")
            .fetch_one(&upgraded)
            .await
            .expect("preserved session");
    assert_eq!(version, 4);
    assert_eq!(preserved, 1);
}

#[tokio::test]
async fn concurrent_version_three_preparation_is_serialized() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let db_path = directory.path().join("concurrent-v3.db");
    let config = StorageConfig {
        db_path: db_path.to_string_lossy().into_owned(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5_000,
        transcript_dir: directory.path().join("transcripts"),
    };
    let pool = create_pool(&config).await.expect("pool");
    migration::run_embedded_migrations(&pool)
        .await
        .expect("fresh schema");
    sqlx::query("DROP TABLE runtime_leases")
        .execute(&pool)
        .await
        .expect("drop leases");
    sqlx::query("DROP TABLE runtime_instances")
        .execute(&pool)
        .await
        .expect("drop instances");
    sqlx::query("PRAGMA user_version = 3")
        .execute(&pool)
        .await
        .expect("mark v3");
    pool.close().await;

    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let preparations = tokio::join!(
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), Arc::clone(&barrier)),
        prepare_after_barrier(config.clone(), barrier),
    );

    for preparation in [
        preparations.0,
        preparations.1,
        preparations.2,
        preparations.3,
        preparations.4,
        preparations.5,
        preparations.6,
        preparations.7,
    ] {
        preparation.expect("concurrent preparation");
    }

    let upgraded = create_pool(&config).await.expect("upgraded pool");
    migration::run_embedded_migrations(&upgraded)
        .await
        .expect("initialize upgraded schema");
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&upgraded)
        .await
        .expect("schema version");
    assert_eq!(version, 4);
}

#[tokio::test]
async fn concurrent_first_startup_initializes_one_consistent_schema() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = StorageConfig {
        db_path: directory
            .path()
            .join("concurrent-new.db")
            .to_string_lossy()
            .into_owned(),
        pool_size: 1,
        wal_enabled: true,
        busy_timeout_ms: 5_000,
        transcript_dir: directory.path().join("transcripts"),
    };
    let barrier = Arc::new(tokio::sync::Barrier::new(4));

    tokio::join!(
        prepare_and_initialize(config.clone(), Arc::clone(&barrier)),
        prepare_and_initialize(config.clone(), Arc::clone(&barrier)),
        prepare_and_initialize(config.clone(), Arc::clone(&barrier)),
        prepare_and_initialize(config.clone(), barrier),
    );

    let pool = create_pool(&config).await.expect("verification pool");
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .expect("schema version");
    let coordination_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name IN ('runtime_instances', 'runtime_leases')",
    )
    .fetch_one(&pool)
    .await
    .expect("coordination tables");
    assert_eq!(version, 4);
    assert_eq!(coordination_tables, 2);
}
