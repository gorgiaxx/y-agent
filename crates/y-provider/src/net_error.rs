//! Diagnostics for transport-layer (reqwest) failures.
//!
//! A `reqwest::Error` carries far more than its top-level `Display`: an HTTP
//! status (when a response was received), a failure *kind* (timeout / connect /
//! body / decode / request), and a chain of underlying `source()` errors that
//! usually names the real cause — e.g. `connection reset by peer (os error 54)`
//! or `unexpected end of file`. The default `format!("{e}")` collapses all of
//! that into a single opaque line like `error decoding response body`.
//!
//! These helpers preserve that detail when converting into
//! [`ProviderError::NetworkError`], so a mid-stream drop reports *why* the
//! transport died instead of a generic decode message.

use std::error::Error as StdError;

use y_core::provider::ProviderError;

/// Render an error and its `source()` chain into a single `": "`-joined string.
///
/// Consecutive duplicate segments are collapsed, since outer reqwest/hyper
/// errors frequently restate their inner cause verbatim.
pub fn describe_error_chain(top: &dyn StdError) -> String {
    let mut parts: Vec<String> = vec![top.to_string()];
    let mut source = top.source();
    while let Some(err) = source {
        let text = err.to_string();
        if parts.last().is_none_or(|last| *last != text) {
            parts.push(text);
        }
        source = err.source();
    }
    parts.join(": ")
}

/// Describe a `reqwest::Error` for human/log consumption: a bracketed kind tag
/// (when the error self-classifies), followed by the full `source()` chain.
pub fn describe_reqwest_error(error: &reqwest::Error) -> String {
    let mut kinds: Vec<&str> = Vec::new();
    if error.is_timeout() {
        kinds.push("timeout");
    }
    if error.is_connect() {
        kinds.push("connect");
    }
    if error.is_body() {
        kinds.push("body");
    }
    if error.is_decode() {
        kinds.push("decode");
    }
    if error.is_request() {
        kinds.push("request");
    }

    let chain = describe_error_chain(error);
    if kinds.is_empty() {
        chain
    } else {
        format!("[{}] {chain}", kinds.join(","))
    }
}

/// Build a [`ProviderError::NetworkError`] from a `reqwest::Error`, preserving
/// the HTTP status (when a response was received) and the underlying cause.
///
/// Used for connect/send failures where no stream context exists; mid-stream
/// failures use the original response status instead (see
/// [`crate::sse::SseStreamState`]).
pub fn network_error_from_reqwest(error: &reqwest::Error) -> ProviderError {
    ProviderError::NetworkError {
        status: error.status().map(|s| s.as_u16()),
        message: describe_reqwest_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct TestError {
        msg: &'static str,
        source: Option<Box<TestError>>,
    }

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.msg)
        }
    }

    impl StdError for TestError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.source
                .as_deref()
                .map(|e| e as &(dyn StdError + 'static))
        }
    }

    #[test]
    fn chain_joins_nested_sources() {
        let err = TestError {
            msg: "error decoding response body",
            source: Some(Box::new(TestError {
                msg: "connection reset by peer (os error 54)",
                source: None,
            })),
        };
        assert_eq!(
            describe_error_chain(&err),
            "error decoding response body: connection reset by peer (os error 54)"
        );
    }

    #[test]
    fn chain_collapses_consecutive_duplicate_segments() {
        let err = TestError {
            msg: "operation timed out",
            source: Some(Box::new(TestError {
                msg: "operation timed out",
                source: None,
            })),
        };
        assert_eq!(describe_error_chain(&err), "operation timed out");
    }

    #[test]
    fn chain_of_single_error_is_just_its_message() {
        let err = TestError {
            msg: "unexpected end of file",
            source: None,
        };
        assert_eq!(describe_error_chain(&err), "unexpected end of file");
    }
}
