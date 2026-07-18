#![cfg(feature = "capability_packs")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use y_service::capability_pack::{
    CapabilityPackChangeKind, CapabilityPackInstallError, CapabilityPackInstallOptions,
    CapabilityPackInstaller, CapabilityPackProvenance, CapabilityPackSourceKind,
    CapabilityPackTransactionJournal, CapabilityPackTransactionState,
    CapabilityPackTransactionStatus, CapabilityResourceKind, DeclarativeCapabilityBackend,
    DurableCapabilityPackInstaller, ValidatedCapabilityPack, ValidatedCapabilityResource,
};

type ResourceKey = (CapabilityResourceKind, String);

#[derive(Default)]
struct FakeBackend {
    state: Mutex<BTreeMap<ResourceKey, String>>,
    events: Mutex<Vec<String>>,
    fail_apply: Option<String>,
    fail_restore: Option<String>,
    fail_validation: Option<String>,
    sabotage_journal_on_apply: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FakeSnapshot {
    previous: Option<String>,
}

#[async_trait]
impl DeclarativeCapabilityBackend for FakeBackend {
    type Snapshot = FakeSnapshot;

    async fn validate(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        if self.fail_validation.as_deref() == Some(resource.id.as_str()) {
            return Err(format!("injected validation failure for {}", resource.id));
        }
        Ok(())
    }

    async fn current_hash(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Option<String>, String> {
        Ok(self
            .state
            .lock()
            .expect("state")
            .get(&(resource.kind, resource.id.clone()))
            .cloned())
    }

    async fn snapshot(
        &self,
        resource: &ValidatedCapabilityResource,
    ) -> Result<Self::Snapshot, String> {
        self.events
            .lock()
            .expect("events")
            .push(format!("snapshot:{}", resource.id));
        Ok(FakeSnapshot {
            previous: self
                .state
                .lock()
                .expect("state")
                .get(&(resource.kind, resource.id.clone()))
                .cloned(),
        })
    }

    async fn apply(&self, resource: &ValidatedCapabilityResource) -> Result<(), String> {
        self.events
            .lock()
            .expect("events")
            .push(format!("apply:{}", resource.id));
        self.state.lock().expect("state").insert(
            (resource.kind, resource.id.clone()),
            resource.sha256.clone(),
        );
        if let Some(root) = &self.sabotage_journal_on_apply {
            std::fs::remove_dir_all(root).expect("remove journal root");
            std::fs::write(root, b"not a directory").expect("replace journal root with file");
        }
        if self.fail_apply.as_deref() == Some(resource.id.as_str()) {
            return Err(format!("injected apply failure for {}", resource.id));
        }
        Ok(())
    }

    async fn restore(
        &self,
        resource: &ValidatedCapabilityResource,
        snapshot: Self::Snapshot,
    ) -> Result<(), String> {
        self.events
            .lock()
            .expect("events")
            .push(format!("restore:{}", resource.id));
        if self.fail_restore.as_deref() == Some(resource.id.as_str()) {
            return Err(format!("injected restore failure for {}", resource.id));
        }
        let key = (resource.kind, resource.id.clone());
        let mut state = self.state.lock().expect("state");
        if let Some(previous) = snapshot.previous {
            state.insert(key, previous);
        } else {
            state.remove(&key);
        }
        Ok(())
    }
}

fn resource(kind: CapabilityResourceKind, id: &str, sha256: &str) -> ValidatedCapabilityResource {
    ValidatedCapabilityResource {
        kind,
        id: id.to_string(),
        path: PathBuf::from(format!("/pack/{id}")),
        sha256: sha256.to_string(),
    }
}

fn pack(resources: Vec<ValidatedCapabilityResource>) -> ValidatedCapabilityPack {
    ValidatedCapabilityPack {
        schema_version: 1,
        id: "test-pack".into(),
        version: "1.0.0".into(),
        description: None,
        provenance: CapabilityPackProvenance {
            source_kind: CapabilityPackSourceKind::LocalDirectory,
            pack_root: PathBuf::from("/pack"),
            manifest_path: PathBuf::from("/pack/capability-pack.toml"),
            manifest_sha256: "f".repeat(64),
        },
        resources,
    }
}

#[tokio::test]
async fn preview_is_deterministic_and_marks_executable_declarations_inactive() {
    let backend = FakeBackend::default();
    backend.state.lock().expect("state").insert(
        (CapabilityResourceKind::Agent, "existing".into()),
        "1".repeat(64),
    );
    let pack = pack(vec![
        resource(CapabilityResourceKind::Lsp, "rust-lsp", &"4".repeat(64)),
        resource(CapabilityResourceKind::Skill, "new-skill", &"3".repeat(64)),
        resource(CapabilityResourceKind::Agent, "existing", &"2".repeat(64)),
    ]);

    let first = CapabilityPackInstaller::preview(&backend, &pack)
        .await
        .expect("preview");
    let second = CapabilityPackInstaller::preview(&backend, &pack)
        .await
        .expect("preview");

    assert_eq!(first, second);
    assert!(first.can_apply);
    assert_eq!(first.changes[0].resource_id, "existing");
    assert_eq!(first.changes[0].change, CapabilityPackChangeKind::Replace);
    assert_eq!(first.changes[1].resource_id, "rust-lsp");
    assert_eq!(first.changes[1].change, CapabilityPackChangeKind::Add);
    assert!(first.changes[1].requires_activation);
    assert_eq!(first.changes[2].resource_id, "new-skill");
    assert!(!first.changes[2].requires_activation);
}

#[tokio::test]
async fn replacement_requires_explicit_install_approval() {
    let backend = FakeBackend::default();
    backend.state.lock().expect("state").insert(
        (CapabilityResourceKind::Agent, "reviewer".into()),
        "1".repeat(64),
    );
    let pack = pack(vec![resource(
        CapabilityResourceKind::Agent,
        "reviewer",
        &"2".repeat(64),
    )]);

    let error =
        CapabilityPackInstaller::install(&backend, &pack, CapabilityPackInstallOptions::default())
            .await
            .expect_err("replacement approval");

    assert!(matches!(
        error,
        CapabilityPackInstallError::ReplacementApprovalRequired { .. }
    ));
    assert!(backend.events.lock().expect("events").is_empty());
}

#[tokio::test]
async fn executable_resources_install_as_owned_inactive_changes() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal = CapabilityPackTransactionJournal::new(temp.path());
    let backend = FakeBackend::default();
    let pack = pack(vec![resource(
        CapabilityResourceKind::Mcp,
        "pack-mcp",
        &"4".repeat(64),
    )]);

