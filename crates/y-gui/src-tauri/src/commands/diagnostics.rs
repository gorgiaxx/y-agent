//! Diagnostics command handlers -- query historical traces and observations.

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Response types (serialised to camelCase for the frontend).
// ---------------------------------------------------------------------------

/// A single historical diagnostic entry returned to the frontend.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoricalEntry {
    LlmResponse {
        iteration: usize,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
        cost_usd: f64,
        tool_calls_requested: Vec<String>,
        prompt_preview: String,
        response_text: String,
        timestamp: String,
    },
    ToolResult {
        name: String,
        success: bool,
        duration_ms: u64,
        result_preview: String,
        timestamp: String,
    },
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Fetch historical diagnostics for a session, ordered by time.
///
/// Returns a flat list of entries reconstructed from stored Traces and
/// Observations.  Limited to the N most recent traces (default 50) so the
/// panel does not grow unbounded for long-lived sessions.
#[tauri::command]
pub async fn diagnostics_get_by_session(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<HistoricalEntry>, String> {
    let store = state.container.diagnostics.store();
    let limit = limit.unwrap_or(50);

    // Fetch traces for this session (most recent first).
    let traces = store
        .list_traces_by_session(&session_id, limit)
        .await
        .map_err(|e| format!("Failed to list traces: {e}"))?;

    let mut entries: Vec<(chrono::DateTime<chrono::Utc>, HistoricalEntry)> = Vec::new();

    for trace in &traces {
        // Fetch observations for this trace.
        let observations = store
            .get_observations(trace.id)
            .await
            .unwrap_or_default();

        // Sort by sequence, then started_at.
        let mut obs_sorted = observations;
        obs_sorted.sort_by(|a, b| {
            a.sequence.cmp(&b.sequence).then(a.started_at.cmp(&b.started_at))
        });

        // Track iteration counter for LLM calls within this trace.
        let mut llm_iter = 0usize;

        for obs in &obs_sorted {
            let ts = obs.completed_at.unwrap_or(obs.started_at);
            let duration_ms = obs
                .metadata
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            match obs.obs_type {
                y_diagnostics::ObservationType::Generation => {
                    llm_iter += 1;
                    let model = obs.model.clone().unwrap_or_default();

                    // Extract prompt_preview from input JSON (full content, no truncation).
                    let prompt_preview = obs.input.to_string();

                    // Extract response_text from output JSON.
                    let response_text = obs
                        .output
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| obs.output.to_string());

                    entries.push((
                        ts,
                        HistoricalEntry::LlmResponse {
                            iteration: llm_iter,
                            model,
                            input_tokens: obs.input_tokens,
                            output_tokens: obs.output_tokens,
                            duration_ms,
                            cost_usd: obs.cost_usd,
                            tool_calls_requested: vec![],
                            prompt_preview,
                            response_text,
                            timestamp: ts.to_rfc3339(),
                        },
                    ));
                }
                y_diagnostics::ObservationType::ToolCall => {
                    let success = obs.status != y_diagnostics::ObservationStatus::Failed;
                    // Full output, no truncation.
                    let result_preview = obs.output.to_string();

                    entries.push((
                        ts,
                        HistoricalEntry::ToolResult {
                            name: obs.name.clone(),
                            success,
                            duration_ms,
                            result_preview,
                            timestamp: ts.to_rfc3339(),
                        },
                    ));
                }
                _ => {} // Skip UserMessage, spans, guardrails, etc.
            }
        }
    }

    // Sort all entries by timestamp ascending so the panel shows them in order.
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(entries.into_iter().map(|(_, e)| e).collect())
}
