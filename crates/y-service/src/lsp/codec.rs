/// Errors while encoding or decoding LSP `Content-Length` frames.
#[derive(Debug, thiserror::Error)]
pub enum LspFrameError {
    #[error("invalid LSP header: {0}")]
    InvalidHeader(String),
    #[error("LSP frame is missing Content-Length")]
    MissingContentLength,
    #[error("LSP message length {actual} exceeds configured maximum {maximum}")]
    MessageTooLarge { actual: usize, maximum: usize },
    #[error("invalid LSP JSON payload: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Incremental decoder for LSP stdio framing.
pub struct LspFrameDecoder {
    buffer: Vec<u8>,
    max_message_bytes: usize,
}

impl LspFrameDecoder {
    pub fn new(max_message_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_message_bytes,
        }
    }

    pub fn encode(payload: &serde_json::Value) -> Result<Vec<u8>, LspFrameError> {
        let body = serde_json::to_vec(payload)?;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend(body);
        Ok(frame)
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<serde_json::Value>, LspFrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        loop {
            let Some(header_end) = find_header_end(&self.buffer) else {
                break;
            };
            let content_length = parse_content_length(&self.buffer[..header_end])?;
            if content_length > self.max_message_bytes {
                return Err(LspFrameError::MessageTooLarge {
                    actual: content_length,
                    maximum: self.max_message_bytes,
                });
            }
            let body_start = header_end + 4;
            let frame_end = body_start.saturating_add(content_length);
            if self.buffer.len() < frame_end {
                break;
            }
            let payload = serde_json::from_slice(&self.buffer[body_start..frame_end])?;
            messages.push(payload);
            self.buffer.drain(..frame_end);
        }
        Ok(messages)
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(header: &[u8]) -> Result<usize, LspFrameError> {
    let text = std::str::from_utf8(header)
        .map_err(|error| LspFrameError::InvalidHeader(error.to_string()))?;
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            return Err(LspFrameError::InvalidHeader(line.to_string()));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse()
                .map_err(|error: std::num::ParseIntError| {
                    LspFrameError::InvalidHeader(error.to_string())
                });
        }
    }
    Err(LspFrameError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::LspFrameDecoder;

    #[test]
    fn encodes_content_length_framing() {
        let payload = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});

        let encoded = LspFrameDecoder::encode(&payload).expect("encode");
        let text = String::from_utf8(encoded).expect("utf8");

        assert!(text.starts_with("Content-Length: "));
        assert!(text.contains("\r\n\r\n{\"id\":1,\"jsonrpc\":\"2.0\""));
    }

    #[test]
    fn decodes_fragmented_and_concatenated_frames() {
        let first = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        let second = serde_json::json!({"jsonrpc": "2.0", "method": "window/logMessage"});
        let mut bytes = LspFrameDecoder::encode(&first).expect("first frame");
        bytes.extend(LspFrameDecoder::encode(&second).expect("second frame"));
        let split = bytes.len() / 3;
        let mut decoder = LspFrameDecoder::new(1024 * 1024);

        let initial = decoder.push(&bytes[..split]).expect("partial frame");
        let completed = decoder.push(&bytes[split..]).expect("completed frames");

        assert!(initial.is_empty());
        assert_eq!(completed, vec![first, second]);
    }

    #[test]
    fn rejects_frames_over_the_configured_limit() {
        let mut decoder = LspFrameDecoder::new(8);
        let bytes = b"Content-Length: 20\r\n\r\n{}";

        let error = decoder.push(bytes).expect_err("oversized frame");

        assert!(error.to_string().contains("exceeds"));
    }
}
