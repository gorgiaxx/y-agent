//! Langfuse ingestion HTTP sender with retry and circuit breaker.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use super::config::{CircuitBreakerConfig, LangfuseConfig, RetryConfig};
use super::types::IngestionBatchRequest;

#[derive(Debug)]
enum CircuitState {
    Closed,
    Open { since: Instant },
    HalfOpen,
}

pub struct LangfuseHttpSender {
    client: Client,
    ingestion_endpoint: String,
    auth_header: String,
    retry: RetryConfig,
    circuit: Mutex<CircuitState>,
    consecutive_failures: AtomicU32,
    failure_threshold: u32,
    recovery_timeout: Duration,
}

impl LangfuseHttpSender {
    pub fn new(config: &LangfuseConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let base = config.base_url.trim_end_matches('/');
        let ingestion_endpoint = format!("{base}/api/public/ingestion");

        let credentials = format!("{}:{}", config.public_key, config.secret_key);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );

        let CircuitBreakerConfig {
            failure_threshold,
            recovery_timeout_secs,
        } = config.circuit_breaker.clone();

        Self {
            client,
            ingestion_endpoint,
            auth_header,
            retry: config.retry.clone(),
            circuit: Mutex::new(CircuitState::Closed),
            consecutive_failures: AtomicU32::new(0),
            failure_threshold,
            recovery_timeout: Duration::from_secs(recovery_timeout_secs),
        }
    }

    pub async fn send_batch(&self, request: &IngestionBatchRequest) -> Result<(), SendError> {
        if request.batch.is_empty() {
            return Ok(());
        }

        if !self.check_circuit().await {
            return Err(SendError::CircuitOpen);
        }

        let body =
            serde_json::to_vec(request).map_err(|e| SendError::Serialization(e.to_string()))?;

        let result = self.send_with_retry(&body).await;
        self.record_result(result.is_ok()).await;
        result
    }

    async fn send_with_retry(&self, body: &[u8]) -> Result<(), SendError> {
        let mut backoff = self.retry.initial_backoff_ms;

        for attempt in 0..=self.retry.max_retries {
            let resp = self
                .client
                .post(&self.ingestion_endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", &self.auth_header)
                .header("X-Langfuse-Sdk-Name", "rust/y-agent")
                .header("X-Langfuse-Sdk-Version", env!("CARGO_PKG_VERSION"))
                .body(body.to_vec())
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 207 => {
                    debug!(
                        url = &self.ingestion_endpoint,
                        attempt, "Langfuse ingestion succeeded"
                    );
                    return Ok(());
                }
                Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                    let status = r.status().as_u16();
                    if attempt < self.retry.max_retries {
                        warn!(
                            url = &self.ingestion_endpoint,
                            status,
                            attempt,
                            backoff_ms = backoff,
                            "Retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        backoff = (backoff * 2).min(self.retry.max_backoff_ms);
                    } else {
                        return Err(SendError::Http(status));
                    }
                }
                Ok(r) => {
                    return Err(SendError::Http(r.status().as_u16()));
                }
                Err(e) => {
                    if attempt < self.retry.max_retries {
                        warn!(
                            url = &self.ingestion_endpoint,
                            %e, attempt, backoff_ms = backoff,
                            "Retrying after network error"
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        backoff = (backoff * 2).min(self.retry.max_backoff_ms);
                    } else {
                        return Err(SendError::Network(e.to_string()));
                    }
                }
            }
        }
        unreachable!()
    }

    async fn check_circuit(&self) -> bool {
        let mut state = self.circuit.lock().await;
        match *state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open { since } => {
                if since.elapsed() >= self.recovery_timeout {
                    *state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    async fn record_result(&self, success: bool) {
        if success {
            self.consecutive_failures.store(0, Ordering::Relaxed);
            let mut state = self.circuit.lock().await;
            *state = CircuitState::Closed;
        } else {
            let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
            if failures >= self.failure_threshold {
                let mut state = self.circuit.lock().await;
                *state = CircuitState::Open {
                    since: Instant::now(),
                };
                warn!(
                    failures,
                    threshold = self.failure_threshold,
                    "Circuit breaker opened"
                );
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("HTTP error: status {0}")]
    Http(u16),
    #[error("network error: {0}")]
    Network(String),
    #[error("circuit breaker open")]
    CircuitOpen,
}
