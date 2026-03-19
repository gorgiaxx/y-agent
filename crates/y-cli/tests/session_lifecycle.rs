//! E2E integration test: Session lifecycle (create → branch → resume → archive).

use y_core::session::{
    CreateSessionOptions, SessionFilter, SessionState, SessionStore, SessionType, TranscriptStore,
};
use y_core::types::Role;
use y_test_utils::{
    make_assistant_message, make_user_message, MockSessionStore, MockTranscriptStore,
};

#[tokio::test]
async fn e2e_session_create_and_branch() {
    let store = MockSessionStore::new();

    // Create main session
    let main = store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Main conversation".into()),
        })
        .await
        .unwrap();
    assert_eq!(main.state, SessionState::Active);

    // Create a branch from main
    let branch = store
        .create(CreateSessionOptions {
            parent_id: Some(main.id.clone()),
            session_type: SessionType::Branch,
            agent_id: None,
            title: Some("Branch: explore idea".into()),
        })
        .await
        .unwrap();
    assert_eq!(branch.session_type, SessionType::Branch);

    // Verify parent-child relationship
    let children = store.children(&main.id).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, branch.id);
}

#[tokio::test]
async fn e2e_session_state_transitions() {
    let store = MockSessionStore::new();

    let session = store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("State test".into()),
        })
        .await
        .unwrap();

    // Active → Paused
    store
        .set_state(&session.id, SessionState::Paused)
        .await
        .unwrap();
    let fetched = store.get(&session.id).await.unwrap();
    assert_eq!(fetched.state, SessionState::Paused);

    // Paused → Active (resume)
    store
        .set_state(&session.id, SessionState::Active)
        .await
        .unwrap();
    let fetched = store.get(&session.id).await.unwrap();
    assert_eq!(fetched.state, SessionState::Active);

    // Active → Archived
    store
        .set_state(&session.id, SessionState::Archived)
        .await
        .unwrap();
    let fetched = store.get(&session.id).await.unwrap();
    assert_eq!(fetched.state, SessionState::Archived);
}

#[tokio::test]
async fn e2e_session_with_transcripts() {
    let session_store = MockSessionStore::new();
    let transcript_store = MockTranscriptStore::new();

    let session = session_store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Transcript test".into()),
        })
        .await
        .unwrap();

    // Append conversation
    transcript_store
        .append(&session.id, &make_user_message("Hello"))
        .await
        .unwrap();
    transcript_store
        .append(&session.id, &make_assistant_message("Hi there!"))
        .await
        .unwrap();
    transcript_store
        .append(&session.id, &make_user_message("What is 2+2?"))
        .await
        .unwrap();
    transcript_store
        .append(&session.id, &make_assistant_message("4"))
        .await
        .unwrap();

    // Verify message count
    let count = transcript_store.message_count(&session.id).await.unwrap();
    assert_eq!(count, 4);

    // Read last 2
    let last2 = transcript_store.read_last(&session.id, 2).await.unwrap();
    assert_eq!(last2.len(), 2);
    assert_eq!(last2[0].role, Role::User);
    assert_eq!(last2[1].role, Role::Assistant);

    // Update session metadata
    session_store
        .update_metadata(&session.id, None, 150, 4)
        .await
        .unwrap();
    let updated = session_store.get(&session.id).await.unwrap();
    assert_eq!(updated.message_count, 4);
    assert_eq!(updated.token_count, 150);
}

#[tokio::test]
async fn e2e_session_filter_by_state() {
    let store = MockSessionStore::new();

    // Create 3 sessions
    #[allow(unused_variables)]
    let s1 = store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Active session".into()),
        })
        .await
        .unwrap();

    let s2 = store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Archived session".into()),
        })
        .await
        .unwrap();

    store
        .set_state(&s2.id, SessionState::Archived)
        .await
        .unwrap();

    let _s3 = store
        .create(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Another active".into()),
        })
        .await
        .unwrap();

    // Filter active only
    let active = store
        .list(&SessionFilter {
            state: Some(SessionState::Active),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(active.len(), 2);

    // Filter archived
    let archived = store
        .list(&SessionFilter {
            state: Some(SessionState::Archived),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(archived.len(), 1);
}
