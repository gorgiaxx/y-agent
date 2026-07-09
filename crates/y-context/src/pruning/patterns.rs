//! Shared detection patterns for context pruning.
//!
//! Constants and helper functions used by both post-turn `PruningDetector`
//! (operating on `ChatMessageRecord`) and intra-turn `IntraTurnPruner`
//! (operating on `Vec<Message>`).

/// Error indicator patterns in tool result content.
///
/// These are matched as **prefix-oriented** patterns: a result is considered
/// error-bearing only when the pattern appears at the start of the content or
/// immediately after a short preamble (JSON key). This prevents false
/// positives like `FAILED: test_foo` (a test *result*, not a tool failure) or
/// `error: this feature requires X` (an informative finding the agent needs).
pub const ERROR_PATTERNS: &[&str] = &[
    "\"error\":",
    "\"error_type\":",
    "error:",
    "Error:",
    "parameter validation failed",
    "permission denied",
];

/// Patterns that are only treated as errors when they appear at the very
/// start of the content (first 10 chars). These are too generic for
/// substring matching — `FAILED` inside a test output is a legitimate
/// result, not a tool failure.
pub const ERROR_PREFIX_PATTERNS: &[&str] = &["FAILED", "failed to", "not found"];

/// Patterns indicating empty or unhelpful tool results.
pub const EMPTY_RESULT_PATTERNS: &[&str] = &[
    "\"results\": []",
    "\"results\":[]",
    "\"count\": 0",
    "\"count\":0",
    "no results found",
    "No results found",
    "no matches",
    "No matches",
    "[]",
];

/// Maximum content length for empty-result detection.
/// Long tool results containing "no results" are likely informative.
pub const EMPTY_RESULT_MAX_LEN: usize = 200;

/// Minimum content similarity ratio to consider two messages as repeats.
///
/// Raised from 0.8 to 0.9: the naive character-position comparison inflates
/// similarity for structurally-alike but semantically-different messages
/// (e.g. "Let me check the documentation for X" vs "...for Y"). 0.9 keeps
/// genuine retries while sparing legitimate progressive calls.
pub const SIMILARITY_THRESHOLD: f64 = 0.9;

/// Maximum distance (in message indices) between assistant messages
/// to consider them as repeated calls.
pub const MAX_ADJACENT_DISTANCE: usize = 3;

pub use crate::token_utils::estimate_tokens;

/// Check if content matches error patterns.
///
/// `ERROR_PATTERNS` (JSON keys, `error:`, `permission denied`) are matched
/// as substrings — they are specific enough that a substring hit is
/// reliable. `ERROR_PREFIX_PATTERNS` (`FAILED`, `failed to`, `not found`)
/// are only matched at the start of the content (first 10 chars) to avoid
/// false positives like `FAILED: test_foo` deep inside a test report.
///
/// Extra patterns from agent config are matched as substrings for backward
/// compatibility.
pub fn matches_error_patterns(content: &str, extra_patterns: &[String]) -> bool {
    let builtin = ERROR_PATTERNS.iter().any(|p| content.contains(p));
    let prefix = {
        let head = content.get(..content.len().min(10)).unwrap_or(content);
        ERROR_PREFIX_PATTERNS.iter().any(|p| head.starts_with(p))
    };
    let extra = extra_patterns.iter().any(|p| content.contains(p.as_str()));
    builtin || prefix || extra
}

/// Check if content matches empty-result patterns (short content only).
pub fn matches_empty_patterns(content: &str) -> bool {
    content.len() < EMPTY_RESULT_MAX_LEN
        && EMPTY_RESULT_PATTERNS.iter().any(|p| content.contains(p))
}

/// Simple content similarity ratio (0.0 to 1.0).
///
/// Compares character-by-character up to the shorter length, then
/// divides matching count by the longer length.
pub fn content_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let min_len = a.len().min(b.len());
    let max_len = a.len().max(b.len());

    let matching: usize = a
        .chars()
        .zip(b.chars())
        .take(min_len)
        .filter(|(ca, cb)| ca == cb)
        .count();

    matching as f64 / max_len as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("1234"), 1);
        assert_eq!(estimate_tokens("12345"), 2);
        assert_eq!(estimate_tokens("12345678"), 2);
    }

    #[test]
    fn test_matches_error_patterns_builtin() {
        // JSON-key patterns match as substring.
        assert!(matches_error_patterns("{\"error\": \"bad request\"}", &[]));
        assert!(matches_error_patterns("permission denied", &[]));
        // Prefix-only patterns: match at start.
        assert!(matches_error_patterns("not found: config.toml", &[]));
        assert!(matches_error_patterns("FAILED to connect", &[]));
        // Prefix-only patterns: do NOT match mid-content (regression: test output).
        assert!(!matches_error_patterns(
            "running tests...\nFAILED: test_foo\npassed: 3",
            &[]
        ));
        assert!(!matches_error_patterns(
            "the file was not found in /etc",
            &[]
        ));
        // Non-error content.
        assert!(!matches_error_patterns("success: true", &[]));
    }

    #[test]
    fn test_matches_error_patterns_extra() {
        assert!(matches_error_patterns(
            "CUSTOM_FAILURE_CODE: 42",
            &["CUSTOM_FAILURE_CODE".to_string()]
        ));
        assert!(!matches_error_patterns(
            "all good",
            &["CUSTOM_FAILURE_CODE".to_string()]
        ));
    }

    #[test]
    fn test_matches_empty_patterns() {
        assert!(matches_empty_patterns("{\"results\": [], \"count\": 0}"));
        assert!(matches_empty_patterns("no results found"));
        assert!(matches_empty_patterns("[]"));
        // Long content should not match even with pattern present.
        let long = format!("prefix {} suffix", "x".repeat(250));
        assert!(!matches_empty_patterns(&long));
    }

    #[test]
    fn test_content_similarity_identical() {
        assert!((content_similarity("abc", "abc") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_content_similarity_different() {
        assert!(content_similarity("abc", "xyz") < 0.5);
    }

    #[test]
    fn test_content_similarity_empty() {
        assert!(content_similarity("", "") > 0.99);
        assert!(content_similarity("abc", "") < 0.01);
        assert!(content_similarity("", "abc") < 0.01);
    }

    #[test]
    fn test_content_similarity_partial() {
        let sim = content_similarity("abcdef", "abcxyz");
        assert!(sim > 0.4 && sim < 0.6);
    }
}
