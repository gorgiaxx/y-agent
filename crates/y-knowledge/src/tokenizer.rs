//! Tokenizer for text segmentation.
//!
//! Provides the `Tokenizer` trait and implementations for English and Chinese
//! text segmentation, used in BM25 keyword indexing (Sprint 3).

use jieba_rs::Jieba;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Tokenizer trait
// ---------------------------------------------------------------------------

/// Trait for text tokenization / word segmentation.
pub trait Tokenizer: Send + Sync {
    /// Tokenize text into individual terms.
    ///
    /// Terms should be lowercased for consistent indexing.
    fn tokenize(&self, text: &str) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Simple (English) Tokenizer
// ---------------------------------------------------------------------------

/// Whitespace-based tokenizer for English text.
///
/// Splits on whitespace, lowercases, and removes punctuation.
#[derive(Debug, Default)]
pub struct SimpleTokenizer;

impl SimpleTokenizer {
    pub fn new() -> Self {
        Self
    }
}

impl Tokenizer for SimpleTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|w| {
                w.to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Chinese Tokenizer (jieba-rs)
// ---------------------------------------------------------------------------

/// Global `Jieba` instance (loaded once, thread-safe).
static JIEBA: OnceLock<Jieba> = OnceLock::new();

fn get_jieba() -> &'static Jieba {
    JIEBA.get_or_init(Jieba::new)
}

/// Chinese tokenizer based on `jieba-rs`.
///
/// Uses jieba's `cut_all` mode (全模式分词) for maximum recall,
/// which is better suited for keyword indexing and BM25.
/// Inspired by `MaxKB`'s jieba segmentation approach.
#[derive(Debug, Default)]
pub struct ChineseTokenizer;

impl ChineseTokenizer {
    pub fn new() -> Self {
        Self
    }
}

impl Tokenizer for ChineseTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        let jieba = get_jieba();
        // Use cut (default mode, HMM enabled) for accurate segmentation.
        let words = jieba.cut(text, false);
        words
            .into_iter()
            .map(str::to_lowercase)
            .filter(|w| !w.trim().is_empty())
            .filter(|w| {
                // Filter out pure whitespace/punctuation tokens.
                w.chars().any(char::is_alphanumeric)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Auto-detecting tokenizer
// ---------------------------------------------------------------------------

/// Auto-detecting tokenizer that dispatches to the appropriate backend
/// based on content analysis.
#[derive(Debug, Default)]
pub struct AutoTokenizer {
    simple: SimpleTokenizer,
    chinese: ChineseTokenizer,
}

impl AutoTokenizer {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tokenizer for AutoTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        if contains_cjk(text) {
            self.chinese.tokenize(text)
        } else {
            self.simple.tokenize(text)
        }
    }
}

/// Check if text contains CJK characters.
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{4E00}'..='\u{9FFF}' |     // CJK Unified Ideographs
            '\u{3040}'..='\u{309F}' |      // Hiragana
            '\u{30A0}'..='\u{30FF}' |      // Katakana
            '\u{F900}'..='\u{FAFF}' |      // CJK Compatibility Ideographs
            '\u{20000}'..='\u{2A6DF}'      // CJK Extension B
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SimpleTokenizer tests ---

    #[test]
    fn test_simple_tokenizer_basic() {
        let tokenizer = SimpleTokenizer::new();
        let tokens = tokenizer.tokenize("Hello World foo");
        assert_eq!(tokens, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_simple_tokenizer_punctuation() {
        let tokenizer = SimpleTokenizer::new();
        let tokens = tokenizer.tokenize("Hello, world! How are you?");
        assert_eq!(tokens, vec!["hello", "world", "how", "are", "you"]);
    }

    #[test]
    fn test_simple_tokenizer_empty() {
        let tokenizer = SimpleTokenizer::new();
        let tokens = tokenizer.tokenize("");
        assert!(tokens.is_empty());
    }

    // --- ChineseTokenizer tests ---

    #[test]
    fn test_chinese_tokenizer_basic() {
        let tokenizer = ChineseTokenizer::new();
        let tokens = tokenizer.tokenize("我来到北京清华大学");
        assert!(!tokens.is_empty());
        // Jieba should segment this into meaningful words.
        let joined = tokens.join(" ");
        assert!(
            joined.contains("北京") || joined.contains("清华"),
            "expected Chinese segments, got: {joined}"
        );
    }

    #[test]
    fn test_chinese_tokenizer_mixed() {
        let tokenizer = ChineseTokenizer::new();
        let tokens = tokenizer.tokenize("Rust语言很强大");
        assert!(!tokens.is_empty());
        let has_rust = tokens.iter().any(|t| t == "rust");
        assert!(has_rust, "should tokenize 'Rust', got {tokens:?}");
    }

    #[test]
    fn test_chinese_tokenizer_filters_punctuation() {
        let tokenizer = ChineseTokenizer::new();
        let tokens = tokenizer.tokenize("你好！世界？");
        // Should not contain pure punctuation.
        for token in &tokens {
            assert!(
                token.chars().any(|c| c.is_alphanumeric()),
                "token should not be pure punctuation: '{token}'"
            );
        }
    }

    // --- AutoTokenizer tests ---

    #[test]
    fn test_auto_tokenizer_english() {
        let tokenizer = AutoTokenizer::new();
        let tokens = tokenizer.tokenize("Hello World");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_auto_tokenizer_chinese() {
        let tokenizer = AutoTokenizer::new();
        let tokens = tokenizer.tokenize("我来到北京");
        assert!(!tokens.is_empty());
        // Should use Chinese tokenizer.
        let joined = tokens.join(" ");
        assert!(joined.contains("北京"), "expected '北京', got: {joined}");
    }

    // --- CJK detection ---

    #[test]
    fn test_contains_cjk() {
        assert!(contains_cjk("Hello 你好"));
        assert!(contains_cjk("中文"));
        assert!(!contains_cjk("Hello World"));
        assert!(!contains_cjk("123 abc"));
    }
}