    let receipt = DurableCapabilityPackInstaller::install(
        &backend,
        &journal,
        &pack,
        CapabilityPackInstallOptions::default(),
    )
    .await
    .expect("inactive executable install");

    assert_eq!(receipt.applied.len(), 1);
    assert!(receipt.applied[0].requires_activation);
    assert_eq!(
        *backend.events.lock().expect("events"),
        vec!["snapshot:pack-mcp", "apply:pack-mcp"]
    );
}

#[tokio::test]
async fn complete_semantic_preflight_finishes_before_any_snapshot_or_apply() {
    let backend = FakeBackend {
        fail_validation: Some("alpha".into()),
        ..FakeBackend::default()
    };
    let pack = pack(vec![
        resource(CapabilityResourceKind::Agent, "bravo", &"2".repeat(64)),
        resource(CapabilityResourceKind::Skill, "alpha", &"1".repeat(64)),
    ]);

    let error = DurableCapabilityPackInstaller::install(
        &backend,
        &CapabilityPackTransactionJournal::new(tempfile::TempDir::new().expect("tempdir").path()),
        &pack,
        CapabilityPackInstallOptions::default(),
    )
    .await
    .expect_err("semantic validation failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::ResourceValidationFailed { resource, .. }
            if resource == "skill:alpha"
    ));
    assert!(backend.events.lock().expect("events").is_empty());
}

