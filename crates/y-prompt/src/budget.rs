//! Token budget enforcement for prompt sections.

/// Estimate the number of tokens in a text string.
///
/// Uses the heuristic: 1 token per 4 characters.
/// This is configurable but the default provides a reasonable estimate
/// that is within 10% for English text.
pub fn estimate_tokens(text: &str) -> u32 {
    let tokens = text.chars().count().div_ceil(4);
    u32::try_from(tokens).unwrap_or(u32::MAX)
}

/// Truncate text to fit within a token budget.
///
/// Returns the (possibly truncated) text and whether truncation occurred.
/// If truncated, appends a `[truncated]` marker.
pub fn truncate_to_budget(text: &str, max_tokens: u32) -> (String, bool) {
    let current = estimate_tokens(text);
    if current <= max_tokens {
        return (text.to_string(), false);
    }

    let marker = "\n[truncated]";
    let char_limit = usize::try_from(max_tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(4);
    let marker_chars = marker.chars().count();

    if char_limit == 0 || char_limit < marker_chars {
        return (String::new(), true);
    }

    let effective_limit = char_limit.saturating_sub(marker_chars);

    // Truncate at a char boundary.
    let text_chars = text.chars().count();
    let truncated = if effective_limit >= text_chars {
        text.to_string()
    } else {
        // Convert the retained character count back into a UTF-8 byte boundary.
        let boundary = text
            .char_indices()
            .nth(effective_limit)
            .map_or(text.len(), |(i, _)| i);
        format!("{}{marker}", &text[..boundary])
    };

    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_simple() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1); // 1 char → ceil(1/4) = 1
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars → 1 token
        assert_eq!(estimate_tokens("abcde"), 2); // 5 chars → ceil(5/4) = 2
    }

    #[test]
    fn test_estimate_tokens_longer() {
        // 100 chars → 25 tokens.
        let text = "a".repeat(100);
        assert_eq!(estimate_tokens(&text), 25);
    }

    #[test]
    fn test_estimate_tokens_counts_multibyte_as_characters() {
        assert_eq!(estimate_tokens("你好你好"), 1); // 4 chars, not 12 UTF-8 bytes
        assert_eq!(estimate_tokens("你好你好你"), 2); // 5 chars
    }

    #[test]
    fn test_truncate_within_budget() {
        let text = "Hello, world!"; // 13 chars → ~4 tokens.
        let (result, truncated) = truncate_to_budget(text, 10);
        assert_eq!(result, text);
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_exceeds_budget() {
        let text = "a".repeat(200); // 200 chars → 50 tokens.
        let (result, truncated) = truncate_to_budget(&text, 10);
        assert!(truncated);
        assert!(result.ends_with("[truncated]"));
        // Result should be within ~10 tokens worth of characters + marker.
        assert!(result.len() < 60);
    }

    #[test]
    fn test_truncate_zero_budget() {
        let text = "Some content here.";
        let (result, truncated) = truncate_to_budget(text, 0);
        assert!(truncated);
        assert!(result.is_empty());
    }

    #[test]
    fn test_truncate_result_stays_within_budget() {
        let text = "a".repeat(200); // 50 tokens.
        let (result, truncated) = truncate_to_budget(&text, 2);
        assert!(truncated);
        assert!(estimate_tokens(&result) <= 2);
    }

    #[test]
    fn test_truncate_multibyte_text_stays_within_budget() {
        let text = "你好".repeat(100);
        let (result, truncated) = truncate_to_budget(&text, 10);
        assert!(truncated);
        assert!(estimate_tokens(&result) <= 10);
    }
}
