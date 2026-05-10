//! Converts an internal `InterStreamEvent` stream into the public `ChatStream`.

use std::pin::Pin;

use futures::{Stream, StreamExt as _};

use y_core::provider::{ChatStream, ChatStreamChunk, ImageContentDelta, ProviderError};

use crate::inter_stream::{ImageDelta, InterStreamEvent};

pub(crate) fn into_chat_stream(
    inner: Pin<Box<dyn Stream<Item = Result<InterStreamEvent, ProviderError>> + Send>>,
) -> ChatStream {
    Box::pin(inner.map(|result| result.map(map_event)))
}

fn map_event(event: InterStreamEvent) -> ChatStreamChunk {
    match event {
        InterStreamEvent::TextDelta(text) => ChatStreamChunk {
            delta_content: Some(text),
            ..Default::default()
        },
        InterStreamEvent::ReasoningDelta(text) => ChatStreamChunk {
            delta_reasoning_content: Some(text),
            ..Default::default()
        },
        InterStreamEvent::ToolCall(tc) => ChatStreamChunk {
            delta_tool_calls: vec![tc],
            ..Default::default()
        },
        InterStreamEvent::ImageDelta(ImageDelta {
            index,
            mime_type,
            partial_data,
            is_complete,
        }) => ChatStreamChunk {
            delta_images: vec![ImageContentDelta {
                index,
                mime_type,
                partial_data,
                is_complete,
            }],
            ..Default::default()
        },
        InterStreamEvent::Usage(usage) => ChatStreamChunk {
            usage: Some(usage),
            ..Default::default()
        },
        InterStreamEvent::Finished(reason) => ChatStreamChunk {
            finish_reason: Some(reason),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use y_core::types::TokenUsage;

    #[tokio::test]
    async fn text_delta_maps_to_chunk() {
        let events = vec![Ok(InterStreamEvent::TextDelta("hello".into()))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta_content.as_deref(), Some("hello"));
        assert!(chunk.delta_reasoning_content.is_none());
        assert!(chunk.delta_tool_calls.is_empty());
        assert!(chunk.usage.is_none());
        assert!(chunk.finish_reason.is_none());
    }

    #[tokio::test]
    async fn reasoning_delta_maps_to_chunk() {
        let events = vec![Ok(InterStreamEvent::ReasoningDelta("think".into()))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        assert!(chunk.delta_content.is_none());
        assert_eq!(chunk.delta_reasoning_content.as_deref(), Some("think"));
    }

    #[tokio::test]
    async fn tool_call_maps_to_chunk() {
        use y_core::types::ToolCallRequest;

        let tc = ToolCallRequest {
            id: "call_1".into(),
            name: "search".into(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let events = vec![Ok(InterStreamEvent::ToolCall(tc.clone()))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta_tool_calls.len(), 1);
        assert_eq!(chunk.delta_tool_calls[0].id, "call_1");
    }

    #[tokio::test]
    async fn usage_maps_to_chunk() {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 20,
            ..Default::default()
        };
        let events = vec![Ok(InterStreamEvent::Usage(usage))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        let u = chunk.usage.unwrap();
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 20);
    }

    #[tokio::test]
    async fn finished_maps_to_chunk() {
        use y_core::provider::FinishReason;

        let events = vec![Ok(InterStreamEvent::Finished(FinishReason::Stop))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.finish_reason, Some(FinishReason::Stop));
    }

    #[tokio::test]
    async fn image_delta_maps_to_chunk() {
        let img = crate::inter_stream::ImageDelta {
            index: 0,
            mime_type: "image/png".into(),
            partial_data: "base64data".into(),
            is_complete: true,
        };
        let events = vec![Ok(InterStreamEvent::ImageDelta(img))];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.delta_images.len(), 1);
        assert_eq!(chunk.delta_images[0].mime_type, "image/png");
        assert!(chunk.delta_images[0].is_complete);
    }

    #[tokio::test]
    async fn error_propagated() {
        let events: Vec<Result<InterStreamEvent, ProviderError>> =
            vec![Err(ProviderError::NetworkError {
                message: "timeout".into(),
            })];
        let mut stream = into_chat_stream(Box::pin(stream::iter(events)));

        let result = stream.next().await.unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn multi_event_sequence() {
        use y_core::provider::FinishReason;

        let events = vec![
            Ok(InterStreamEvent::TextDelta("he".into())),
            Ok(InterStreamEvent::TextDelta("llo".into())),
            Ok(InterStreamEvent::Usage(TokenUsage {
                input_tokens: 5,
                output_tokens: 2,
                ..Default::default()
            })),
            Ok(InterStreamEvent::Finished(FinishReason::Stop)),
        ];
        let stream = into_chat_stream(Box::pin(stream::iter(events)));
        let chunks: Vec<_> = stream.collect().await;

        assert_eq!(chunks.len(), 4);
        assert_eq!(
            chunks[0].as_ref().unwrap().delta_content.as_deref(),
            Some("he")
        );
        assert_eq!(
            chunks[1].as_ref().unwrap().delta_content.as_deref(),
            Some("llo")
        );
        assert!(chunks[2].as_ref().unwrap().usage.is_some());
        assert_eq!(
            chunks[3].as_ref().unwrap().finish_reason,
            Some(FinishReason::Stop)
        );
    }
}