#[tokio::test]
async fn partial_apply_failure_restores_every_snapshot_in_reverse_order() {
    let backend = FakeBackend {
        fail_apply: Some("charlie".into()),
        ..FakeBackend::default()
    };
    backend.state.lock().expect("state").insert(
        (CapabilityResourceKind::Agent, "bravo".into()),
        "0".repeat(64),
    );
    let pack = pack(vec![
        resource(CapabilityResourceKind::Skill, "alpha", &"1".repeat(64)),
        resource(CapabilityResourceKind::Agent, "bravo", &"2".repeat(64)),
        resource(CapabilityResourceKind::Workflow, "charlie", &"3".repeat(64)),
    ]);

    let error = CapabilityPackInstaller::install(
        &backend,
        &pack,
        CapabilityPackInstallOptions {
            allow_replacements: true,
        },
    )
    .await
    .expect_err("apply failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::ApplyFailed { .. }
    ));
    assert_eq!(
        *backend.events.lock().expect("events"),
        vec![
            "snapshot:bravo",
            "apply:bravo",
            "snapshot:alpha",
            "apply:alpha",
            "snapshot:charlie",
            "apply:charlie",
            "restore:charlie",
            "restore:alpha",
            "restore:bravo",
        ]
    );
    let state = backend.state.lock().expect("state");
    assert_eq!(
        state.get(&(CapabilityResourceKind::Agent, "bravo".into())),
        Some(&"0".repeat(64))
    );
    assert!(!state.contains_key(&(CapabilityResourceKind::Skill, "alpha".into())));
    assert!(!state.contains_key(&(CapabilityResourceKind::Workflow, "charlie".into())));
}

#[tokio::test]
async fn compensation_failure_is_reported_as_a_distinct_terminal_state() {
    let backend = FakeBackend {
        fail_apply: Some("bravo".into()),
        fail_restore: Some("bravo".into()),
        ..FakeBackend::default()
    };
    let pack = pack(vec![
        resource(CapabilityResourceKind::Skill, "alpha", &"1".repeat(64)),
        resource(CapabilityResourceKind::Agent, "bravo", &"2".repeat(64)),
    ]);

    let error =
        CapabilityPackInstaller::install(&backend, &pack, CapabilityPackInstallOptions::default())
            .await
            .expect_err("compensation failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::CompensationFailed { .. }
    ));
}

#[tokio::test]
async fn durable_failure_persists_a_terminal_rolled_back_record() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal = CapabilityPackTransactionJournal::new(temp.path());
    let backend = FakeBackend {
        fail_apply: Some("charlie".into()),
        ..FakeBackend::default()
    };
    let pack = pack(vec![
        resource(CapabilityResourceKind::Skill, "alpha", &"1".repeat(64)),
        resource(CapabilityResourceKind::Agent, "bravo", &"2".repeat(64)),
        resource(CapabilityResourceKind::Workflow, "charlie", &"3".repeat(64)),
    ]);

    let error = DurableCapabilityPackInstaller::install(
        &backend,
        &journal,
        &pack,
        CapabilityPackInstallOptions::default(),
    )
    .await
    .expect_err("apply failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::ApplyFailed { .. }
    ));
    let records = journal.load_all().expect("journal records");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].status,
        CapabilityPackTransactionStatus::RolledBack
    );
    assert!(records[0]
        .resources
        .iter()
        .all(|resource| resource.state == CapabilityPackTransactionState::Restored));
}

#[tokio::test]
async fn journal_failure_after_apply_still_attempts_immediate_compensation() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal_root = temp.path().join("journal");
    let journal = CapabilityPackTransactionJournal::new(&journal_root);
    let backend = FakeBackend {
        sabotage_journal_on_apply: Some(journal_root),
        ..FakeBackend::default()
    };
    let pack = pack(vec![resource(
        CapabilityResourceKind::Skill,
        "alpha",
        &"1".repeat(64),
    )]);

    let error = DurableCapabilityPackInstaller::install(
        &backend,
        &journal,
        &pack,
        CapabilityPackInstallOptions::default(),
    )
    .await
    .expect_err("journal failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::CompensationFailed { .. }
    ));
    assert_eq!(
        *backend.events.lock().expect("events"),
        vec!["snapshot:alpha", "apply:alpha", "restore:alpha"]
    );
    assert!(backend.state.lock().expect("state").is_empty());
}

