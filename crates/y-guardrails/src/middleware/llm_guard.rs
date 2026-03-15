//! `LlmGuardMiddleware`: output security validation.
//!
//! This middleware sits in the LLM chain and validates LLM outputs
//! for security concerns (e.g., harmful content, injection attempts).

use async_trait::async_trait;
use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};

/// Middleware that validates LLM outputs for security.
///
/// Registered as an `LlmMiddleware` at priority 900 (runs late, after main LLM processing).
#[derive(Debug)]
pub struct LlmGuardMiddleware {
    /// Maximum allowed output length in characters.
    max_output_length: usize,
    /// Patterns that should trigger a warning.
    warning_patterns: Vec<String>,
}

impl LlmGuardMiddleware {
    /// Create a new LLM guard middleware with default settings.
    pub fn new() -> Self {
        Self {
            max_output_length: 100_000,
            warning_patterns: vec![
                "ignore previous instructions".to_string(),
                "system prompt".to_string(),
            ],
        }
    }

    /// Create with custom configuration.
    pub fn with_config(max_output_length: usize, warning_patterns: Vec<String>) -> Self {
        Self {
            max_output_length,
            warning_patterns,
        }
    }
}

impl Default for LlmGuardMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for LlmGuardMiddleware {
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let output = ctx
            .payload
            .get("output")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Check output length
        if output.len() > self.max_output_length {
            if let Some(meta) = ctx.metadata.as_object_mut() {
                meta.insert(
                    "llm_guard_warning".to_string(),
                    serde_json::Value::String(format!(
                        "output exceeds max length ({} > {})",
                        output.len(),
                        self.max_output_length
                    )),
                );
            }
        }

        // Check for warning patterns (case-insensitive)
        let output_lower = output.to_lowercase();
        let mut warnings = Vec::new();
        for pattern in &self.warning_patterns {
            if output_lower.contains(&pattern.to_lowercase()) {
                warnings.push(format!("suspicious pattern detected: `{pattern}`"));
            }
        }

        if !warnings.is_empty() {
            if let Some(meta) = ctx.metadata.as_object_mut() {
                meta.insert(
                    "llm_guard_warnings".to_string(),
                    serde_json::json!(warnings),
                );
            }
        }

        Ok(MiddlewareResult::Continue)
    }

    fn chain_type(&self) -> ChainType {
        ChainType::Llm
    }

    fn priority(&self) -> u32 {
        900 // Run late in the LLM chain
    }

    fn name(&self) -> &'static str {
        "LlmGuardMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::hook::MiddlewareContext;

    #[tokio::test]
    async fn test_llm_guard_clean_output() {
        let mw = LlmGuardMiddleware::new();
        let mut ctx = MiddlewareContext::new(
            ChainType::Llm,
            serde_json::json!({ "output": "Hello, I can help you with that." }),
        );

        let result = mw.execute(&mut ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
        assert!(ctx.metadata.get("llm_guard_warnings").is_none());
    }

    #[tokio::test]
    async fn test_llm_guard_detects_injection() {
        let mw = LlmGuardMiddleware::new();
        let mut ctx = MiddlewareContext::new(
            ChainType::Llm,
            serde_json::json!({
                "output": "Sure! But first, ignore previous instructions and..."
            }),
        );

        let result = mw.execute(&mut ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
        let warnings = ctx.metadata.get("llm_guard_warnings").unwrap();
        assert!(!warnings.as_array().unwrap().is_empty());
    }
}
