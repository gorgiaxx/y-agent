//! Cost intelligence: accumulate and summarise LLM spending.

use std::collections::HashMap;

use chrono::{NaiveDate, Utc};
use uuid::Uuid;

use crate::trace_store::{TraceStore, TraceStoreError};
use crate::types::{CostRecord, DailyCostSummary, ObservationType};

/// Accumulates cost data from observations and provides summaries.
pub struct CostIntelligence<S> {
    store: S,
}

impl<S: TraceStore> CostIntelligence<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Calculate total cost for a specific trace.
    pub async fn trace_cost(&self, trace_id: Uuid) -> Result<f64, TraceStoreError> {
        let observations = self.store.get_observations(trace_id).await?;
        Ok(observations.iter().map(|o| o.cost_usd).sum())
    }

    /// Get cost breakdown by model for a specific trace.
    pub async fn trace_cost_by_model(
        &self,
        trace_id: Uuid,
    ) -> Result<Vec<CostRecord>, TraceStoreError> {
        let observations = self.store.get_observations(trace_id).await?;

        let mut model_costs: HashMap<String, CostRecord> = HashMap::new();

        for obs in observations
            .iter()
            .filter(|o| o.obs_type == ObservationType::Generation)
        {
            let model = obs.model.clone().unwrap_or_else(|| "unknown".into());
            let entry = model_costs.entry(model.clone()).or_insert(CostRecord {
                model,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
            });
            entry.input_tokens += obs.input_tokens;
            entry.output_tokens += obs.output_tokens;
            entry.cost_usd += obs.cost_usd;
        }

        Ok(model_costs.into_values().collect())
    }

    /// Generate a daily cost summary for the given date.
    pub async fn daily_summary(
        &self,
        date: NaiveDate,
    ) -> Result<DailyCostSummary, TraceStoreError> {
        let (start, end) = day_bounds(date)?;

        let traces = self.store.list_traces(None, Some(start), 10_000).await?;

        let day_traces: Vec<_> = traces
            .into_iter()
            .filter(|t| t.started_at >= start && t.started_at < end)
            .collect();

        let mut by_model: HashMap<String, CostRecord> = HashMap::new();
        let mut total_cost = 0.0;

        for trace in &day_traces {
            let observations = self.store.get_observations(trace.id).await?;
            for obs in observations
                .iter()
                .filter(|o| o.obs_type == ObservationType::Generation)
            {
                let model = obs.model.clone().unwrap_or_else(|| "unknown".into());
                let entry = by_model.entry(model.clone()).or_insert(CostRecord {
                    model,
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                });
                entry.input_tokens += obs.input_tokens;
                entry.output_tokens += obs.output_tokens;
                entry.cost_usd += obs.cost_usd;
                total_cost += obs.cost_usd;
            }
        }

        Ok(DailyCostSummary {
            date,
            total_cost_usd: total_cost,
            total_traces: day_traces.len() as u64,
            by_model: by_model.into_values().collect(),
        })
    }
}

fn day_bounds(
    date: NaiveDate,
) -> Result<(chrono::DateTime<Utc>, chrono::DateTime<Utc>), TraceStoreError> {
    let start = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| TraceStoreError::Storage {
            message: format!("invalid daily summary start date: {date}"),
        })?;
    let next_day = date.succ_opt().ok_or_else(|| TraceStoreError::Storage {
        message: format!("daily summary date has no following day: {date}"),
    })?;
    let end = next_day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| TraceStoreError::Storage {
            message: format!("invalid daily summary end date: {next_day}"),
        })?;

    Ok((
        chrono::DateTime::from_naive_utc_and_offset(start, Utc),
        chrono::DateTime::from_naive_utc_and_offset(end, Utc),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_store::InMemoryTraceStore;
    use crate::types::*;

    #[tokio::test]
    async fn test_cost_accumulation() {
        let store = InMemoryTraceStore::new();
        let session = Uuid::new_v4();
        let trace = Trace::new(session, "cost-trace");
        let trace_id = trace.id;
        store.insert_trace(trace).await.unwrap();

        // Add two generation observations with costs.
        let mut obs1 = Observation::new(trace_id, ObservationType::Generation, "gen-1");
        obs1.model = Some("gpt-4".into());
        obs1.input_tokens = 100;
        obs1.output_tokens = 50;
        obs1.cost_usd = 0.005;
        store.insert_observation(obs1).await.unwrap();

        let mut obs2 = Observation::new(trace_id, ObservationType::Generation, "gen-2");
        obs2.model = Some("gpt-4".into());
        obs2.input_tokens = 200;
        obs2.output_tokens = 100;
        obs2.cost_usd = 0.010;
        store.insert_observation(obs2).await.unwrap();

        let cost = CostIntelligence::new(store);

        let total = cost.trace_cost(trace_id).await.unwrap();
        assert!((total - 0.015).abs() < f64::EPSILON);

        let by_model = cost.trace_cost_by_model(trace_id).await.unwrap();
        assert_eq!(by_model.len(), 1);
        assert_eq!(by_model[0].model, "gpt-4");
        assert_eq!(by_model[0].input_tokens, 300);
        assert_eq!(by_model[0].output_tokens, 150);
    }

    #[tokio::test]
    async fn test_daily_summary() {
        let store = InMemoryTraceStore::new();
        let session = Uuid::new_v4();

        let trace = Trace::new(session, "today-trace");
        let trace_id = trace.id;
        store.insert_trace(trace).await.unwrap();

        let mut obs = Observation::new(trace_id, ObservationType::Generation, "gen");
        obs.model = Some("claude-3".into());
        obs.cost_usd = 0.02;
        store.insert_observation(obs).await.unwrap();

        let cost = CostIntelligence::new(store);

        let today = Utc::now().date_naive();
        let summary = cost.daily_summary(today).await.unwrap();
        assert_eq!(summary.total_traces, 1);
        assert!((summary.total_cost_usd - 0.02).abs() < f64::EPSILON);
        assert_eq!(summary.by_model.len(), 1);
    }

    #[tokio::test]
    async fn test_daily_summary_rejects_unrepresentable_date() {
        let store = InMemoryTraceStore::new();
        let cost = CostIntelligence::new(store);

        let result = cost.daily_summary(NaiveDate::MAX).await;

        assert!(matches!(result, Err(TraceStoreError::Storage { .. })));
    }
}
