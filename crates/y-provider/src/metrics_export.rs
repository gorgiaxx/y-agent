//! Prometheus-compatible metrics export.
//!
//! Renders per-provider metrics into Prometheus text exposition format (text/plain).
//! Supports standard metric types: counters and gauges.
//!
//! Design reference: providers-design.md §Observability

use std::fmt::Write;

use crate::metrics::MetricsSnapshot;

/// Metric name prefix for all y-agent provider metrics.
const METRIC_PREFIX: &str = "y_agent_provider";

/// Render a set of per-provider metrics snapshots into Prometheus text format.
///
/// Each metric is scoped by a `provider` label containing the provider ID.
///
/// # Example output
/// ```text
/// # HELP y_agent_provider_requests_total Total LLM requests made.
/// # TYPE y_agent_provider_requests_total counter
/// y_agent_provider_requests_total{provider="openai-main"} 42
/// ```
pub fn render_prometheus(snapshots: &[(&str, MetricsSnapshot)]) -> String {
    let mut buf = String::with_capacity(2048);

    // -- requests_total (counter)
    write_metric_header(
        &mut buf,
        "requests_total",
        "counter",
        "Total LLM requests made.",
    );
    for (id, snap) in snapshots {
        write_metric_value(&mut buf, "requests_total", id, snap.total_requests as f64);
    }

    // -- errors_total (counter)
    write_metric_header(
        &mut buf,
        "errors_total",
        "counter",
        "Total failed LLM requests.",
    );
    for (id, snap) in snapshots {
        write_metric_value(&mut buf, "errors_total", id, snap.total_errors as f64);
    }

    // -- error_rate (gauge)
    write_metric_header(
        &mut buf,
        "error_rate",
        "gauge",
        "Error rate as a fraction (0.0 to 1.0).",
    );
    for (id, snap) in snapshots {
        write_metric_value(&mut buf, "error_rate", id, snap.error_rate());
    }

    // -- input_tokens_total (counter)
    write_metric_header(
        &mut buf,
        "input_tokens_total",
        "counter",
        "Total input tokens consumed.",
    );
    for (id, snap) in snapshots {
        write_metric_value(
            &mut buf,
            "input_tokens_total",
            id,
            snap.total_input_tokens as f64,
        );
    }

    // -- output_tokens_total (counter)
    write_metric_header(
        &mut buf,
        "output_tokens_total",
        "counter",
        "Total output tokens generated.",
    );
    for (id, snap) in snapshots {
        write_metric_value(
            &mut buf,
            "output_tokens_total",
            id,
            snap.total_output_tokens as f64,
        );
    }

    // -- cost_usd (counter)
    write_metric_header(
        &mut buf,
        "cost_usd",
        "counter",
        "Estimated cost in US dollars.",
    );
    for (id, snap) in snapshots {
        write_metric_value(&mut buf, "cost_usd", id, snap.estimated_cost_usd());
    }

    buf
}

/// Write a HELP + TYPE header block.
fn write_metric_header(buf: &mut String, name: &str, metric_type: &str, help: &str) {
    let _ = writeln!(buf, "# HELP {METRIC_PREFIX}_{name} {help}");
    let _ = writeln!(buf, "# TYPE {METRIC_PREFIX}_{name} {metric_type}");
}

/// Write a single metric value line with provider label.
fn write_metric_value(buf: &mut String, name: &str, provider_id: &str, value: f64) {
    // Prometheus recommends integer representation for counters when possible.
    if value == value.floor() && value.abs() < 1e15 {
        let _ = writeln!(
            buf,
            "{METRIC_PREFIX}_{name}{{provider=\"{provider_id}\"}} {}",
            value as i64
        );
    } else {
        let _ = writeln!(
            buf,
            "{METRIC_PREFIX}_{name}{{provider=\"{provider_id}\"}} {value}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot(
        requests: u64,
        errors: u64,
        input: u64,
        output: u64,
        cost_micros: u64,
    ) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: requests,
            total_errors: errors,
            total_input_tokens: input,
            total_output_tokens: output,
            estimated_cost_micros: cost_micros,
        }
    }

    #[test]
    fn test_render_prometheus_basic() {
        let snapshots = vec![(
            "openai-main",
            sample_snapshot(100, 5, 50000, 30000, 1_500_000),
        )];

        let output = render_prometheus(&snapshots);

        // Verify HELP/TYPE headers.
        assert!(output.contains("# HELP y_agent_provider_requests_total"));
        assert!(output.contains("# TYPE y_agent_provider_requests_total counter"));

        // Verify values.
        assert!(output.contains("y_agent_provider_requests_total{provider=\"openai-main\"} 100"));
        assert!(output.contains("y_agent_provider_errors_total{provider=\"openai-main\"} 5"));
        assert!(
            output.contains("y_agent_provider_input_tokens_total{provider=\"openai-main\"} 50000")
        );
        assert!(
            output.contains("y_agent_provider_output_tokens_total{provider=\"openai-main\"} 30000")
        );
    }

    #[test]
    fn test_render_prometheus_multiple_providers() {
        let snapshots = vec![
            ("openai-main", sample_snapshot(50, 2, 10000, 5000, 500_000)),
            (
                "anthropic-main",
                sample_snapshot(30, 1, 8000, 4000, 300_000),
            ),
        ];

        let output = render_prometheus(&snapshots);

        assert!(output.contains("provider=\"openai-main\""));
        assert!(output.contains("provider=\"anthropic-main\""));
    }

    #[test]
    fn test_render_prometheus_zero_metrics() {
        let snapshots = vec![("empty-provider", sample_snapshot(0, 0, 0, 0, 0))];

        let output = render_prometheus(&snapshots);
        assert!(output.contains("y_agent_provider_requests_total{provider=\"empty-provider\"} 0"));
        assert!(output.contains("y_agent_provider_errors_total{provider=\"empty-provider\"} 0"));
    }

    #[test]
    fn test_render_prometheus_error_rate() {
        let snapshots = vec![("test", sample_snapshot(10, 3, 100, 50, 0))];

        let output = render_prometheus(&snapshots);
        // 3/10 = 0.3
        assert!(output.contains("y_agent_provider_error_rate{provider=\"test\"} 0.3"));
    }

    #[test]
    fn test_render_prometheus_cost_usd() {
        let snapshots = vec![("test", sample_snapshot(10, 0, 100, 50, 1_500_000))];

        let output = render_prometheus(&snapshots);
        // 1_500_000 micro-dollars = $1.50
        assert!(output.contains("y_agent_provider_cost_usd{provider=\"test\"} 1.5"));
    }

    #[test]
    fn test_render_prometheus_empty_input() {
        let output = render_prometheus(&[]);
        // Should still have headers but no data lines.
        assert!(output.contains("# HELP"));
        assert!(!output.contains("provider="));
    }
}
