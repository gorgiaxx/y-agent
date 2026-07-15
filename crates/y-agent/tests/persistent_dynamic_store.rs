use std::fs::OpenOptions;
use std::io::Write;

use tempfile::tempdir;
use y_agent::agent::dynamic_agent::{
    make_dynamic_agent, AgentStatus, CreatorPermissionSnapshot, DynamicAgentStoreBackend,
};
use y_agent::agent::persistent_dynamic_store::PersistentDynamicAgentStore;

fn creator_snapshot() -> CreatorPermissionSnapshot {
    CreatorPermissionSnapshot {
        tools_allowed: vec!["FileRead".to_string(), "SearchCode".to_string()],
        max_iterations: 30,
        max_tool_calls: 60,
        max_tokens: 8_192,
        delegation_depth: 3,
    }
}

fn agent(name: &str) -> y_agent::agent::dynamic_agent::DynamicAgentDefinition {
    make_dynamic_agent(
        name,
        "Searches the repository for relevant implementation details",
        "root-agent",
        &["FileRead".to_string(), "SearchCode".to_string()],
        &creator_snapshot(),
    )
}

#[test]
fn reopens_created_dynamic_agents() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agents.jsonl");

    let store = PersistentDynamicAgentStore::open(&path).unwrap();
    let created = store.create(agent("code-scout")).unwrap();
    drop(store);

    let reopened = PersistentDynamicAgentStore::open(&path).unwrap();
    assert_eq!(reopened.count(), 1);
    let replayed = reopened.get(&created.id).unwrap();
    assert_eq!(replayed.id, created.id);
    assert_eq!(replayed.definition.name, created.definition.name);
    assert_eq!(
        replayed.effective_permissions.tools_allowed,
        created.effective_permissions.tools_allowed
    );
}

#[test]
fn replays_latest_update_and_deactivation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agents.jsonl");

    let store = PersistentDynamicAgentStore::open(&path).unwrap();
    let created = store.create(agent("code-scout")).unwrap();
    let mut changed = created.clone();
    changed.definition.description = "Finds architecture and test evidence".to_string();
    let updated = store.update(changed).unwrap();
    store
        .deactivate(&created.id, "replaced by a specialized agent")
        .unwrap();
    drop(store);

    let reopened = PersistentDynamicAgentStore::open(&path).unwrap();
    let replayed = reopened.get(&created.id).unwrap();
    assert_eq!(replayed.version, updated.version);
    assert_eq!(replayed.status, AgentStatus::Deactivated);
    assert_eq!(
        replayed.deactivation_reason.as_deref(),
        Some("replaced by a specialized agent")
    );
    assert!(reopened.list_active().is_empty());
}

#[test]
fn ignores_an_uncommitted_truncated_tail_record() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agents.jsonl");

    let store = PersistentDynamicAgentStore::open(&path).unwrap();
    let created = store.create(agent("code-scout")).unwrap();
    drop(store);

    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(br#"{"schema_version":1,"agent":{"id":"partial""#)
        .unwrap();
    file.sync_all().unwrap();

    let reopened = PersistentDynamicAgentStore::open(&path).unwrap();
    assert_eq!(reopened.count(), 1);
    assert_eq!(reopened.get(&created.id).unwrap().id, created.id);
}

#[test]
fn rollback_restores_a_historical_snapshot_as_a_new_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agents.jsonl");

    let store = PersistentDynamicAgentStore::open(&path).unwrap();
    let created = store.create(agent("code-scout")).unwrap();
    let mut changed = created.clone();
    changed.definition.description = "A regressed description".to_string();
    let updated = store.update(changed).unwrap();
    assert_eq!(updated.version, 2);

    let rolled_back = store.rollback(&created.id, 1).unwrap();
    assert_eq!(rolled_back.version, 3);
    assert_eq!(
        rolled_back.definition.description,
        created.definition.description
    );
    assert_eq!(rolled_back.status, AgentStatus::Active);
    drop(store);

    let reopened = PersistentDynamicAgentStore::open(&path).unwrap();
    let replayed = reopened.get(&created.id).unwrap();
    assert_eq!(replayed.version, 3);
    assert_eq!(
        replayed.definition.description,
        created.definition.description
    );
}
