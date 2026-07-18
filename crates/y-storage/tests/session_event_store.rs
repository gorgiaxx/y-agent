use y_core::session_event::{NewSessionEvent, SessionEventKind, SessionEventRetention};
use y_core::types::SessionId;
use y_storage::{create_pool, migration::run_embedded_migrations, SqliteSessionEventStore};

async fn setup() -> SqliteSessionEventStore {
    let pool = create_pool(&y_storage::StorageConfig::in_memory())
        .await
        .unwrap();
    run_embedded_migrations(&pool).await.unwrap();
    SqliteSessionEventStore::new(pool)
}

fn event(session_id: &str, correlation_id: &str) -> NewSessionEvent {
    NewSessionEvent {
        session_id: SessionId(session_id.to_string()),
        kind: SessionEventKind::ChatProgress,
        payload: serde_json::json!({"value": correlation_id}),
        retention: SessionEventRetention::Durable,
        correlation_id: Some(correlation_id.to_string()),
    }
}

fn short_lived_event(session_id: &str, correlation_id: &str, value: usize) -> NewSessionEvent {
    NewSessionEvent {
        session_id: SessionId(session_id.to_string()),
        kind: SessionEventKind::ToolRuntime,
        payload: serde_json::json!({"value": value}),
        retention: SessionEventRetention::ShortLived,
        correlation_id: Some(correlation_id.to_string()),
    }
}

#[tokio::test]
async fn append_assigns_global_ids_and_per_session_sequences() {
    let store = setup().await;

    let a1 = store.append(&event("session-a", "a1")).await.unwrap();
    let b1 = store.append(&event("session-b", "b1")).await.unwrap();
    let a2 = store.append(&event("session-a", "a2")).await.unwrap();

    assert!(a1.event_id < b1.event_id && b1.event_id < a2.event_id);
    assert_eq!(a1.seq, 1);
    assert_eq!(b1.seq, 1);
    assert_eq!(a2.seq, 2);
}

#[tokio::test]
async fn a_new_store_instance_continues_sequences_from_sqlite() {
    let store = setup().await;
    let first = store.append(&event("session-a", "a1")).await.unwrap();
    let restarted = SqliteSessionEventStore::new(store.pool().clone());

    let second = restarted.append(&event("session-a", "a2")).await.unwrap();

    assert_eq!(second.seq, first.seq + 1);
    assert_eq!(second.event_id, first.event_id + 1);
}

#[tokio::test]
async fn concurrent_appends_keep_session_sequence_unique_and_gap_free() {
    let store = setup().await;
    let mut tasks = Vec::new();
    for index in 0..16 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .append(&event("session-a", &format!("event-{index}")))
                .await
                .unwrap()
        }));
    }

    let mut sequences = Vec::new();
    for task in tasks {
        sequences.push(task.await.unwrap().seq);
    }
    sequences.sort_unstable();

    assert_eq!(sequences, (1..=16).collect::<Vec<_>>());
}

#[tokio::test]
async fn replay_queries_are_strictly_after_cursor_and_deterministic() {
    let store = setup().await;
    let a1 = store.append(&event("session-a", "a1")).await.unwrap();
    let _b1 = store.append(&event("session-b", "b1")).await.unwrap();
    let a2 = store.append(&event("session-a", "a2")).await.unwrap();

    let global = store
        .list_after_event_id(a1.event_id, None, 100)
        .await
        .unwrap();
    assert_eq!(global.len(), 2);
    assert!(global
        .windows(2)
        .all(|pair| pair[0].event_id < pair[1].event_id));

    let session = store
        .list_after_event_id(a1.event_id, Some(&SessionId("session-a".into())), 100)
        .await
        .unwrap();
    assert_eq!(session, vec![a2]);
}

#[tokio::test]
async fn latest_event_id_provides_a_no_history_subscription_floor() {
    let store = setup().await;
    assert_eq!(store.latest_event_id().await.unwrap(), 0);

    store.append(&event("session-a", "a1")).await.unwrap();
    let latest = store.append(&event("session-b", "b1")).await.unwrap();

    assert_eq!(store.latest_event_id().await.unwrap(), latest.event_id);
}

#[tokio::test]
async fn latest_correlation_query_supports_pending_request_replay() {
    let store = setup().await;
    store
        .append(&event("session-a", "permission-1"))
        .await
        .unwrap();
    store.append(&event("session-a", "tool-1")).await.unwrap();

    let rows = store
        .latest_for_correlations(
            &SessionId("session-a".into()),
            &["permission-1".to_string()],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].correlation_id.as_deref(), Some("permission-1"));
}

#[tokio::test]
async fn pruning_short_lived_correlation_keeps_latest_chunks_and_durable_events() {
    let store = setup().await;
    for value in 0..5 {
        store
            .append(&short_lived_event("session-a", "runtime:proc-1", value))
            .await
            .unwrap();
    }
    let durable = store
        .append(&event("session-a", "runtime:proc-1"))
        .await
        .unwrap();

    store
        .prune_short_lived_for_correlation(&SessionId("session-a".into()), "runtime:proc-1", 2)
        .await
        .unwrap();

    let replay = store.list_after_event_id(0, None, 100).await.unwrap();
    assert_eq!(replay.len(), 3);
    assert_eq!(
        replay
            .iter()
            .filter(|event| event.retention == SessionEventRetention::ShortLived)
            .count(),
        2
    );
    assert!(replay
        .iter()
        .any(|event| event.event_id == durable.event_id));
}
