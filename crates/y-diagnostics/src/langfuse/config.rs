//! Langfuse OTLP export configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LangfuseConfig {
    pub enabled: bool,
    pub base_url: String,
    pub public_key: String,
    pub secret_key: String,
    pub project_id: Option<String>,
    pub content: ContentConfig,
    pub redaction: RedactionConfig,
    pub sampling: SamplingConfig,
    pub retry: RetryConfig,
    pub feedback: FeedbackConfig,
    pub circuit_breaker: CircuitBreakerConfig,
    pub flush_interval_secs: u64,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: "https://cloud.langfuse.com".to_string(),
            public_key: String::new(),
            secret_key: String::new(),
            project_id: None,
            content: ContentConfig::default(),
            redaction: RedactionConfig::default(),
            sampling: SamplingConfig::default(),
            retry: RetryConfig::default(),
            feedback: FeedbackConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
            flush_interval_secs: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentConfig {
    pub capture_input: bool,
    pub capture_output: bool,
    pub max_content_length: usize,
}

impl Default for ContentConfig {
    fn default() -> Self {
        Self {
            capture_input: false,
            capture_output: false,
            max_content_length: 10_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RedactionConfig {
    pub enabled: bool,
    pub patterns: Vec<String>,
    pub replacement: String,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            patterns: vec![
                r"(?i)(api[_-]?key|secret|token|password|bearer)\s*[:=]\s*\S+".to_string(),
                r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b".to_string(),
            ],
            replacement: "[REDACTED]".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SamplingConfig {
    pub rate: f64,
    pub include_tags: Vec<String>,
    pub exclude_tags: Vec<String>,
}

impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            rate: 1.0,
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            max_backoff_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackConfig {
    pub import_enabled: bool,
    pub poll_interval_secs: u64,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            import_enabled: false,
            poll_interval_secs: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub recovery_timeout_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout_secs: 60,
        }
    }
}