#[tokio::test]
async fn durable_compensation_failure_persists_recovery_diagnostics() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal = CapabilityPackTransactionJournal::new(temp.path());
    let backend = FakeBackend {
        fail_apply: Some("bravo".into()),
        fail_restore: Some("bravo".into()),
        ..FakeBackend::default()
    };
    let pack = pack(vec![resource(
        CapabilityResourceKind::Agent,
        "bravo",
        &"2".repeat(64),
    )]);

    let error = DurableCapabilityPackInstaller::install(
        &backend,
        &journal,
        &pack,
        CapabilityPackInstallOptions::default(),
    )
    .await
    .expect_err("compensation failure");

    assert!(matches!(
        error,
        CapabilityPackInstallError::CompensationFailed { .. }
    ));
    let records = journal.load_all().expect("journal records");
    assert_eq!(
        records[0].status,
        CapabilityPackTransactionStatus::CompensationFailed
    );
    assert!(records[0]
        .errors
        .iter()
        .any(|error| error.contains("injected apply failure")));
    assert!(records[0]
        .errors
        .iter()
        .any(|error| error.contains("injected restore failure")));
}

#[tokio::test]
async fn startup_recovery_restores_nonterminal_snapshots() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal = CapabilityPackTransactionJournal::new(temp.path());
    let backend = FakeBackend::default();
    let resource = resource(CapabilityResourceKind::Agent, "reviewer", &"2".repeat(64));
    backend
        .state
        .lock()
        .expect("state")
        .insert((resource.kind, resource.id.clone()), "1".repeat(64));
    let pack = pack(vec![resource.clone()]);
    let mut record = journal.begin(&pack).expect("begin transaction");
    let snapshot = backend.snapshot(&resource).await.expect("snapshot");
    backend.apply(&resource).await.expect("simulated apply");
    record.status = CapabilityPackTransactionStatus::Applying;
    record.resources[0].snapshot = Some(serde_json::to_value(snapshot).expect("snapshot json"));
    record.resources[0].state = CapabilityPackTransactionState::Applied;
    journal
        .save(&record)
        .expect("persist interrupted transaction");

    let recovered = DurableCapabilityPackInstaller::recover(&backend, &journal)
        .await
        .expect("startup recovery");

    assert_eq!(recovered, vec![record.id.clone()]);
    assert_eq!(
        backend
            .state
            .lock()
            .expect("state")
            .get(&(resource.kind, resource.id.clone())),
        Some(&"1".repeat(64))
    );
    let recovered_record = journal.load(&record.id).expect("recovered record");
    assert_eq!(
        recovered_record.status,
        CapabilityPackTransactionStatus::RolledBack
    );
    assert_eq!(
        recovered_record.resources[0].state,
        CapabilityPackTransactionState::Restored
    );
}

#[tokio::test]
async fn startup_recovery_surfaces_compensation_failed_transactions() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let journal = CapabilityPackTransactionJournal::new(temp.path());
    let backend = FakeBackend::default();
    let pack = pack(vec![resource(
        CapabilityResourceKind::Agent,
        "reviewer",
        &"2".repeat(64),
    )]);
    let mut record = journal.begin(&pack).expect("begin transaction");
    record.status = CapabilityPackTransactionStatus::CompensationFailed;
    record.errors = vec!["agent:reviewer: injected restore failure".into()];
    journal.save(&record).expect("persist compensation failure");

    let error = DurableCapabilityPackInstaller::recover(&backend, &journal)
        .await
        .expect_err("compensation failure must block startup recovery");

    assert!(matches!(
        error,
        CapabilityPackInstallError::RecoveryFailed {
            transaction_id,
            errors,
        } if transaction_id == record.id && errors == record.errors
    ));
}
