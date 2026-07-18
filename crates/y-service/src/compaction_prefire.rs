//! Service-owned background compaction prefire lifecycle.

use std::collections::HashMap;
use std::future::Future;
use std::ops::Range;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex};
use y_context::{CompactionFailureClass, CompactionLlmError, CompactionOutcome, CompactionResult};
use y_core::types::SessionId;

const TRANSIENT_FAILURE_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefireKey {
    pub fingerprint: String,
    pub range: Range<usize>,
}

pub enum PrefireConsume {
    Miss,
    Stale,
    Suppressed,
    Ready {
        key: PrefireKey,
        result: CompactionResult,
    },
    Failed {
        key: PrefireKey,
        failure: CompactionLlmError,
    },
}

enum PrefireEntry {
    Running {
        key: PrefireKey,
        receiver: oneshot::Receiver<CompactionResult>,
        handle: tokio::task::JoinHandle<()>,
    },
    Failure {
        key: PrefireKey,
        class: CompactionFailureClass,
        retry_at: Instant,
    },
}

/// Per-session lifecycle state for prefired compaction work.
pub struct CompactionPrefireRegistry {
    entries: Mutex<HashMap<SessionId, PrefireEntry>>,
}

impl CompactionPrefireRegistry {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub async fn record_failure(
        &self,
        session_id: SessionId,
        key: PrefireKey,
        class: CompactionFailureClass,
    ) {
        let mut entries = self.entries.lock().await;
        if let Some(PrefireEntry::Running { handle, .. }) = entries.remove(&session_id) {
            handle.abort();
        }
        entries.insert(
            session_id,
            PrefireEntry::Failure {
                key,
                class,
                retry_at: Instant::now() + TRANSIENT_FAILURE_BACKOFF,
            },
        );
    }

    pub async fn is_suppressed(&self, session_id: &SessionId, fingerprint: &str) -> bool {
        self.entries
            .lock()
            .await
            .get(session_id)
            .is_some_and(|entry| match entry {
                PrefireEntry::Failure {
                    key,
                    class,
                    retry_at,
                } if key.fingerprint == fingerprint => {
                    *class == CompactionFailureClass::Deterministic || Instant::now() < *retry_at
                }
                PrefireEntry::Failure { .. } | PrefireEntry::Running { .. } => false,
            })
    }

