//! Trigger queue: async bounded channel for enqueuing fired triggers.

use tokio::sync::mpsc;

use crate::trigger::FiredTrigger;

/// Sender half of the trigger queue.
pub type TriggerSender = mpsc::Sender<FiredTrigger>;

/// Receiver half of the trigger queue.
pub type TriggerReceiver = mpsc::Receiver<FiredTrigger>;

/// Default capacity for the trigger queue.
const DEFAULT_QUEUE_CAPACITY: usize = 256;

/// Create a new trigger queue with the default capacity.
pub fn trigger_queue() -> (TriggerSender, TriggerReceiver) {
    mpsc::channel(DEFAULT_QUEUE_CAPACITY)
}

/// Create a trigger queue with a custom capacity.
pub fn trigger_queue_with_capacity(capacity: usize) -> (TriggerSender, TriggerReceiver) {
    mpsc::channel(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::trigger::TriggerType;

    #[tokio::test]
    async fn test_trigger_queue_send_receive() {
        let (tx, mut rx) = trigger_queue();

        let trigger = FiredTrigger {
            schedule_id: "test".into(),
            fired_at: Utc::now(),
            trigger_type: TriggerType::Interval,
        };

        tx.send(trigger.clone()).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received.schedule_id, "test");
    }

    #[tokio::test]
    async fn test_trigger_queue_closes_on_drop() {
        let (tx, mut rx) = trigger_queue();
        drop(tx);
        let result = rx.recv().await;
        assert!(result.is_none());
    }
}
