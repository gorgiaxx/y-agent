//! Message scheduling: queue modes for message delivery ordering.
//!
//! Design reference: message-scheduling-design.md
//!
//! Three queue modes control how messages are ordered before delivery:
//! - **FIFO**: First-in-first-out (default)
//! - **Priority**: Ordered by urgency score (higher = more urgent)
//! - **Dedup**: FIFO with content-hash deduplication within a window

use std::collections::{BinaryHeap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Scheduling mode for message queues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SchedulingMode {
    /// First-in-first-out ordering.
    #[default]
    Fifo,
    /// Priority-based ordering (higher priority dequeued first).
    Priority,
    /// FIFO with content-hash deduplication.
    Dedup,
}

/// A message queued for scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    /// Unique message identifier.
    pub id: String,
    /// Message content.
    pub content: String,
    /// Priority (higher = more urgent). Used in Priority mode.
    pub priority: u32,
    /// Timestamp for ordering (epoch millis).
    pub timestamp_ms: u64,
}

impl PartialEq for QueuedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for QueuedMessage {}

impl PartialOrd for QueuedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first; break ties by earlier timestamp.
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.timestamp_ms.cmp(&self.timestamp_ms))
    }
}

// ---------------------------------------------------------------------------
// Scheduling queue
// ---------------------------------------------------------------------------

/// A scheduling queue that supports FIFO, Priority, and Dedup modes.
pub struct SchedulingQueue {
    mode: SchedulingMode,
    /// FIFO / Dedup backing store.
    fifo: VecDeque<QueuedMessage>,
    /// Priority backing store.
    heap: BinaryHeap<QueuedMessage>,
    /// Content hashes for dedup mode.
    seen_hashes: HashSet<String>,
}

impl SchedulingQueue {
    /// Create a new scheduling queue with the specified mode.
    pub fn new(mode: SchedulingMode) -> Self {
        Self {
            mode,
            fifo: VecDeque::new(),
            heap: BinaryHeap::new(),
            seen_hashes: HashSet::new(),
        }
    }

    /// Get the current scheduling mode.
    pub fn mode(&self) -> SchedulingMode {
        self.mode
    }

    /// Enqueue a message.
    ///
    /// In **Dedup** mode, messages with duplicate content hashes are
    /// silently dropped (returns `false`). Otherwise returns `true`.
    pub fn enqueue(&mut self, msg: QueuedMessage) -> bool {
        match self.mode {
            SchedulingMode::Fifo => {
                self.fifo.push_back(msg);
                true
            }
            SchedulingMode::Priority => {
                self.heap.push(msg);
                true
            }
            SchedulingMode::Dedup => {
                let hash = content_hash(&msg.content);
                if self.seen_hashes.contains(&hash) {
                    return false; // duplicate
                }
                self.seen_hashes.insert(hash);
                self.fifo.push_back(msg);
                true
            }
        }
    }

    /// Dequeue the next message according to the scheduling mode.
    pub fn dequeue(&mut self) -> Option<QueuedMessage> {
        match self.mode {
            SchedulingMode::Fifo | SchedulingMode::Dedup => self.fifo.pop_front(),
            SchedulingMode::Priority => self.heap.pop(),
        }
    }

    /// Peek at the next message without removing it.
    pub fn peek(&self) -> Option<&QueuedMessage> {
        match self.mode {
            SchedulingMode::Fifo | SchedulingMode::Dedup => self.fifo.front(),
            SchedulingMode::Priority => self.heap.peek(),
        }
    }

    /// Number of messages in the queue.
    pub fn len(&self) -> usize {
        match self.mode {
            SchedulingMode::Fifo | SchedulingMode::Dedup => self.fifo.len(),
            SchedulingMode::Priority => self.heap.len(),
        }
    }

    /// Check if the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all messages and dedup state.
    pub fn clear(&mut self) {
        self.fifo.clear();
        self.heap.clear();
        self.seen_hashes.clear();
    }

    /// Clear only the dedup hash set (allows previously-seen content again).
    pub fn reset_dedup_window(&mut self) {
        self.seen_hashes.clear();
    }
}

