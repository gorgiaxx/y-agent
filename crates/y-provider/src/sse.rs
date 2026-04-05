//! Shared SSE (Server-Sent Events) stream parsing infrastructure.
//!
//! Provides a reusable byte-stream-to-text decoder and SSE event extractor
//! used by all streaming LLM provider implementations. This module
//! eliminates ~150 lines of duplicated UTF-8 decoding and event boundary
//! detection logic that was previously copied across 5 provider files.

use std::pin::Pin;

use bytes::Bytes;
use futures::Stream;

use y_core::provider::ProviderError;

// ---------------------------------------------------------------------------
// Byte stream type alias
// ---------------------------------------------------------------------------

/// A pinned, boxed byte stream from an HTTP response.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

// ---------------------------------------------------------------------------
// SSE stream state
// ---------------------------------------------------------------------------

/// Shared state for SSE stream parsing.
///
/// Handles incremental UTF-8 decoding from a raw byte stream into a text
/// buffer. Provider-specific state (e.g. tool call accumulators) should be
/// stored alongside this struct, not inside it.
pub struct SseStreamState {
    /// Raw byte stream from the HTTP response.
    pub byte_stream: ByteStream,
    /// Accumulated text buffer for SSE event parsing.
    pub buffer: String,
    /// Leftover bytes from the previous chunk that form an incomplete UTF-8 sequence.
    pub bytes_remainder: Vec<u8>,
    /// Whether the stream has been fully consumed.
    pub done: bool,
}

impl SseStreamState {
    /// Create a new SSE stream state from an HTTP byte stream.
    pub fn new(byte_stream: ByteStream) -> Self {
        Self {
            byte_stream,
            buffer: String::new(),
            bytes_remainder: Vec::new(),
            done: false,
        }
    }

    /// Read the next chunk of bytes from the stream and append decoded UTF-8
    /// text to the internal buffer.
    ///
    /// Returns:
    /// - `Ok(true)` if data was successfully read
    /// - `Ok(false)` if the stream has ended (no more data)
    /// - `Err(ProviderError)` on network errors
    pub async fn read_next(&mut self) -> Result<bool, ProviderError> {
        use futures::StreamExt as _;

        match self.byte_stream.next().await {
            Some(Ok(bytes)) => {
                self.decode_bytes(&bytes);
                Ok(true)
            }
            Some(Err(e)) => {
                self.done = true;
                Err(ProviderError::NetworkError {
                    message: format!("stream read error: {e}"),
                })
            }
            None => {
                self.done = true;
                Ok(false)
            }
        }
    }

    /// Decode raw bytes into the text buffer, handling incomplete UTF-8
    /// sequences at chunk boundaries.
    fn decode_bytes(&mut self, bytes: &[u8]) {
        let combined = if self.bytes_remainder.is_empty() {
            bytes.to_vec()
        } else {
            let mut combined = std::mem::take(&mut self.bytes_remainder);
            combined.extend_from_slice(bytes);
            combined
        };

        match std::str::from_utf8(&combined) {
            Ok(text) => self.buffer.push_str(text),
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to > 0 {
                    // valid_up_to is guaranteed to be a valid UTF-8 boundary,
                    // so this unwrap is infallible.
                    let valid_text = std::str::from_utf8(&combined[..valid_up_to])
                        .expect("valid_up_to guarantees valid UTF-8");
                    self.buffer.push_str(valid_text);
                }
                // Keep the remaining bytes for the next chunk.
                self.bytes_remainder = combined[valid_up_to..].to_vec();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SSE event extraction
// ---------------------------------------------------------------------------

/// Extract one SSE event `data:` payload from the buffer.
///
/// SSE events are separated by double newlines (`\n\n` or `\r\n\r\n`).
/// Each event may contain multiple `data:` lines which are joined with `\n`.
/// Non-data fields (`event:`, `id:`, `retry:`) are ignored.
///
/// Returns `None` if no complete event is available yet.
/// Returns `Some("")` for events with no `data:` lines (e.g. comments).
///
/// Used by `OpenAI`, Azure, Gemini, and compatible providers.
pub fn extract_sse_data(buffer: &mut String) -> Option<String> {
    let boundary = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))?;

    let raw_event: String = buffer.drain(..boundary).collect();
    // Consume the boundary newlines.
    while buffer.starts_with('\n') || buffer.starts_with('\r') {
        buffer.remove(0);
    }

    let mut data_parts = Vec::new();
    for line in raw_event.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            data_parts.push(data.trim().to_string());
        }
    }

    if data_parts.is_empty() {
        return Some(String::new());
    }

    Some(data_parts.join("\n"))
}

/// Extract one newline-delimited JSON line from the buffer.
///
/// Used by Ollama which sends one JSON object per line (NDJSON format).
pub fn extract_json_line(buffer: &mut String) -> Option<String> {
    let newline_pos = buffer.find('\n')?;
    let line: String = buffer.drain(..=newline_pos).collect();
    Some(line)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_sse_data_simple() {
        let mut buf = "data: {\"hello\":\"world\"}\n\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "{\"hello\":\"world\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_sse_data_done_signal() {
        let mut buf = "data: [DONE]\n\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "[DONE]");
    }

    #[test]
    fn extract_sse_data_incomplete() {
        let mut buf = "data: {\"partial\":".to_string();
        assert!(extract_sse_data(&mut buf).is_none());
        assert_eq!(buf, "data: {\"partial\":");
    }

    #[test]
    fn extract_sse_data_multiple_events() {
        let mut buf = "data: first\n\ndata: second\n\n".to_string();
        let e1 = extract_sse_data(&mut buf).unwrap();
        assert_eq!(e1, "first");
        let e2 = extract_sse_data(&mut buf).unwrap();
        assert_eq!(e2, "second");
    }

    #[test]
    fn extract_sse_data_with_event_type() {
        let mut buf = "event: content_block_delta\ndata: {\"type\":\"delta\"}\n\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "{\"type\":\"delta\"}");
    }

    #[test]
    fn extract_sse_data_no_data_lines() {
        let mut buf = "event: ping\n\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "");
    }

    #[test]
    fn extract_sse_data_crlf_boundary() {
        let mut buf = "data: {\"ok\":true}\r\n\r\n".to_string();
        let event = extract_sse_data(&mut buf).unwrap();
        assert_eq!(event, "{\"ok\":true}");
    }

    #[test]
    fn extract_json_line_simple() {
        let mut buf = "{\"done\":false,\"message\":\"hi\"}\n".to_string();
        let line = extract_json_line(&mut buf).unwrap();
        assert!(line.contains("\"done\":false"));
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_json_line_incomplete() {
        let mut buf = "{\"partial\":true".to_string();
        assert!(extract_json_line(&mut buf).is_none());
    }

    #[test]
    fn decode_bytes_handles_split_utf8() {
        let mut state = SseStreamState::new(Box::pin(futures::stream::empty()));

        // "e" with acute accent = 0xC3 0xA9 in UTF-8
        // Simulate splitting the multi-byte sequence across chunks
        state.decode_bytes(&[b'h', b'i', 0xC3]);
        assert_eq!(state.buffer, "hi");
        assert_eq!(state.bytes_remainder, vec![0xC3]);

        state.decode_bytes(&[0xA9, b'!']);
        assert_eq!(state.buffer, "hi\u{00e9}!");
        assert!(state.bytes_remainder.is_empty());
    }
}
