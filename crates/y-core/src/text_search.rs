//! Shared deterministic text-search normalization.
/// Split a natural-language query into sorted, unique lexical search tokens.
pub fn lexical_query_tokens(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "and", "are", "for", "from", "into", "please", "that", "the", "these", "this", "with",
    ];
    let mut tokens: Vec<_> = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(&token.as_str()))
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::lexical_query_tokens;

    #[test]
    fn lexical_tokens_remove_stop_words_and_duplicates() {
        assert_eq!(
            lexical_query_tokens("Please review these Rust rust-errors"),
            ["errors", "review", "rust"]
        );
    }
}
