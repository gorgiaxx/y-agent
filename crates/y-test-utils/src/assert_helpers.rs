//! Custom assertion helpers for y-agent tests.

/// Assert that two `serde_json::Value` instances are equal, with a readable
/// diff on failure.
#[macro_export]
macro_rules! assert_json_eq {
    ($left:expr, $right:expr) => {
        let left = &$left;
        let right = &$right;
        if left != right {
            panic!(
                "JSON values differ:\n  left:  {}\n  right: {}",
                serde_json::to_string_pretty(left).unwrap(),
                serde_json::to_string_pretty(right).unwrap(),
            );
        }
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        let left = &$left;
        let right = &$right;
        if left != right {
            panic!(
                "{}\n  left:  {}\n  right: {}",
                format_args!($($arg)+),
                serde_json::to_string_pretty(left).unwrap(),
                serde_json::to_string_pretty(right).unwrap(),
            );
        }
    };
}

/// Assert that a duration (in milliseconds) is within a given bound.
#[macro_export]
macro_rules! assert_within_ms {
    ($actual_ms:expr, $max_ms:expr) => {
        let actual = $actual_ms;
        let max = $max_ms;
        assert!(
            actual <= max,
            "Duration {actual}ms exceeded maximum {max}ms"
        );
    };
}

/// Assert that token count is within a given budget.
#[macro_export]
macro_rules! assert_tokens_within {
    ($usage:expr, $budget:expr) => {
        let total = $usage.total();
        let budget = $budget;
        assert!(
            total <= budget,
            "Token usage {total} exceeded budget {budget}"
        );
    };
}

#[cfg(test)]
mod tests {
    use y_core::types::TokenUsage;

    #[test]
    fn test_assert_json_eq() {
        let a = serde_json::json!({"key": "value"});
        let b = serde_json::json!({"key": "value"});
        assert_json_eq!(a, b);
    }

    #[test]
    #[should_panic(expected = "JSON values differ")]
    fn test_assert_json_eq_fails() {
        let a = serde_json::json!({"key": "a"});
        let b = serde_json::json!({"key": "b"});
        assert_json_eq!(a, b);
    }

    #[test]
    fn test_assert_within_ms() {
        assert_within_ms!(50u64, 100u64);
    }

    #[test]
    fn test_assert_tokens_within() {
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        assert_tokens_within!(usage, 200u32);
    }
}
