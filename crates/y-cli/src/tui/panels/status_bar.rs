//! Status bar renderer.
//!
//! Single-line bar showing model, token usage, context window utilization, and
//! connection state.  Data is pulled from `AppState` (populated by the chat
//! flow after each LLM response).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::state::AppState;

/// Render the status bar into the given area using live data from `AppState`.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let sep = Span::styled(" │ ", Style::default().fg(Color::DarkGray));

    // Model name.
    let model_label = if state.status_model.is_empty() {
        "—".to_string()
    } else {
        state.status_model.clone()
    };

    // Token usage string (e.g. "10↑ 5↓").
    let tokens_label = if state.status_tokens.is_empty() {
        "0tok".to_string()
    } else {
        state.status_tokens.clone()
    };

    // Context window usage bar & label.
    let ctx_spans = build_context_spans(state);

    let mut spans = vec![
        Span::styled(" ", Style::default()),
        Span::styled(model_label, Style::default().fg(Color::Magenta)),
        sep.clone(),
        Span::styled(tokens_label, Style::default().fg(Color::DarkGray)),
        sep.clone(),
    ];
    spans.extend(ctx_spans);

    // Truncate with ellipsis if too wide.
    let total_len: usize = spans.iter().map(|s| s.content.len()).sum();
    if total_len > area.width as usize && area.width > 3 {
        let max = area.width as usize - 1;
        let mut acc = 0;
        let mut truncated_spans: Vec<Span> = Vec::new();
        for span in &spans {
            let slen = span.content.len();
            if acc + slen <= max {
                truncated_spans.push(span.clone());
                acc += slen;
            } else {
                let remaining = max - acc;
                if remaining > 0 {
                    let partial: String = span.content.chars().take(remaining).collect();
                    truncated_spans.push(Span::styled(partial, span.style));
                }
                truncated_spans.push(Span::styled("…", Style::default().fg(Color::DarkGray)));
                break;
            }
        }
        spans = truncated_spans;
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(Color::Rgb(30, 30, 40)));

    frame.render_widget(para, area);
}

/// Build styled spans for the context window usage indicator.
///
/// Format: `█████░░░░░ 52% (65k/128k)`
///
/// Color coding:
/// - Green  : < 50%
/// - Yellow : 50–80%
/// - Red    : > 80%
fn build_context_spans(state: &AppState) -> Vec<Span<'static>> {
    if state.context_window == 0 {
        return vec![Span::styled("ctx: —", Style::default().fg(Color::DarkGray))];
    }

    let pct = state.context_usage_percent().min(100.0);
    let bar_width = 10usize;
    let filled = ((pct / 100.0) * bar_width as f32).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_color = if pct < 50.0 {
        Color::Green
    } else if pct < 80.0 {
        Color::Yellow
    } else {
        Color::Red
    };

    let filled_str: String = "█".repeat(filled);
    let empty_str: String = "░".repeat(empty);

    let used_k = format_token_count(state.last_input_tokens);
    let total_k = format_token_count(state.context_window as u64);
    let label = format!(" {pct:.0}% ({used_k}/{total_k})");

    vec![
        Span::styled(
            filled_str,
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(empty_str, Style::default().fg(Color::DarkGray)),
        Span::styled(label, Style::default().fg(bar_color)),
    ]
}

/// Format a token count for compact display: e.g. 128000 → "128k", 1500 → "1.5k".
fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        if count.is_multiple_of(1_000_000) {
            let val = count / 1_000_000;
            format!("{val}M")
        } else {
            let m = count as f64 / 1_000_000.0;
            format!("{m:.1}M")
        }
    } else if count >= 1_000 {
        if count.is_multiple_of(1_000) {
            let val = count / 1_000;
            format!("{val}k")
        } else {
            let k = count as f64 / 1_000.0;
            format!("{k:.1}k")
        }
    } else {
        format!("{count}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_token_count_small() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn test_format_token_count_thousands() {
        assert_eq!(format_token_count(1_000), "1k");
        assert_eq!(format_token_count(1_500), "1.5k");
        assert_eq!(format_token_count(128_000), "128k");
        assert_eq!(format_token_count(200_000), "200k");
    }

    #[test]
    fn test_format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }

    #[test]
    fn test_build_context_spans_zero_window() {
        let state = AppState::default();
        let spans = build_context_spans(&state);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].content.contains("—"));
    }

    #[test]
    fn test_build_context_spans_with_usage() {
        let mut state = AppState::default();
        state.context_window = 128_000;
        state.last_input_tokens = 64_000;
        let spans = build_context_spans(&state);
        // Should have 3 spans: filled bar, empty bar, label.
        assert_eq!(spans.len(), 3);
        // Label should contain percentage and token counts.
        let label = &spans[2].content;
        assert!(label.contains("50%"), "expected 50% in label, got: {label}");
        assert!(label.contains("64k"), "expected 64k in label, got: {label}");
        assert!(
            label.contains("128k"),
            "expected 128k in label, got: {label}"
        );
    }

    #[test]
    fn test_context_color_coding() {
        let mut state = AppState::default();
        state.context_window = 100;

        // < 50% -> green
        state.last_input_tokens = 30;
        let spans = build_context_spans(&state);
        assert_eq!(spans[0].style.fg, Some(Color::Green));

        // 50-80% -> yellow
        state.last_input_tokens = 60;
        let spans = build_context_spans(&state);
        assert_eq!(spans[0].style.fg, Some(Color::Yellow));

        // > 80% -> red
        state.last_input_tokens = 90;
        let spans = build_context_spans(&state);
        assert_eq!(spans[0].style.fg, Some(Color::Red));
    }
}