    pub async fn schedule<F>(&self, session_id: SessionId, key: PrefireKey, future: F) -> bool
    where
        F: Future<Output = CompactionResult> + Send + 'static,
    {
        let mut entries = self.entries.lock().await;
        if let Some(existing) = entries.get(&session_id) {
            match existing {
                PrefireEntry::Running {
                    key: existing_key, ..
                } if existing_key == &key => return false,
                PrefireEntry::Failure {
                    key: failed_key,
                    class,
                    retry_at,
                } if failed_key == &key
                    && (*class == CompactionFailureClass::Deterministic
                        || Instant::now() < *retry_at) =>
                {
                    return false;
                }
                PrefireEntry::Running { handle, .. } => handle.abort(),
                PrefireEntry::Failure { .. } => {}
            }
        }

        let (sender, receiver) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let result = future.await;
            let _ = sender.send(result);
        });
        entries.insert(
            session_id,
            PrefireEntry::Running {
                key,
                receiver,
                handle,
            },
        );
        true
    }

    pub async fn consume(&self, session_id: &SessionId, fingerprint: &str) -> PrefireConsume {
        let entry = self.entries.lock().await.remove(session_id);
        match entry {
            None => PrefireConsume::Miss,
            Some(PrefireEntry::Failure {
                key,
                class,
                retry_at,
            }) => {
                if key.fingerprint == fingerprint
                    && (class == CompactionFailureClass::Deterministic || Instant::now() < retry_at)
                {
                    self.entries.lock().await.insert(
                        session_id.clone(),
                        PrefireEntry::Failure {
                            key,
                            class,
                            retry_at,
                        },
                    );
                    PrefireConsume::Suppressed
                } else {
                    PrefireConsume::Stale
                }
            }
            Some(PrefireEntry::Running {
                key,
                receiver,
                handle,
            }) => {
                if key.fingerprint != fingerprint {
                    handle.abort();
                    return PrefireConsume::Stale;
                }
                let Ok(result) = receiver.await else {
                    let failure = CompactionLlmError::transient(
                        "prefire task ended before producing a result",
                    );
                    self.record_failure(session_id.clone(), key.clone(), failure.class)
                        .await;
                    return PrefireConsume::Failed { key, failure };
                };
                match &result.outcome {
                    CompactionOutcome::Fallback {
                        failure: Some(failure),
                    } => {
                        let failure = failure.clone();
                        self.record_failure(session_id.clone(), key.clone(), failure.class)
                            .await;
                        PrefireConsume::Failed { key, failure }
                    }
                    CompactionOutcome::Fallback { failure: None } => {
                        let failure = CompactionLlmError::deterministic(
                            "prefire produced an unclassified fallback",
                        );
                        self.record_failure(session_id.clone(), key.clone(), failure.class)
                            .await;
                        PrefireConsume::Failed { key, failure }
                    }
                    CompactionOutcome::Noop | CompactionOutcome::Summarized => {
                        PrefireConsume::Ready { key, result }
                    }
                }
            }
        }
    }

    pub async fn pending_key(&self, session_id: &SessionId) -> Option<PrefireKey> {
        self.entries
            .lock()
            .await
            .get(session_id)
            .map(|entry| match entry {
                PrefireEntry::Running { key, .. } | PrefireEntry::Failure { key, .. } => {
                    key.clone()
                }
            })
    }

    pub async fn clear(&self, session_id: &SessionId) {
        if let Some(PrefireEntry::Running { handle, .. }) =
            self.entries.lock().await.remove(session_id)
        {
            handle.abort();
        }
    }
}

impl Default for CompactionPrefireRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use y_context::{CompactionFailureClass, CompactionOutcome, CompactionResult};
    use y_core::types::SessionId;

    use super::{CompactionPrefireRegistry, PrefireConsume, PrefireKey};

    #[tokio::test]
    async fn deterministic_failure_is_suppressed_until_fingerprint_changes() {
        let registry = CompactionPrefireRegistry::new();
        let session_id = SessionId("session-1".to_string());

        registry
            .record_failure(
                session_id.clone(),
                PrefireKey {
                    fingerprint: "fingerprint-a".to_string(),
                    range: 0..2,
                },
                CompactionFailureClass::Deterministic,
            )
            .await;

        assert!(registry.is_suppressed(&session_id, "fingerprint-a").await);
        assert!(!registry.is_suppressed(&session_id, "fingerprint-b").await);
    }

    #[tokio::test]
    async fn prefire_result_is_rejected_when_fingerprint_is_stale() {
        let registry = CompactionPrefireRegistry::new();
        let session_id = SessionId("session-1".to_string());
        registry
            .schedule(
                session_id.clone(),
                PrefireKey {
                    fingerprint: "fingerprint-a".to_string(),
                    range: 0..2,
                },
                async {
                    CompactionResult {
                        summary: "summary".to_string(),
                        messages_compacted: 2,
                        tokens_saved: 100,
                        summary_tokens: 10,
                        outcome: CompactionOutcome::Summarized,
                    }
                },
            )
            .await;

        let outcome = registry.consume(&session_id, "fingerprint-b").await;

        assert!(matches!(outcome, PrefireConsume::Stale));
    }

    #[tokio::test]
    async fn clear_removes_pending_prefire_work() {
        let registry = CompactionPrefireRegistry::new();
        let session_id = SessionId("session-1".to_string());
        registry
            .schedule(
                session_id.clone(),
                PrefireKey {
                    fingerprint: "fingerprint-a".to_string(),
                    range: 0..2,
                },
                std::future::pending(),
            )
            .await;

        registry.clear(&session_id).await;

        assert!(registry.pending_key(&session_id).await.is_none());
    }
}
