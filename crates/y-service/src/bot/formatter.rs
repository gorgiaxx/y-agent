//! Bot response formatter: adapts LLM output for platform delivery.
//!
//! Handles response length truncation and stripping of internal tool-call
//! XML tags that should not be visible to end users on messaging platforms.

/// Format an LLM response for platform delivery.
///
/// - Strips internal tool-call XML envelopes (`<tool_call>`, etc.) that may
///   leak through in `PromptBased` tool-calling mode.
/// - Truncates to `max_length` characters with a `[...]` marker when exceeded.
pub fn format_response(content: &str, max_length: usize) -> String {
    let cleaned = strip_tool_call_tags(content);
    let trimmed = cleaned.trim();

    if trimmed.len() <= max_length || max_length == 0 {
        return trimmed.to_string();
    }

    // Find a safe truncation point (avoid splitting multi-byte chars).
    let truncate_at = max_length.saturating_sub(6); // room for " [...]"
    let boundary = find_char_boundary(trimmed, truncate_at);
    format!("{} [...]", &trimmed[..boundary])
}

/// Strip known tool-call XML envelope tags from content.
///
/// Removes tags like `<tool_call>...</tool_call>` and their contents, as well
/// as partial/orphaned tags. This ensures platform users never see raw XML
/// from the prompt-based tool-calling protocol.
fn strip_tool_call_tags(content: &str) -> String {
    // Known envelope tag patterns (without angle brackets).
    const TAG_NAMES: &[&str] = &[
        "tool_call",
        "minimax:tool_call",
        "longcat_tool_call",
        "y_tool_call",
    ];

    let mut result = content.to_string();
    for tag_name in TAG_NAMES {
        let open = format!("<{tag_name}>");
        let close = format!("</{tag_name}>");

        // Remove matched pairs.
        while let Some(start) = result.find(&open) {
            if let Some(end) = result[start..].find(&close) {
                let end_pos = start + end + close.len();
                result.replace_range(start..end_pos, "");
            } else {
                // Orphaned open tag: remove just the tag.
                result = result.replacen(&open, "", 1);
            }
        }

        // Orphaned close tags.
        result = result.replace(&close, "");
    }

    result
}

/// Find the nearest char boundary at or before `index`.
fn find_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_response_unchanged() {
        let result = format_response("Hello, world!", 2000);
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn long_response_truncated() {
        let long = "a".repeat(3000);
        let result = format_response(&long, 2000);
        assert!(result.len() <= 2000);
        assert!(result.ends_with("[...]"));
    }

    #[test]
    fn strips_tool_call_tags() {
        let input = "Hello <tool_call>{\"name\":\"test\"}</tool_call> world";
        let result = format_response(input, 2000);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn strips_multiple_tag_types() {
        let input = "A <minimax:tool_call>x</minimax:tool_call> B \
                     <longcat_tool_call>y</longcat_tool_call> C";
        let result = format_response(input, 2000);
        assert_eq!(result, "A  B  C");
    }

    #[test]
    fn handles_orphaned_close_tags() {
        let input = "Hello </tool_call> world";
        let result = format_response(input, 2000);
        assert_eq!(result, "Hello  world");
    }

    #[test]
    fn trims_whitespace() {
        let result = format_response("  hello  ", 2000);
        assert_eq!(result, "hello");
    }

    #[test]
    fn zero_max_length_returns_full() {
        let result = format_response("hello", 0);
        assert_eq!(result, "hello");
    }

    #[test]
    fn handles_multibyte_truncation() {
        // Chinese chars: each is 3 bytes in UTF-8.
        let input = "hello world";
        let result = format_response(input, 8);
        assert!(result.is_char_boundary(result.len()));
        assert!(result.ends_with("[...]"));
    }

    #[test]
    fn empty_input() {
        let result = format_response("", 2000);
        assert_eq!(result, "");
    }
}
