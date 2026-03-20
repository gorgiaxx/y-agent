//! Shared detection patterns for context pruning.
//!
//! Constants and helper functions used by both post-turn `PruningDetector`
//! (operating on `ChatMessageRecord`) and intra-turn `IntraTurnPruner`
//! (operating on `Vec<Message>`).

/// Error indicator patterns in tool result content.
pub const ERROR_PATTERNS: &[&str] = &[
    "\"error\":",
    "\"error_type\":",
    "error:",
    "Error:",
    "FAILED",
    "failed to",
    "parameter validation failed",
    "not found",
    "permission denied",
];

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
pub const SIMILARITY_THRESHOLD: f64 = 0.8;

/// Maximum distance (in message indices) between assistant messages
/// to consider them as repeated calls.
pub const MAX_ADJACENT_DISTANCE: usize = 3;

/// Simple token estimation (4 chars per token).
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

/// Check if content matches any built-in or extra error patterns.
pub fn matches_error_patterns(content: &str, extra_patterns: &[String]) -> bool {
    let builtin = ERROR_PATTERNS.iter().any(|p| content.contains(p));
    let extra = extra_patterns.iter().any(|p| content.contains(p.as_str()));
    builtin || extra
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
        assert!(matches_error_patterns("{\"error\": \"bad request\"}", &[]));
        assert!(matches_error_patterns("FAILED to connect", &[]));
        assert!(matches_error_patterns("permission denied", &[]));
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
