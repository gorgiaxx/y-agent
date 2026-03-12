//! Error classifier: normalizes provider errors into a `StandardError` enum.
//!
//! Design reference: providers-design.md §Error Classification
//!
//! The classifier examines HTTP status codes and error body content to
//! categorize provider failures into standard types. These standard errors
//! drive freeze duration decisions, alerting, and retry strategies.

use std::time::Duration;

/// Standardized error classification for provider failures.
///
/// All provider-specific errors are normalized to one of these variants,
/// which then drive the freeze/retry logic in the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandardError {
    /// Context window exceeded (too many tokens).
    ContextWindowExceeded,
    /// Rate limited by the provider.
    RateLimited {
        /// Suggested retry delay from the provider (Retry-After header).
        retry_after: Option<Duration>,
    },
    /// API quota or billing limit exhausted.
    QuotaExhausted,
    /// Authentication failed (invalid credentials format, etc.).
    AuthenticationFailed,
    /// API key is invalid or revoked.
    KeyInvalid,
    /// Account has insufficient balance/credits.
    InsufficientBalance,
    /// Requested model does not exist or is not accessible.
    ModelNotFound,
    /// Server-side error (5xx).
    ServerError,
    /// Network connectivity issue.
    NetworkError,
    /// Content was filtered by the provider's safety system.
    ContentFiltered,
    /// Unclassified error.
    Unknown,
}

impl StandardError {
    /// Recommended freeze duration for this error type.
    ///
    /// Returns `None` for errors that should cause permanent freeze
    /// (requiring manual intervention).
    pub fn freeze_duration(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => {
                Some(retry_after.unwrap_or(Duration::from_secs(60)))
            }
            Self::ServerError => Some(Duration::from_secs(300)),
            Self::NetworkError => Some(Duration::from_secs(30)),
            Self::ModelNotFound => Some(Duration::from_secs(3600)),
            Self::AuthenticationFailed => Some(Duration::from_secs(86400)), // 24h
            Self::Unknown => Some(Duration::from_secs(60)),
            // Not a provider issue (context window, content filter) — don't freeze.
            // Permanent errors (key invalid, quota, balance) — freeze duration
            // is effectively infinite, handled by freeze_permanent().
            Self::ContextWindowExceeded
            | Self::ContentFiltered
            | Self::KeyInvalid
            | Self::QuotaExhausted
            | Self::InsufficientBalance => None,
        }
    }

    /// Whether this error should cause a permanent freeze (no auto-thaw).
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::KeyInvalid | Self::QuotaExhausted | Self::InsufficientBalance
        )
    }

    /// Whether this error is a transient provider issue (freeze + retry).
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::ServerError | Self::NetworkError | Self::Unknown
        )
    }

    /// Whether this error should NOT cause a provider freeze.
    ///
    /// Some errors (context window, content filter) are request-specific
    /// and don't indicate a provider problem.
    pub fn should_freeze(&self) -> bool {
        !matches!(self, Self::ContextWindowExceeded | Self::ContentFiltered)
    }
}

/// Classify a provider error from HTTP status code and response body.
///
/// This function examines the status code first, then falls back to
/// regex-like pattern matching on the error body for more specific
/// classification.
pub fn classify(status: u16, body: &str) -> StandardError {
    // 1. Status-code based classification.
    match status {
        401 => classify_auth_error(body),
        403 => classify_forbidden_error(body),
        429 => StandardError::RateLimited { retry_after: None },
        404 => {
            if body_contains_any(body, &["model", "not found", "does not exist"]) {
                StandardError::ModelNotFound
            } else {
                StandardError::Unknown
            }
        }
        400 => classify_bad_request(body),
        500..=599 => StandardError::ServerError,
        0 => StandardError::NetworkError, // Status 0 indicates network failure.
        _ => classify_from_body(body),
    }
}