/// Compute SHA-256 content hash for deduplication.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("{result:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, content: &str, priority: u32, ts: u64) -> QueuedMessage {
        QueuedMessage {
            id: id.to_string(),
            content: content.to_string(),
            priority,
            timestamp_ms: ts,
        }
    }

    /// T-P3-37-01: FIFO mode preserves insertion order.
    #[test]
    fn test_fifo_order() {
        let mut q = SchedulingQueue::new(SchedulingMode::Fifo);
        q.enqueue(msg("a", "first", 0, 1));
        q.enqueue(msg("b", "second", 0, 2));
        q.enqueue(msg("c", "third", 0, 3));

        assert_eq!(q.len(), 3);
        assert_eq!(q.dequeue().unwrap().id, "a");
        assert_eq!(q.dequeue().unwrap().id, "b");
        assert_eq!(q.dequeue().unwrap().id, "c");
        assert!(q.is_empty());
    }

    /// T-P3-37-02: Priority mode orders by priority (descending).
    #[test]
    fn test_priority_ordering() {
        let mut q = SchedulingQueue::new(SchedulingMode::Priority);
        q.enqueue(msg("low", "lo", 1, 1));
        q.enqueue(msg("high", "hi", 10, 2));
        q.enqueue(msg("med", "md", 5, 3));

        assert_eq!(q.dequeue().unwrap().id, "high");
        assert_eq!(q.dequeue().unwrap().id, "med");
        assert_eq!(q.dequeue().unwrap().id, "low");
    }

    /// T-P3-37-03: Priority mode breaks ties by earlier timestamp.
    #[test]
    fn test_priority_timestamp_tiebreak() {
        let mut q = SchedulingQueue::new(SchedulingMode::Priority);
        q.enqueue(msg("later", "msg", 5, 200));
        q.enqueue(msg("earlier", "msg", 5, 100));

        // Same priority → earlier timestamp wins.
        assert_eq!(q.dequeue().unwrap().id, "earlier");
        assert_eq!(q.dequeue().unwrap().id, "later");
    }

    /// T-P3-37-04: Dedup mode drops messages with identical content.
    #[test]
    fn test_dedup_drops_duplicate() {
        let mut q = SchedulingQueue::new(SchedulingMode::Dedup);
        let accepted = q.enqueue(msg("a", "hello", 0, 1));
        assert!(accepted);

        let rejected = q.enqueue(msg("b", "hello", 0, 2));
        assert!(!rejected);

        assert_eq!(q.len(), 1);
        assert_eq!(q.dequeue().unwrap().id, "a");
    }

    /// T-P3-37-05: Dedup mode allows messages with distinct content.
    #[test]
    fn test_dedup_allows_unique() {
        let mut q = SchedulingQueue::new(SchedulingMode::Dedup);
        assert!(q.enqueue(msg("a", "hello", 0, 1)));
        assert!(q.enqueue(msg("b", "world", 0, 2)));

        assert_eq!(q.len(), 2);
    }

    /// T-P3-37-06: Dedup window reset allows previously-seen content.
    #[test]
    fn test_dedup_window_reset() {
        let mut q = SchedulingQueue::new(SchedulingMode::Dedup);
        q.enqueue(msg("a", "hello", 0, 1));
        assert!(!q.enqueue(msg("b", "hello", 0, 2)));

        q.reset_dedup_window();
        assert!(q.enqueue(msg("c", "hello", 0, 3)));
        assert_eq!(q.len(), 2); // a + c
    }

    /// T-P3-37-07: Peek does not remove the message.
    #[test]
    fn test_peek_non_destructive() {
        let mut q = SchedulingQueue::new(SchedulingMode::Fifo);
        q.enqueue(msg("a", "first", 0, 1));

        assert_eq!(q.peek().unwrap().id, "a");
        assert_eq!(q.peek().unwrap().id, "a"); // still there
        assert_eq!(q.len(), 1);
    }

    /// T-P3-37-08: Empty queue returns None.
    #[test]
    fn test_empty_queue() {
        let mut q = SchedulingQueue::new(SchedulingMode::Priority);
        assert!(q.dequeue().is_none());
        assert!(q.peek().is_none());
        assert!(q.is_empty());
    }
}
