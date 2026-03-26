//! OpenAI-compatible embedding provider.
//!
//! Implements the `EmbeddingProvider` trait from `y-core` using the `OpenAI`
//! `/embeddings` API. Compatible with any OpenAI-API-compatible endpoint
//! (`OpenAI`, Azure, Ollama, local servers).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use y_core::embedding::{EmbeddingError, EmbeddingProvider, EmbeddingResult};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the `OpenAI` embedding provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Whether embedding is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Provider identifier (currently only "openai" is supported).
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Embedding model name.
    #[serde(default = "default_model")]
    pub model: String,

    /// Embedding vector dimensions.
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,

    /// Base URL for the embedding API.
    #[serde(default = "default_base_url")]
    pub base_url: String,

    /// API key (resolved at construction time, not serialized).
    #[serde(skip)]
    pub api_key: String,

    /// Environment variable name that holds the API key.
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
}

fn default_provider() -> String {
    "openai".to_string()
}
fn default_model() -> String {
    "text-embedding-3-small".to_string()
}
const fn default_dimensions() -> usize {
    1536
}
fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
fn default_api_key_env() -> String {
    "OPENAI_API_KEY".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            model: default_model(),
            dimensions: default_dimensions(),
            base_url: default_base_url(),
            api_key: String::new(),
            api_key_env: default_api_key_env(),
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI API request/response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    model: String,
    usage: EmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    prompt_tokens: u32,
    #[allow(dead_code)]
    total_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct EmbeddingErrorResponse {
    error: EmbeddingApiError,
}

#[derive(Debug, Deserialize)]
struct EmbeddingApiError {
    message: String,
}

// ---------------------------------------------------------------------------
// Provider implementation
// ---------------------------------------------------------------------------

/// OpenAI-compatible embedding provider.
///
/// Works with any API that implements the `OpenAI` `/embeddings` endpoint,
/// including Azure `OpenAI`, Ollama with OpenAI-compatible mode, and others.
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    dimensions: usize,
}

impl OpenAiEmbeddingProvider {
    /// Create a new provider from an `EmbeddingConfig`.
    ///
    /// Resolves the API key from the environment variable specified in config
    /// if `config.api_key` is empty.
    pub fn from_config(config: &EmbeddingConfig) -> Result<Self, EmbeddingError> {
        let api_key = if config.api_key.is_empty() {
            std::env::var(&config.api_key_env).unwrap_or_default()
        } else {
            config.api_key.clone()
        };

        // Skip the error. Empty API key is allowed for local/open endpoints.

        Ok(Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            model: config.model.clone(),
            dimensions: config.dimensions,
        })
    }

    /// Create a new provider with explicit parameters.
    pub fn new(api_key: String, base_url: &str, model: String, dimensions: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            dimensions,
        }
    }

    /// Call the OpenAI-compatible embedding API for multiple texts.
    async fn call_api(&self, texts: &[&str]) -> Result<Vec<EmbeddingResult>, EmbeddingError> {
        let url = format!("{}/embeddings", self.base_url);

        let request_body = EmbeddingRequest {
            model: &self.model,
            input: texts.to_vec(),
            dimensions: Some(self.dimensions),
        };

        let mut request_builder = self
            .client
            .post(&url)
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = request_builder
            .json(&request_body)
            .send()
            .await
            .map_err(|e| EmbeddingError::ProviderError {
                message: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|e| EmbeddingError::ProviderError {
                message: format!("failed to read response body: {e}"),
            })?;

        if !status.is_success() {
            let error_msg = if let Ok(err) = serde_json::from_slice::<EmbeddingErrorResponse>(&body)
            {
                err.error.message
            } else {
                String::from_utf8_lossy(&body).to_string()
            };
            return Err(EmbeddingError::ProviderError {
                message: format!("embedding API error ({status}): {error_msg}"),
            });
        }

        let response: EmbeddingResponse =
            serde_json::from_slice(&body).map_err(|e| EmbeddingError::ProviderError {
                message: format!("failed to parse embedding response: {e}"),
            })?;

        // Distribute token count evenly across inputs (API gives aggregate).
        let per_text_tokens = if texts.is_empty() {
            0
        } else {
            response.usage.prompt_tokens / u32::try_from(texts.len()).unwrap_or(1)
        };

        let results = response
            .data
            .into_iter()
            .map(|d| {
                let dims = d.embedding.len();
                EmbeddingResult {
                    vector: d.embedding,
                    dimensions: dims,
                    model: response.model.clone(),
                    token_count: per_text_tokens,
                }
            })
            .collect();

        Ok(results)
    }
}

impl std::fmt::Debug for OpenAiEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbeddingProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<EmbeddingResult, EmbeddingError> {
        let results = self.call_api(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or(EmbeddingError::ProviderError {
                message: "embedding API returned empty results".to_string(),
            })
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<EmbeddingResult>, EmbeddingError> {
        // OpenAI allows up to 2048 inputs per batch. Split if necessary.
        const MAX_BATCH: usize = 2048;

        if texts.is_empty() {
            return Ok(vec![]);
        }

        let mut all_results = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(MAX_BATCH) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let results = self.call_api(&refs).await?;
            all_results.extend(results);
        }

        Ok(all_results)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = EmbeddingConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "text-embedding-3-small");
        assert_eq!(config.dimensions, 1536);
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.api_key_env, "OPENAI_API_KEY");
    }

    #[test]
    fn test_config_serialization() {
        let config = EmbeddingConfig {
            enabled: true,
            ..Default::default()
        };
        let toml_str = toml::to_string(&config).expect("serialize");
        let parsed: EmbeddingConfig = toml::from_str(&toml_str).expect("deserialize");
        assert!(parsed.enabled);
        assert_eq!(parsed.model, "text-embedding-3-small");
    }

    // Empty key test doesn't apply because we allow empty keys.

    #[test]
    fn test_provider_debug() {
        let provider = OpenAiEmbeddingProvider::new(
            "test-key".to_string(),
            "http://localhost:8080",
            "test-model".to_string(),
            384,
        );
        let debug = format!("{provider:?}");
        assert!(debug.contains("test-model"));
        assert!(!debug.contains("test-key")); // API key must not leak.
    }

    #[test]
    fn test_provider_dimensions_and_model() {
        let provider = OpenAiEmbeddingProvider::new(
            "key".to_string(),
            "http://localhost",
            "my-model".to_string(),
            768,
        );
        assert_eq!(provider.dimensions(), 768);
        assert_eq!(provider.model_name(), "my-model");
    }
}
