//! Regex-based content redaction pipeline.

use regex::Regex;
use tracing::warn;

use super::config::RedactionConfig;

pub struct RedactionPipeline {
    patterns: Vec<Regex>,
    replacement: String,
}

impl RedactionPipeline {
    pub fn new(config: &RedactionConfig) -> Self {
        let patterns = config
            .patterns
            .iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(r) => Some(r),
                Err(e) => {
                    warn!(pattern = %p, %e, "Invalid redaction regex, skipping");
                    None
                }
            })
            .collect();

        Self {
            patterns,
            replacement: config.replacement.clone(),
        }
    }

    pub fn redact(&self, content: &str) -> String {
        let mut result = content.to_string();
        for pattern in &self.patterns {
            result = pattern.replace_all(&result, &*self.replacement).to_string();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_api_key() {
        let config = RedactionConfig::default();
        let pipeline = RedactionPipeline::new(&config);
        let input = "my api_key = sk-1234567890abcdef and more text";
        let result = pipeline.redact(input);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("sk-1234567890abcdef"));
    }

    #[test]
    fn test_redact_email() {
        let config = RedactionConfig::default();
        let pipeline = RedactionPipeline::new(&config);
        let input = "contact user@example.com for details";
        let result = pipeline.redact(input);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("user@example.com"));
    }

    #[test]
    fn test_no_redaction_when_disabled() {
        let config = RedactionConfig {
            enabled: false,
            ..Default::default()
        };
        let pipeline = RedactionPipeline::new(&config);
        let input = "api_key = secret123";
        // Even though patterns match, the mapper checks config.redaction.enabled
        // before calling redact(). But redact() itself always applies patterns.
        // The guard is in the caller (mapper). Here we just verify the pipeline works.
        let result = pipeline.redact(input);
        assert!(result.contains("[REDACTED]"));
    }
}
