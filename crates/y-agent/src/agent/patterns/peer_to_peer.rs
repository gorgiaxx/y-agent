//! Peer-to-peer pattern: agents communicate through shared typed channels.
//!
//! Design reference: multi-agent-design.md §Collaboration Patterns
//!
//! In P2P collaboration, agents exchange messages through shared channels
//! rather than having a coordinator. A channel reducer aggregates final
//! results from all participating agents.

use std::sync::{Arc, Mutex};

use crate::agent::delegation::{DelegationProtocol, DelegationResult, DelegationTask};
use crate::agent::error::MultiAgentError;

// ---------------------------------------------------------------------------
// Channel message
// ---------------------------------------------------------------------------

/// A message exchanged through a P2P channel.
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    /// Agent that sent the message.
    pub sender: String,
    /// Message content.
    pub content: String,
    /// Optional typed tag for filtering.
    pub tag: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared channel
// ---------------------------------------------------------------------------

/// A shared communication channel for P2P agent collaboration.
///
/// Thread-safe: agents can read/write concurrently through `Arc<Mutex<_>>`.
#[derive(Debug, Clone)]
pub struct SharedChannel {
    name: String,
    messages: Arc<Mutex<Vec<ChannelMessage>>>,
}

impl SharedChannel {
    /// Create a new named channel.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Channel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Send a message to the channel.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn send(&self, sender: &str, content: &str, tag: Option<&str>) {
        let mut msgs = self.messages.lock().expect("channel lock");
        msgs.push(ChannelMessage {
            sender: sender.to_string(),
            content: content.to_string(),
            tag: tag.map(ToString::to_string),
        });
    }

    /// Read all messages from the channel.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn read_all(&self) -> Vec<ChannelMessage> {
        let msgs = self.messages.lock().expect("channel lock");
        msgs.clone()
    }

    /// Read messages filtered by tag.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn read_by_tag(&self, tag: &str) -> Vec<ChannelMessage> {
        let msgs = self.messages.lock().expect("channel lock");
        msgs.iter()
            .filter(|m| m.tag.as_deref() == Some(tag))
            .cloned()
            .collect()
    }

    /// Number of messages in the channel.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn len(&self) -> usize {
        let msgs = self.messages.lock().expect("channel lock");
        msgs.len()
    }

    /// Whether the channel is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Channel reducer
// ---------------------------------------------------------------------------

/// Reduces channel messages into a single aggregated result.
pub trait ChannelReducer: Send + Sync {
    /// Reduce all channel messages into a single output string.
    fn reduce(&self, messages: &[ChannelMessage]) -> String;
}

/// Concatenation reducer: joins all message contents with newlines.
pub struct ConcatReducer;

impl ChannelReducer for ConcatReducer {
    fn reduce(&self, messages: &[ChannelMessage]) -> String {
        messages
            .iter()
            .map(|m| format!("[{}]: {}", m.sender, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Last-value reducer: returns the last message content.
pub struct LastValueReducer;

impl ChannelReducer for LastValueReducer {
    fn reduce(&self, messages: &[ChannelMessage]) -> String {
        messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// P2P pattern
// ---------------------------------------------------------------------------

/// Peer-to-peer collaboration pattern.
///
/// Agents execute concurrently (simulated sequentially here) and communicate
/// through shared channels. A reducer aggregates the final results.
#[derive(Debug)]
pub struct PeerToPeerPattern;

impl PeerToPeerPattern {
    /// Execute agents in P2P mode with a shared channel.
    ///
    /// Each agent executes its task and writes to the shared channel.
    /// After all agents complete, the reducer produces the final output.
    pub fn execute(
        protocol: &DelegationProtocol,
        tasks: &[DelegationTask],
        channel: &SharedChannel,
        reducer: &dyn ChannelReducer,
    ) -> Result<String, MultiAgentError> {
        let mut results: Vec<DelegationResult> = Vec::new();

        // Execute each agent (simulated sequentially)
        for task in tasks {
            let result = protocol.execute_sync(task)?;

            // Agent writes its result to shared channel
            channel.send(&result.agent_id, &result.output, None);

            results.push(result);
        }

        // Reduce channel messages
        let messages = channel.read_all();
        Ok(reducer.reduce(&messages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;

    /// T-MA-P7-07: P2P channel communication between 3 agents.
    #[test]
    fn test_peer_to_peer_channel_communication() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let channel = SharedChannel::new("collab");
        let tasks = vec![
            protocol.create_task("agent-a", "research part A"),
            protocol.create_task("agent-b", "research part B"),
            protocol.create_task("agent-c", "research part C"),
        ];

        let result =
            PeerToPeerPattern::execute(&protocol, &tasks, &channel, &ConcatReducer).unwrap();

        // Channel should have 3 messages
        assert_eq!(channel.len(), 3);
        let msgs = channel.read_all();
        assert_eq!(msgs[0].sender, "agent-a");
        assert_eq!(msgs[1].sender, "agent-b");
        assert_eq!(msgs[2].sender, "agent-c");

        // Reduced output should contain all agents
        assert!(result.contains("[agent-a]"));
        assert!(result.contains("[agent-b]"));
        assert!(result.contains("[agent-c]"));
    }

    /// T-MA-P7-08: Channel reducer aggregates results correctly.
    #[test]
    fn test_peer_to_peer_channel_reducer() {
        let channel = SharedChannel::new("test");
        channel.send("a1", "result 1", None);
        channel.send("a2", "result 2", None);
        channel.send("a3", "result 3", None);

        // Concat reducer
        let concat = ConcatReducer;
        let output = concat.reduce(&channel.read_all());
        assert!(output.contains("[a1]: result 1"));
        assert!(output.contains("[a2]: result 2"));
        assert!(output.contains("[a3]: result 3"));

        // Last value reducer
        let last = LastValueReducer;
        let output = last.reduce(&channel.read_all());
        assert_eq!(output, "result 3");
    }

    /// Channel tag filtering works correctly.
    #[test]
    fn test_channel_tag_filtering() {
        let channel = SharedChannel::new("tagged");
        channel.send("a1", "code review", Some("review"));
        channel.send("a2", "test results", Some("test"));
        channel.send("a3", "another review", Some("review"));

        let reviews = channel.read_by_tag("review");
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[0].sender, "a1");
        assert_eq!(reviews[1].sender, "a3");

        let tests = channel.read_by_tag("test");
        assert_eq!(tests.len(), 1);
    }

    /// Empty channel returns empty from reducer.
    #[test]
    fn test_empty_channel_reducer() {
        let channel = SharedChannel::new("empty");
        assert!(channel.is_empty());

        let concat = ConcatReducer;
        assert!(concat.reduce(&channel.read_all()).is_empty());

        let last = LastValueReducer;
        assert!(last.reduce(&channel.read_all()).is_empty());
    }
}
