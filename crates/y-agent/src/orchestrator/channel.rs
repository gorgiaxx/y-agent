//! Typed state channels with configurable reducers.

use serde::{Deserialize, Serialize};

/// Channel reducer type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelType {
    /// Last write wins (default, backward-compatible).
    #[default]
    LastValue,
    /// Accumulates values into an array.
    Append,
    /// Deep-merges JSON objects.
    Merge,
}

/// A typed channel holding state.
#[derive(Debug, Clone)]
pub struct Channel {
    /// Channel name.
    pub name: String,
    /// Current value.
    pub value: serde_json::Value,
    /// Reducer type.
    pub channel_type: ChannelType,
    /// Write version counter.
    pub version: u64,
}

impl Channel {
    /// Create a new channel with the given type.
    pub fn new(name: &str, channel_type: ChannelType) -> Self {
        Self {
            name: name.to_string(),
            value: serde_json::Value::Null,
            channel_type,
            version: 0,
        }
    }

    /// Write a value using the channel's reducer.
    pub fn write(&mut self, value: serde_json::Value) {
        self.version += 1;
        match &self.channel_type {
            ChannelType::LastValue => {
                self.value = value;
            }
            ChannelType::Append => {
                if let serde_json::Value::Array(ref mut arr) = self.value {
                    arr.push(value);
                } else {
                    self.value = serde_json::Value::Array(vec![value]);
                }
            }
            ChannelType::Merge => {
                if self.value.is_null() {
                    self.value = serde_json::Value::Object(serde_json::Map::new());
                }
                if let (
                    serde_json::Value::Object(ref mut existing),
                    serde_json::Value::Object(incoming),
                ) = (&mut self.value, value)
                {
                    for (k, v) in incoming {
                        existing.insert(k, v);
                    }
                }
            }
        }
    }

    /// Read the current value.
    pub fn read(&self) -> &serde_json::Value {
        &self.value
    }
}

/// Workflow context: manages typed channels.
#[derive(Debug)]
pub struct WorkflowContext {
    channels: std::collections::HashMap<String, Channel>,
}

impl WorkflowContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self {
            channels: std::collections::HashMap::new(),
        }
    }

    /// Define a channel.
    pub fn define_channel(&mut self, name: &str, channel_type: ChannelType) {
        self.channels
            .insert(name.to_string(), Channel::new(name, channel_type));
    }

    /// Write to a channel (creates `LastValue` channel if not defined).
    pub fn write(&mut self, channel: &str, value: serde_json::Value) {
        self.channels
            .entry(channel.to_string())
            .or_insert_with(|| Channel::new(channel, ChannelType::LastValue))
            .write(value);
    }

    /// Read from a channel.
    pub fn read(&self, channel: &str) -> Option<&serde_json::Value> {
        self.channels.get(channel).map(Channel::read)
    }

    /// Get all channel names.
    pub fn channel_names(&self) -> Vec<&str> {
        self.channels.keys().map(String::as_str).collect()
    }
}

impl Default for WorkflowContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_last_value_channel() {
        let mut ch = Channel::new("x", ChannelType::LastValue);
        ch.write(serde_json::json!(1));
        ch.write(serde_json::json!(2));
        assert_eq!(*ch.read(), serde_json::json!(2));
        assert_eq!(ch.version, 2);
    }

    #[test]
    fn test_append_channel() {
        let mut ch = Channel::new("results", ChannelType::Append);
        ch.write(serde_json::json!("a"));
        ch.write(serde_json::json!("b"));
        assert_eq!(*ch.read(), serde_json::json!(["a", "b"]));
    }

    #[test]
    fn test_merge_channel() {
        let mut ch = Channel::new("data", ChannelType::Merge);
        ch.write(serde_json::json!({"a": 1}));
        ch.write(serde_json::json!({"b": 2}));
        assert_eq!(*ch.read(), serde_json::json!({"a": 1, "b": 2}));
    }

    #[test]
    fn test_merge_overwrites_keys() {
        let mut ch = Channel::new("data", ChannelType::Merge);
        ch.write(serde_json::json!({"a": 1}));
        ch.write(serde_json::json!({"a": 2}));
        assert_eq!(*ch.read(), serde_json::json!({"a": 2}));
    }

    #[test]
    fn test_workflow_context() {
        let mut ctx = WorkflowContext::new();
        ctx.define_channel("results", ChannelType::Append);
        ctx.write("results", serde_json::json!("item1"));
        ctx.write("results", serde_json::json!("item2"));

        // Auto-created LastValue channel.
        ctx.write("status", serde_json::json!("done"));

        assert_eq!(
            ctx.read("results"),
            Some(&serde_json::json!(["item1", "item2"]))
        );
        assert_eq!(ctx.read("status"), Some(&serde_json::json!("done")));
        assert_eq!(ctx.read("nonexistent"), None);
    }
}
