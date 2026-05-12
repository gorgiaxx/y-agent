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
    /// Process a tool-call delta from a streaming chunk.
    ///
    /// When `index` is `None`, the delta is assigned the next sequential
    /// slot (`entries.len()`). This matches the behavior of the Vercel
    /// `@ai-sdk/openai-compatible` reference and is required for providers
    /// (e.g. Google Gemini's OpenAI-compat surface) that omit `index` for
    /// parallel tool calls. Defaulting to `0` would merge every call into
    /// a single corrupt entry.
    pub fn process_delta(
        &mut self,
        index: Option<usize>,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) {
        let index = index.unwrap_or(self.entries.len());
        while self.entries.len() <= index {
            self.entries.push(Entry::default());
        }
        let entry = &mut self.entries[index];
        if let Some(id) = id.filter(|s| !s.is_empty()) {
            entry.id.clear();
            entry.id.push_str(id);
        }
        if let Some(name) = name.filter(|s| !s.is_empty()) {
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
            .filter(|e| !e.id.is_empty() && !e.name.is_empty())
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

        acc.process_delta(Some(0), Some("call_1"), Some("search"), Some(r#"{"q":"#));
        acc.process_delta(Some(0), None, None, Some(r#""rust"}"#));

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments, serde_json::json!({"q": "rust"}));
    }

    #[test]
    fn parallel_tool_calls() {
        let mut acc = ToolCallAccumulatorSet::default();

        acc.process_delta(
            Some(0),
            Some("call_1"),
            Some("search"),
            Some(r#"{"q":"a"}"#),
        );
        acc.process_delta(
            Some(1),
            Some("call_2"),
            Some("fetch"),
            Some(r#"{"url":"b"}"#),
        );

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[1].name, "fetch");
    }

    #[test]
    fn invalid_json_stored_as_string() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), Some("call_1"), Some("broken"), Some("not json"));

        let calls = acc.drain_completed();
        assert_eq!(
            calls[0].arguments,
            serde_json::Value::String("not json".into())
        );
    }

    #[test]
    fn empty_id_filtered() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), None, Some("ghost"), Some("{}"));

        let calls = acc.drain_completed();
        assert!(calls.is_empty());
    }

    #[test]
    fn empty_string_name_does_not_overwrite() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), Some("call_1"), Some("search"), Some(r#"{"q":"#));
        acc.process_delta(Some(0), None, Some(""), Some(r#""rust"}"#));

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments, serde_json::json!({"q": "rust"}));
    }

    #[test]
    fn empty_string_id_does_not_overwrite() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), Some("call_1"), Some("search"), Some("{}"));
        acc.process_delta(Some(0), Some(""), None, None);

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn empty_name_filtered() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), Some("call_1"), None, Some("{}"));

        let calls = acc.drain_completed();
        assert!(calls.is_empty());
    }

    #[test]
    fn drain_clears_state() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(0), Some("call_1"), Some("test"), Some("{}"));
        let _ = acc.drain_completed();
        assert!(acc.entries.is_empty());
    }

    #[test]
    fn gap_indices_handled() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(Some(2), Some("call_3"), Some("third"), Some("{}"));

        assert_eq!(acc.entries.len(), 3);
        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_3");
    }

    /// Regression: when a provider omits `index` (e.g. Gemini's OpenAI-compat
    /// surface), each delta with `id`+`name` set must land in its own slot,
    /// not collapse into entry 0.
    #[test]
    fn missing_index_assigns_sequential_slots() {
        let mut acc = ToolCallAccumulatorSet::default();
        acc.process_delta(None, Some("call_1"), Some("a"), Some(r#"{"x":1}"#));
        acc.process_delta(None, Some("call_2"), Some("b"), Some(r#"{"y":2}"#));

        let calls = acc.drain_completed();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "a");
        assert_eq!(calls[0].arguments, serde_json::json!({"x": 1}));
        assert_eq!(calls[1].id, "call_2");
        assert_eq!(calls[1].name, "b");
        assert_eq!(calls[1].arguments, serde_json::json!({"y": 2}));
    }
}
