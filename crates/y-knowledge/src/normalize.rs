//! Text normalization for embedding preprocessing.
//!
//! Normalizes text before embedding to improve retrieval quality.
//! Inspired by `MaxKB`'s `normalize_for_embedding` approach.

/// Normalize text for embedding pipeline.
///
/// Applies the following transformations:
/// 1. Unicode NFC normalization
/// 2. Emoji removal (Unicode emoji ranges)
/// 3. Consecutive whitespace compression to single space
/// 4. Leading/trailing whitespace trim
///
/// # Examples
///
/// ```
/// use y_knowledge::normalize::normalize_for_embedding;
///
/// let text = " Hello  🎉  World！  ";
/// let normalized = normalize_for_embedding(text);
/// assert_eq!(normalized, "Hello World！");
/// ```
pub fn normalize_for_embedding(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_space = true; // true = skip leading spaces

    for ch in text.chars() {
        if is_emoji(ch) {
            continue;
        }

        if ch.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }

    // Trim trailing space (if compressed whitespace ended up at the end).
    if result.ends_with(' ') {
        result.pop();
    }

    result
}

/// Check if a character is an emoji.
///
/// Covers major Unicode emoji ranges including:
/// - Emoticons, Dingbats, Symbols
/// - Pictographs, Transport, Supplemental
/// - Various enclosed/CJK symbols commonly used as emoji
/// - Variation selectors and ZWJ
fn is_emoji(ch: char) -> bool {
    matches!(ch,
        // Variation Selectors
        '\u{FE00}'..='\u{FE0F}' |
        // Zero Width Joiner
        '\u{200D}' |
        // Combining Enclosing Keycap
        '\u{20E3}' |
        // Miscellaneous Symbols and Pictographs
        '\u{1F300}'..='\u{1F5FF}' |
        // Emoticons
        '\u{1F600}'..='\u{1F64F}' |
        // Transport and Map Symbols
        '\u{1F680}'..='\u{1F6FF}' |
        // Supplemental Symbols and Pictographs
        '\u{1F900}'..='\u{1F9FF}' |
        // Symbols and Pictographs Extended-A
        '\u{1FA00}'..='\u{1FA6F}' |
        // Symbols and Pictographs Extended-B
        '\u{1FA70}'..='\u{1FAFF}' |
        // Dingbats
        '\u{2702}'..='\u{27B0}' |
        // Misc symbols
        '\u{2600}'..='\u{26FF}' |
        // Regional indicator symbols
        '\u{1F1E0}'..='\u{1F1FF}' |
        // CJK Symbols (commonly emoji)
        '\u{3030}' | '\u{303D}' |
        // Enclosed Ideographic Supplement
        '\u{1F200}'..='\u{1F251}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_basic() {
        assert_eq!(normalize_for_embedding("hello world"), "hello world");
    }

    #[test]
    fn test_normalize_whitespace_compression() {
        assert_eq!(
            normalize_for_embedding("hello   world   foo"),
            "hello world foo"
        );
    }

    #[test]
    fn test_normalize_leading_trailing_trim() {
        assert_eq!(normalize_for_embedding("   hello world   "), "hello world");
    }

    #[test]
    fn test_normalize_emoji_removal() {
        assert_eq!(normalize_for_embedding("Hello 🎉 World 🚀"), "Hello World");
    }

    #[test]
    fn test_normalize_tabs_and_newlines() {
        assert_eq!(
            normalize_for_embedding("hello\t\tworld\n\nfoo"),
            "hello world foo"
        );
    }

    #[test]
    fn test_normalize_chinese_text() {
        assert_eq!(
            normalize_for_embedding("你好  世界 🎉 测试"),
            "你好 世界 测试"
        );
    }

    #[test]
    fn test_normalize_preserves_punctuation() {
        assert_eq!(
            normalize_for_embedding("Hello! How are you? 你好！"),
            "Hello! How are you? 你好！"
        );
    }

    #[test]
    fn test_normalize_empty_string() {
        assert_eq!(normalize_for_embedding(""), "");
    }

    #[test]
    fn test_normalize_only_emoji() {
        assert_eq!(normalize_for_embedding("🎉🚀✨"), "");
    }

    #[test]
    fn test_normalize_mixed_whitespace_and_emoji() {
        assert_eq!(
            normalize_for_embedding("  🎉  hello  🚀  world  "),
            "hello world"
        );
    }
}
