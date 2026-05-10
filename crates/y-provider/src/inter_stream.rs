//! Internal intermediate stream events for provider-specific parsers.
//!
//! Each provider emits `InterStreamEvent` values from its wire protocol;
//! `inter_stream_adapter::into_chat_stream` converts them to the public
//! `ChatStreamChunk` type defined in `y-core`.

use y_core::provider::FinishReason;
use y_core::types::{TokenUsage, ToolCallRequest};

#[derive(Debug, Clone)]
pub(crate) enum InterStreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCall(ToolCallRequest),
    ImageDelta(ImageDelta),
    Usage(TokenUsage),
    Finished(FinishReason),
}

#[derive(Debug, Clone)]
pub(crate) struct ImageDelta {
    pub index: usize,
    pub mime_type: String,
    pub partial_data: String,
    pub is_complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<InterStreamEvent>();
    }

    #[test]
    fn event_debug_format() {
        let event = InterStreamEvent::TextDelta("hello".into());
        let dbg = format!("{event:?}");
        assert!(dbg.contains("TextDelta"));
    }
}