/// Classify a provider error from a `ProviderError` enum.
///
/// This provides a bridge from the existing `ProviderError` type to
/// the new `StandardError` classification.
pub fn classify_provider_error(error: &y_core::provider::ProviderError) -> StandardError {
    use y_core::provider::ProviderError;
    match error {
        ProviderError::RateLimited {
            retry_after_secs, ..
        } => StandardError::RateLimited {
            retry_after: Some(Duration::from_secs(*retry_after_secs)),
        },
        ProviderError::QuotaExhausted { .. } => StandardError::QuotaExhausted,
        ProviderError::AuthenticationFailed { .. } => StandardError::AuthenticationFailed,
        ProviderError::KeyInvalid { .. } => StandardError::KeyInvalid,
        ProviderError::ServerError { message, .. } => {
            // Try to sub-classify server errors from the message.
            if message.contains("context") && message.contains("length") {
                StandardError::ContextWindowExceeded
            } else {
                StandardError::ServerError
            }
        }
        ProviderError::NetworkError { .. } => StandardError::NetworkError,
        ProviderError::NoProviderAvailable { .. }
        | ProviderError::Cancelled
        | ProviderError::ParseError { .. } => StandardError::Unknown,
        ProviderError::Other { message } => classify_from_body(message),
    }
}

// ---------------------------------------------------------------------------
// Internal classification helpers
// ---------------------------------------------------------------------------

fn classify_auth_error(body: &str) -> StandardError {
    let lower = body.to_lowercase();
    if (lower.contains("invalid") && lower.contains("key"))
        || lower.contains("expired")
        || lower.contains("revoked")
    {
        StandardError::KeyInvalid
    } else {
        StandardError::AuthenticationFailed
    }
}

fn classify_forbidden_error(body: &str) -> StandardError {
    let lower = body.to_lowercase();
    if lower.contains("quota") || lower.contains("exceeded") {
        StandardError::QuotaExhausted
    } else if lower.contains("balance")
        || lower.contains("insufficient")
        || lower.contains("billing")
        || lower.contains("payment")
    {
        StandardError::InsufficientBalance
    } else {
        StandardError::AuthenticationFailed
    }
}

fn classify_bad_request(body: &str) -> StandardError {
    let lower = body.to_lowercase();
    if (lower.contains("context") && (lower.contains("length") || lower.contains("window")))
        || (lower.contains("maximum") && lower.contains("token"))
    {
        StandardError::ContextWindowExceeded
    } else if lower.contains("content_filter") || lower.contains("content filter") {
        StandardError::ContentFiltered
    } else if lower.contains("model") && lower.contains("not") {
        StandardError::ModelNotFound
    } else {
        StandardError::Unknown
    }
}

fn classify_from_body(body: &str) -> StandardError {
    let lower = body.to_lowercase();
    if lower.contains("rate limit") || lower.contains("rate_limit") {
        StandardError::RateLimited { retry_after: None }
    } else if lower.contains("quota") || lower.contains("exceeded") {
        StandardError::QuotaExhausted
    } else if lower.contains("invalid api key") || lower.contains("invalid_api_key") {
        StandardError::KeyInvalid
    } else if lower.contains("insufficient") && lower.contains("balance") {
        StandardError::InsufficientBalance
    } else if lower.contains("context") && lower.contains("length") {
        StandardError::ContextWindowExceeded
    } else if lower.contains("content_filter") || lower.contains("content filter") {
        StandardError::ContentFiltered
    } else {
        StandardError::Unknown
    }
}

