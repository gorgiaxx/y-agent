//! E2E integration test: Checkpoint save, recovery, and interrupt/resume.

use y_core::checkpoint::{CheckpointStatus, CheckpointStorage};
use y_core::types::{SessionId, WorkflowId};
use y_test_utils::MockCheckpointStorage;

#[tokio::test]
async fn e2e_checkpoint_write_commit_read() {
    let store = MockCheckpointStorage::new();
    let wid = WorkflowId::new();
    let sid = SessionId::new();

    // Step 1: pending write
    let state1 = serde_json::json!({"dag": {"task_a": "completed"}});
    store.write_pending(&wid, &sid, 1, &state1).await.unwrap();

    // Not committed yet — but read_committed returns the entry with null committed_state
    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.committed_state, serde_json::Value::Null);

    // Commit step 1
    store.commit(&wid, 1).await.unwrap();
    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.committed_state, state1);
    assert_eq!(cp.step_number, 1);
}

#[tokio::test]
async fn e2e_checkpoint_multi_step_recovery() {
    let store = MockCheckpointStorage::new();
    let wid = WorkflowId::new();
    let sid = SessionId::new();

    // Simulate a 3-step DAG execution
    for step in 1..=3 {
        let state =
            serde_json::json!({"step": step, "outputs": {"result": format!("step-{step}")}});
        store.write_pending(&wid, &sid, step, &state).await.unwrap();
        store.commit(&wid, step).await.unwrap();
    }

    // Recovery: read committed should show step 3
    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.step_number, 3);
    assert_eq!(cp.committed_state["step"], 3);
}

#[tokio::test]
async fn e2e_checkpoint_interrupt_and_resume() {
    let store = MockCheckpointStorage::new();
    let wid = WorkflowId::new();
    let sid = SessionId::new();

    // Execute steps 1 and 2
    let state1 = serde_json::json!({"step": 1});
    store.write_pending(&wid, &sid, 1, &state1).await.unwrap();
    store.commit(&wid, 1).await.unwrap();

    let state2 = serde_json::json!({"step": 2});
    store.write_pending(&wid, &sid, 2, &state2).await.unwrap();
    store.commit(&wid, 2).await.unwrap();

    // Interrupt at step 2 (e.g., HITL approval needed)
    let interrupt_data = serde_json::json!({
        "reason": "human_approval_required",
        "pending_action": "delete_file",
        "file": "/important/data.txt"
    });
    store
        .set_interrupted(&wid, interrupt_data.clone())
        .await
        .unwrap();

    // Verify interrupted state
    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.status, CheckpointStatus::Interrupted);
    assert_eq!(cp.interrupt_data, Some(interrupt_data));
    assert_eq!(cp.committed_state["step"], 2);
}

#[tokio::test]
async fn e2e_checkpoint_crash_simulation() {
    let store = MockCheckpointStorage::new();
    let wid = WorkflowId::new();
    let sid = SessionId::new();

    // Step 1: committed successfully
    let state1 = serde_json::json!({"step": 1, "data": "safe"});
    store.write_pending(&wid, &sid, 1, &state1).await.unwrap();
    store.commit(&wid, 1).await.unwrap();

    // Step 2: written as pending, but NOT committed (simulating crash)
    let state2 = serde_json::json!({"step": 2, "data": "lost_on_crash"});
    store.write_pending(&wid, &sid, 2, &state2).await.unwrap();
    // -- crash happens here, commit never called --

    // Recovery: committed state should still be step 1
    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.committed_state["step"], 1);
    assert_eq!(cp.committed_state["data"], "safe");
    // pending_state contains step 2 but wasn't committed
    assert!(cp.pending_state.is_some());
}

#[tokio::test]
async fn e2e_checkpoint_completion() {
    let store = MockCheckpointStorage::new();
    let wid = WorkflowId::new();
    let sid = SessionId::new();

    let state = serde_json::json!({"final": true});
    store.write_pending(&wid, &sid, 1, &state).await.unwrap();
    store.commit(&wid, 1).await.unwrap();
    store.set_completed(&wid).await.unwrap();

    let cp = store.read_committed(&wid).await.unwrap().unwrap();
    assert_eq!(cp.status, CheckpointStatus::Completed);
}
