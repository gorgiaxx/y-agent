//! Shared tool-call accumulation for OpenAI-compatible streaming protocols.
//!
//! `OpenAI` and Azure send tool calls as incremental deltas keyed by `index`.
//! This module provides a single reusable accumulator that both providers share.

use y_core::types::ToolCallRequest;

#[derive(Debug, Clone, Default)]
struct Entry {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
pub(crate) struct ToolCallAccumulatorSet {
    entries: Vec<Entry>,
}

impl ToolCallAccumulatorSet {
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) {
        while self.entries.len() <= index {
            self.entries.push(Entry::default());
        }
        let entry = &mut self.entries[index];
        if let Some(id) = id {
            entry.id.clear();
            entry.id.push_str(id);
        }
        if let Some(name) = name {
            entry.name.clear();
            entry.name.push_str(name);
        }
        if let Some(args) = args {
            entry.arguments.push_str(args);
        }
    }

    pub fn drain_completed(&mut self) -> Vec<ToolCallRequest> {
        self.entries
            .drain(..)
            .filter(|e| !e.id.is_empty())
            .map(|e| ToolCallRequest {
                id: e.id,
                name: e.name,
                arguments: serde_json::from_str(&e.arguments)
                    .unwrap_or(serde_json::Value::String(e.arguments)),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tool_call_accumulation() {
        let mut acc = ToolCallAccumulatorSet::default();

        acc.process_delta(0, Some("call_1"), Some("search"), Some(r#"{"q":"#));
        acc.process_delta(0, None, None, Some(r#""rust"}"#));

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments, serde_json::json!({"q": "rust"}));
    }

    #[test]
    fn parallel_tool_calls() {
        let mut acc = ToolCallAccumulatorSet::default();

        acc.process_delta(0, Some("call_1"), Some("search"), Some(r#"{"q":"a"}"#));
        acc.process_delta(1, Some("call_2"), Some("fetch"), Some(r#"{"url":"b"}"#));

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[1].name, "fetch");
    }

    #[test]
    fn invalid_json_stored_as_string() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(0, Some("call_1"), Some("broken"), Some("not json"));

        let calls = acc.drain_completed();
        assert_eq!(
            calls[0].arguments,
            serde_json::Value::String("not json".into())
        );
    }

    #[test]
    fn empty_id_filtered() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(0, None, Some("ghost"), Some("{}"));

        let calls = acc.drain_completed();
        assert!(calls.is_empty());
    }

    #[test]
    fn drain_clears_state() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(0, Some("call_1"), Some("test"), Some("{}"));
        let _ = acc.drain_completed();
        assert!(acc.entries.is_empty());
    }

    #[test]
    fn gap_indices_handled() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(2, Some("call_3"), Some("third"), Some("{}"));

        assert_eq!(acc.entries.len(), 3);
        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_3");
    }
}
