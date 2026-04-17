//! Status bar renderer.
//!
//! Single-line bar aligned with the GUI's `StatusBar.tsx` layout:
//!
//! ```text
//! [Left]                                        [Right]
//! model_name  tokens/context (pct%)  $cost      v0.x.x
//!             [=========-------]
//! ```
//!
//! Data is pulled from `AppState` (populated by the chat flow after each
//! LLM response).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::state::AppState;
use crate::tui::theme::Theme;

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the status bar into the given area using live data from `AppState`.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = &state.theme;
    let sep = Span::styled(" | ", Style::default().fg(t.status_sep()));

    // -- Left section --

    // Model name.
    let model_label = if state.status_model.is_empty() {
        "\u{2014}".to_string() // em dash
    } else {
        state.status_model.clone()
    };

    let mut left_spans: Vec<Span> = vec![
        Span::styled(" ", Style::default()),
        Span::styled(model_label, Style::default().fg(t.status_model())),
    ];

    // Context window usage (tokens/window + pct + bar).
    let ctx_spans = build_context_spans(state, t);
    if !ctx_spans.is_empty() {
        left_spans.push(sep.clone());
        left_spans.extend(ctx_spans);
    }

    // Cost.
    if let Some(cost) = state.last_cost {
        if cost > 0.0 {
            left_spans.push(sep.clone());
            left_spans.push(Span::styled(
                format!("${cost:.4}"),
                Style::default().fg(t.status_cost()),
            ));
        }
    }

    // -- Right section --
    let right_str = format!("v{} ", state.version);
    let right_len = right_str.len();

    // Compute available width for left section.
    let total_width = area.width as usize;
    let left_len: usize = left_spans.iter().map(|s| s.content.len()).sum();

    // Fill gap between left and right.
    let gap = total_width.saturating_sub(left_len + right_len);

    let mut spans = left_spans;
    if gap > 0 {
        spans.push(Span::styled(" ".repeat(gap), Style::default()));
    }
    spans.push(Span::styled(
        right_str,
        Style::default().fg(t.status_version()),
    ));

    // Truncate if too wide.
    let total_len: usize = spans.iter().map(|s| s.content.len()).sum();
    if total_len > total_width && total_width > 3 {
        spans = truncate_spans(spans, total_width, t);
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(t.status_bg()));
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Context window bar (aligned with GUI status-token-bar)
// ---------------------------------------------------------------------------

/// Build styled spans for the context window usage indicator.
///
/// Format: `tokens/window (pct%) [=========-------]`
///
/// Color coding:
/// - Normal (accent) : < 80%
/// - Warning (yellow) : >= 80%
fn build_context_spans(state: &AppState, t: &Theme) -> Vec<Span<'static>> {
    if state.context_window == 0 {
        if state.status_tokens.is_empty() {
            return vec![];
        }
        return vec![Span::styled(
            state.status_tokens.clone(),
            Style::default().fg(t.muted()),
        )];
    }

    let occupancy = state.last_input_tokens;
    if occupancy == 0 {
        return vec![];
    }

    let ctx_window = state.context_window as u64;
    let pct = ((occupancy as f64 / ctx_window as f64) * 100.0).min(100.0);
    let bar_width = 12usize;
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_color = if pct >= 80.0 {
        t.status_bar_warn()
    } else {
        t.status_bar_normal()
    };

    let filled_str: String = "\u{2501}".repeat(filled);
    let empty_str: String = "\u{2500}".repeat(empty);

    let used_label = format_token_count(occupancy);
    let total_label = format_token_count(ctx_window);

    vec![
        Span::styled(
            format!("{used_label}/{total_label}"),
            Style::default().fg(t.status_token_ratio()),
        ),
        Span::styled(format!(" ({pct:.1}%) "), Style::default().fg(t.muted())),
        Span::styled(
            filled_str,
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(empty_str, Style::default().fg(t.status_bar_track())),
    ]
}

/// Truncate a span list to fit within `max_width` characters, appending an
/// ellipsis if truncation occurs.
fn truncate_spans(spans: Vec<Span<'static>>, max_width: usize, t: &Theme) -> Vec<Span<'static>> {
    let max = max_width.saturating_sub(1);
    let mut acc = 0;
    let mut result: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let slen = span.content.len();
        if acc + slen <= max {
            result.push(span);
            acc += slen;
        } else {
            let remaining = max - acc;
            if remaining > 0 {
                let partial: String = span.content.chars().take(remaining).collect();
                result.push(Span::styled(partial, span.style));
            }
            result.push(Span::styled("\u{2026}", Style::default().fg(t.muted())));
            break;
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Token formatting (aligned with GUI formatTokens)
// ---------------------------------------------------------------------------

/// Format a token count for compact display: e.g. 128000 -> "128k", 1500 -> "1.5k".
fn format_token_count(count: u64) -> String {
    if count >= 1_000_000 {
        let m = count as f64 / 1_000_000.0;
        if (m - m.round()).abs() < 0.05 {
            format!("{}M", m.round() as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if count >= 1_000 {
        let k = count as f64 / 1_000.0;
        if (k - k.round()).abs() < 0.05 {
            format!("{}k", k.round() as u64)
        } else {
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
        let spans = build_context_spans(&state, &state.theme);
        assert!(spans.is_empty(), "no spans when window is 0 and no tokens");
    }

    #[test]
    fn test_build_context_spans_with_usage() {
        let mut state = AppState::default();
        state.context_window = 128_000;
        state.last_input_tokens = 64_000;
        let spans = build_context_spans(&state, &state.theme);
        // Should have 4 spans: ratio, pct, filled bar, empty bar.
        assert_eq!(spans.len(), 4);
        // Ratio should contain token counts.
        let ratio = &spans[0].content;
        assert!(ratio.contains("64k"), "expected 64k in ratio, got: {ratio}");
        assert!(
            ratio.contains("128k"),
            "expected 128k in ratio, got: {ratio}"
        );
        // Pct should contain percentage.
        let pct_span = &spans[1].content;
        assert!(
            pct_span.contains("50.0%"),
            "expected 50.0% in pct, got: {pct_span}"
        );
    }

    #[test]
    fn test_context_color_coding() {
        let mut state = AppState::default();
        state.context_window = 100;
        let t = &state.theme;

        // < 80% -> normal accent color
        state.last_input_tokens = 30;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.status_bar_normal()));

        // >= 80% -> warning color
        state.last_input_tokens = 85;
        let spans = build_context_spans(&state, t);
        assert_eq!(spans[2].style.fg, Some(t.status_bar_warn()));
    }

    #[test]
    fn test_truncate_spans_short() {
        let t = Theme::default();
        let spans = vec![Span::raw("hello")];
        let result = truncate_spans(spans, 10, &t);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_ref(), "hello");
    }

    #[test]
    fn test_truncate_spans_overflow() {
        let t = Theme::default();
        let spans = vec![Span::raw("hello world this is long")];
        let result = truncate_spans(spans, 10, &t);
        // Should truncate and add ellipsis.
        assert!(result.len() >= 2);
        let total_chars: usize = result.iter().map(|s| s.content.chars().count()).sum();
        assert!(
            total_chars <= 10,
            "truncated result too long: {total_chars} chars",
        );
    }
}
