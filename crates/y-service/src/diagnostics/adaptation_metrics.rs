//! Aggregation of durable adaptation outcomes from diagnostics traces.

use std::collections::BTreeMap;

use y_diagnostics::{Trace, TraceStatus};

/// Aggregated observed outcome metrics for one resolved orchestration mode.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct OrchestrationModeMetrics {
    pub mode: String,
    pub total_runs: usize,
    pub successful_runs: usize,
    pub failed_runs: usize,
    pub cancelled_runs: usize,
    pub success_rate: f64,
    pub average_tokens: f64,
    pub average_cost_usd: f64,
    pub average_duration_ms: f64,
}

/// Aggregated observed outcome metrics for one dynamic-agent version.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct DynamicAgentVersionMetrics {
    pub agent_id: String,
    pub version: u64,
    pub total_runs: usize,
    pub successful_runs: usize,
    pub failed_runs: usize,
    pub cancelled_runs: usize,
    pub success_rate: f64,
    pub average_tokens: f64,
    pub average_cost_usd: f64,
    pub average_duration_ms: f64,
}

/// Evidence-backed regression between adjacent dynamic-agent versions.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct DynamicAgentRegressionFinding {
    pub agent_id: String,
    pub baseline_version: u64,
    pub current_version: u64,
    pub baseline_samples: usize,
    pub current_samples: usize,
    pub baseline_success_rate: f64,
    pub current_success_rate: f64,
    pub success_rate_drop: f64,
    pub recommendation: String,
}

#[derive(Default)]
struct OutcomeAccumulator {
    total_runs: usize,
    successful_runs: usize,
    failed_runs: usize,
    cancelled_runs: usize,
    total_tokens: u64,
    total_cost_usd: f64,
    total_duration_ms: u64,
}

impl OutcomeAccumulator {
    fn observe(&mut self, trace: &Trace) {
        let status = trace
            .metadata
            .pointer("/user_feedback/score")
            .and_then(serde_json::Value::as_f64)
            .map_or(trace.status, |score| {
                if score <= 0.25 {
                    TraceStatus::Failed
                } else if score >= 0.75 {
                    TraceStatus::Completed
                } else {
                    trace.status
                }
            });
        match status {
            TraceStatus::Completed => self.successful_runs += 1,
            TraceStatus::Failed => self.failed_runs += 1,
            TraceStatus::Cancelled => self.cancelled_runs += 1,
            TraceStatus::Active => return,
        }
        self.total_runs += 1;
        self.total_tokens = self
            .total_tokens
            .saturating_add(trace.total_input_tokens)
            .saturating_add(trace.total_output_tokens);
        self.total_cost_usd += trace.total_cost_usd;
        self.total_duration_ms = self
            .total_duration_ms
            .saturating_add(trace.total_duration_ms.unwrap_or_default());
    }

    fn averages(&self) -> (f64, f64, f64, f64) {
        let denominator = self.total_runs as f64;
        (
            self.successful_runs as f64 / denominator,
            self.total_tokens as f64 / denominator,
            self.total_cost_usd / denominator,
            self.total_duration_ms as f64 / denominator,
        )
    }
}

pub(super) fn orchestration_mode_metrics(traces: Vec<Trace>) -> Vec<OrchestrationModeMetrics> {
    let mut grouped = BTreeMap::<String, OutcomeAccumulator>::new();
    for trace in traces {
        if trace.status == TraceStatus::Active {
            continue;
        }
        let Some(mode) = trace
            .metadata
            .pointer("/orchestration/selected_mode")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        grouped.entry(mode.to_string()).or_default().observe(&trace);
    }

    grouped
        .into_iter()
        .map(|(mode, outcomes)| {
            let (success_rate, average_tokens, average_cost_usd, average_duration_ms) =
                outcomes.averages();
            OrchestrationModeMetrics {
                mode,
                total_runs: outcomes.total_runs,
                successful_runs: outcomes.successful_runs,
                failed_runs: outcomes.failed_runs,
                cancelled_runs: outcomes.cancelled_runs,
                success_rate,
                average_tokens,
                average_cost_usd,
                average_duration_ms,
            }
        })
        .collect()
}

pub(super) fn dynamic_agent_version_metrics(traces: Vec<Trace>) -> Vec<DynamicAgentVersionMetrics> {
    let mut grouped = BTreeMap::<(String, u64), OutcomeAccumulator>::new();
    for trace in traces {
        if trace.status == TraceStatus::Active {
            continue;
        }
        let Some(agent_id) = trace
            .metadata
            .pointer("/dynamic_agent/id")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(version) = trace
            .metadata
            .pointer("/dynamic_agent/version")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        grouped
            .entry((agent_id.to_string(), version))
            .or_default()
            .observe(&trace);
    }

    grouped
        .into_iter()
        .map(|((agent_id, version), outcomes)| {
            let (success_rate, average_tokens, average_cost_usd, average_duration_ms) =
                outcomes.averages();
            DynamicAgentVersionMetrics {
                agent_id,
                version,
                total_runs: outcomes.total_runs,
                successful_runs: outcomes.successful_runs,
                failed_runs: outcomes.failed_runs,
                cancelled_runs: outcomes.cancelled_runs,
                success_rate,
                average_tokens,
                average_cost_usd,
                average_duration_ms,
            }
        })
        .collect()
}

pub(super) fn dynamic_agent_regressions(
    metrics: &[DynamicAgentVersionMetrics],
    min_samples: usize,
    max_success_rate_drop: f64,
) -> Vec<DynamicAgentRegressionFinding> {
    let mut by_agent = BTreeMap::<&str, Vec<&DynamicAgentVersionMetrics>>::new();
    for metric in metrics {
        by_agent.entry(&metric.agent_id).or_default().push(metric);
    }

    let mut findings = Vec::new();
    for versions in by_agent.values_mut() {
        versions.sort_by_key(|metric| metric.version);
        for pair in versions.windows(2) {
            let baseline = pair[0];
            let current = pair[1];
            if baseline.total_runs < min_samples || current.total_runs < min_samples {
                continue;
            }
            let drop = baseline.success_rate - current.success_rate;
            if drop < max_success_rate_drop {
                continue;
            }
            findings.push(DynamicAgentRegressionFinding {
                agent_id: current.agent_id.clone(),
                baseline_version: baseline.version,
                current_version: current.version,
                baseline_samples: baseline.total_runs,
                current_samples: current.total_runs,
                baseline_success_rate: baseline.success_rate,
                current_success_rate: current.success_rate,
                success_rate_drop: drop,
                recommendation:
                    "Review the current definition and consider a supervised rollback or update"
                        .to_string(),
            });
        }
    }
    findings
}