/// Check if body contains any of the given substrings (case-insensitive).
fn body_contains_any(body: &str, patterns: &[&str]) -> bool {
    let lower = body.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Status-code based classification
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_rate_limited() {
        let err = classify(429, "rate limit exceeded");
        assert_eq!(err, StandardError::RateLimited { retry_after: None });
    }

    #[test]
    fn test_classify_auth_failed() {
        let err = classify(401, "unauthorized");
        assert_eq!(err, StandardError::AuthenticationFailed);
    }

    #[test]
    fn test_classify_invalid_key() {
        let err = classify(401, "Invalid API key provided");
        assert_eq!(err, StandardError::KeyInvalid);
    }

    #[test]
    fn test_classify_quota_exhausted() {
        let err = classify(403, "quota exceeded");
        assert_eq!(err, StandardError::QuotaExhausted);
    }

    #[test]
    fn test_classify_insufficient_balance() {
        let err = classify(403, "insufficient balance");
        assert_eq!(err, StandardError::InsufficientBalance);
    }

    #[test]
    fn test_classify_billing_issue() {
        let err = classify(403, "billing hard limit reached, payment required");
        assert_eq!(err, StandardError::InsufficientBalance);
    }

    #[test]
    fn test_classify_model_not_found() {
        let err = classify(404, "The model gpt-5 does not exist");
        assert_eq!(err, StandardError::ModelNotFound);
    }

    #[test]
    fn test_classify_context_window_exceeded() {
        let err = classify(400, "maximum context length exceeded");
        assert_eq!(err, StandardError::ContextWindowExceeded);
    }

    #[test]
    fn test_classify_context_token_limit() {
        let err = classify(400, "maximum token limit reached");
        assert_eq!(err, StandardError::ContextWindowExceeded);
    }

    #[test]
    fn test_classify_content_filtered() {
        let err = classify(400, "content_filter triggered");
        assert_eq!(err, StandardError::ContentFiltered);
    }

    #[test]
    fn test_classify_server_error() {
        let err = classify(500, "internal server error");
        assert_eq!(err, StandardError::ServerError);
    }

    #[test]
    fn test_classify_bad_gateway() {
        let err = classify(502, "Bad Gateway");
        assert_eq!(err, StandardError::ServerError);
    }

    #[test]
    fn test_classify_network_error() {
        let err = classify(0, "connection refused");
        assert_eq!(err, StandardError::NetworkError);
    }

    #[test]
    fn test_classify_unknown_status() {
        let err = classify(418, "I'm a teapot");
        assert_eq!(err, StandardError::Unknown);
    }

    // -----------------------------------------------------------------------
    // ProviderError bridge
    // -----------------------------------------------------------------------

    #[test]
    fn test_classify_provider_error_rate_limited() {
        use y_core::provider::ProviderError;
        let err = ProviderError::RateLimited {
            provider: "test".into(),
            retry_after_secs: 120,
        };
        let std_err = classify_provider_error(&err);
        assert_eq!(
            std_err,
            StandardError::RateLimited {
                retry_after: Some(Duration::from_secs(120))
            }
        );
    }

    #[test]
    fn test_classify_provider_error_key_invalid() {
        use y_core::provider::ProviderError;
        let err = ProviderError::KeyInvalid {
            provider: "test".into(),
        };
        assert_eq!(classify_provider_error(&err), StandardError::KeyInvalid);
    }

    #[test]
    fn test_classify_provider_error_network() {
        use y_core::provider::ProviderError;
        let err = ProviderError::NetworkError {
            message: "connection reset".into(),
        };
        assert_eq!(classify_provider_error(&err), StandardError::NetworkError);
    }

    // -----------------------------------------------------------------------
    // Freeze behavior
    // -----------------------------------------------------------------------

    #[test]
    fn test_permanent_errors() {
        assert!(StandardError::KeyInvalid.is_permanent());
        assert!(StandardError::QuotaExhausted.is_permanent());
        assert!(StandardError::InsufficientBalance.is_permanent());
        assert!(!StandardError::RateLimited { retry_after: None }.is_permanent());
        assert!(!StandardError::ServerError.is_permanent());
    }

    #[test]
    fn test_transient_errors() {
        assert!(StandardError::RateLimited { retry_after: None }.is_transient());
        assert!(StandardError::ServerError.is_transient());
        assert!(StandardError::NetworkError.is_transient());
        assert!(!StandardError::KeyInvalid.is_transient());
    }

    #[test]
    fn test_should_freeze() {
        assert!(!StandardError::ContextWindowExceeded.should_freeze());
        assert!(!StandardError::ContentFiltered.should_freeze());
        assert!(StandardError::RateLimited { retry_after: None }.should_freeze());
        assert!(StandardError::KeyInvalid.should_freeze());
    }

    #[test]
    fn test_freeze_durations() {
        // Rate limited: default 60s.
        let rl = StandardError::RateLimited { retry_after: None };
        assert_eq!(rl.freeze_duration(), Some(Duration::from_secs(60)));

        // Rate limited with custom retry-after.
        let rl_custom = StandardError::RateLimited {
            retry_after: Some(Duration::from_secs(120)),
        };
        assert_eq!(rl_custom.freeze_duration(), Some(Duration::from_secs(120)));

        // Server error: 5min.
        assert_eq!(
            StandardError::ServerError.freeze_duration(),
            Some(Duration::from_secs(300))
        );

        // Auth: 24h.
        assert_eq!(
            StandardError::AuthenticationFailed.freeze_duration(),
            Some(Duration::from_secs(86400))
        );

        // Key invalid: permanent (None).
        assert_eq!(StandardError::KeyInvalid.freeze_duration(), None);

        // Quota: permanent (None).
        assert_eq!(StandardError::QuotaExhausted.freeze_duration(), None);

        // Context window: no freeze (None).
        assert_eq!(StandardError::ContextWindowExceeded.freeze_duration(), None);
    }
}
