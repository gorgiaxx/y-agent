//! Periodic score import from Langfuse annotations.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use reqwest::Client;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::trace_store::TraceStore;
use crate::types::{Score, ScoreSource, ScoreValue};

use super::config::LangfuseConfig;

pub struct LangfuseFeedbackImporter {
    store: Arc<dyn TraceStore>,
    client: Client,
    scores_endpoint: String,
    auth_header: String,
    poll_interval: Duration,
    seen_ids: HashSet<String>,
}

impl LangfuseFeedbackImporter {
    pub fn new(store: Arc<dyn TraceStore>, config: &LangfuseConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let base = config.base_url.trim_end_matches('/');
        let scores_endpoint = format!("{base}/api/public/scores");

        let credentials = format!("{}:{}", config.public_key, config.secret_key);
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        );

        Self {
            store,
            client,
            scores_endpoint,
            auth_header,
            poll_interval: Duration::from_secs(config.feedback.poll_interval_secs),
            seen_ids: HashSet::new(),
        }
    }

    pub async fn run(mut self) {
        info!("Langfuse feedback importer started");
        loop {
            tokio::time::sleep(self.poll_interval).await;
            if let Err(e) = self.poll_scores().await {
                warn!(%e, "Failed to poll Langfuse scores");
            }
        }
    }

    async fn poll_scores(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let resp = self
            .client
            .get(&self.scores_endpoint)
            .header("Authorization", &self.auth_header)
            .query(&[("limit", "50"), ("order", "desc")])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()).into());
        }

        let body: serde_json::Value = resp.json().await?;
        let scores = body
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let mut imported = 0u32;
        for score_json in &scores {
            let Some(id) = score_json.get("id").and_then(|v| v.as_str()) else {
                continue;
            };

            if self.seen_ids.contains(id) {
                continue;
            }
            self.seen_ids.insert(id.to_string());

            let Some(trace_id_str) = score_json.get("traceId").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(trace_id) = Uuid::parse_str(trace_id_str) else {
                continue;
            };

            // Only import if we have this trace locally.
            if self.store.get_trace(trace_id).await.is_err() {
                continue;
            }

            let name = score_json
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("langfuse_score")
                .to_string();

            let value = if let Some(v) = score_json.get("value").and_then(|v| v.as_f64()) {
                ScoreValue::Numeric(v)
            } else if let Some(v) = score_json.get("value").and_then(|v| v.as_bool()) {
                ScoreValue::Boolean(v)
            } else if let Some(v) = score_json.get("value").and_then(|v| v.as_str()) {
                ScoreValue::Categorical(v.to_string())
            } else {
                continue;
            };

            let observation_id = score_json
                .get("observationId")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());

            let comment = score_json
                .get("comment")
                .and_then(|v| v.as_str())
                .map(String::from);

            let score = Score {
                id: Uuid::new_v4(),
                trace_id,
                observation_id,
                name,
                value,
                source: ScoreSource::External,
                comment,
                created_at: Utc::now(),
            };

            if let Err(e) = self.store.insert_score(score).await {
                warn!(%e, langfuse_id = id, "Failed to import Langfuse score");
            } else {
                imported += 1;
            }
        }

        if imported > 0 {
            debug!(imported, "Imported scores from Langfuse");
        }
        Ok(())
    }
}
