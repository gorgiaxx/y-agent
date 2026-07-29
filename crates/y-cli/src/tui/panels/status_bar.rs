//! Status bar renderer.
//!
//! Single-line bar aligned with the GUI's `StatusBar.tsx` layout:
//!
//! ```text
//! [Left]                                        [Right]
//! session  mode  prompt  model  tokens/context (pct%)  $cost      / commands
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::panels::chat::SPINNER_FRAMES;
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
    let mut left_spans: Vec<Span> = vec![Span::styled(" ", Style::default())];

    // Always-on running indicator while a response streams.
    let running_spans = build_running_spans(state, t);
    if !running_spans.is_empty() {
        left_spans.extend(running_spans);
        left_spans.push(sep.clone());
    }

    // Session and orchestration mode replace the persistent session sidebar.
    left_spans.push(Span::styled(
        state.current_session_label(),
        Style::default().fg(t.text()).add_modifier(Modifier::BOLD),
    ));
    left_spans.push(sep.clone());
    left_spans.push(Span::styled(
        state.turn_mode.label(),
        Style::default().fg(t.input_border_focused()),
    ));
    left_spans.push(sep.clone());
    left_spans.push(build_prompt_status_span(state, t));
    if let Some(queue_span) = build_queue_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(queue_span);
    }
    if let Some(bg_span) = build_bg_task_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(bg_span);
    }
    if let Some(agents_span) = build_subagent_status_span(state, t) {
        left_spans.push(sep.clone());
        left_spans.push(agents_span);
    }
    left_spans.push(sep.clone());

    // Model name.
    let model_label = if state.status_model.is_empty() {
        "\u{2014}".to_string() // em dash
    } else {
        state.status_model.clone()
    };

    left_spans.push(Span::styled(
        model_label,
        Style::default().fg(t.status_model()),
    ));

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
    let right_str = format!("/ commands  v{} ", state.version);
    let right_len = UnicodeWidthStr::width(right_str.as_str());

    // Compute available width for left section.
    let total_width = area.width as usize;
    // Display width, not bytes: the bar holds multibyte chars (box-drawing,
    // em dash) that occupy a single cell but multiple bytes.
    let left_len: usize = left_spans.iter().map(Span::width).sum();

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
    let total_len: usize = spans.iter().map(Span::width).sum();
    if total_len > total_width && total_width > 3 {
        spans = truncate_spans(spans, total_width, t);
    }

    let line = Line::from(spans);
    // No background fill: a tinted bar that stops where the text stops (or
    // fights the terminal's own background) reads as a rendering bug. Plain
    // foreground colors keep the bar visually flat.
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}

fn build_prompt_status_span(state: &AppState, t: &Theme) -> Span<'static> {
    Span::styled(
        state.prompt_template_status.label(),
        Style::default().fg(t.active()),
    )
}

/// Build the always-on "running" segment shown while a response streams.
///
/// The spinner frame is driven by `state.tick_counter` (100 ms ticks) and uses
/// the same braille frames as the chat panel's streaming header marker.
/// Returns an empty vec when idle.
fn build_running_spans(state: &AppState, t: &Theme) -> Vec<Span<'static>> {
    if !state.is_streaming {
        return Vec::new();
    }
    let frame = SPINNER_FRAMES[(state.tick_counter as usize) % SPINNER_FRAMES.len()];
    vec![Span::styled(
        format!("{frame} running"),
        Style::default()
            .fg(t.streaming_dot())
            .add_modifier(Modifier::BOLD),
    )]
}

/// Build the follow-up queue depth segment (`queue: N`), visible only while
/// the service-side queue holds pending messages.
fn build_queue_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    let depth = state.follow_up_queue.len();
    if depth == 0 {
        return None;
    }
    Some(Span::styled(
        format!("queue: {depth}"),
        Style::default().fg(t.active()),
    ))
}

/// Build the background task count segment (`bg: N`), visible only while
/// background shell tasks are running.
fn build_bg_task_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    if state.bg_task_count == 0 {
        return None;
    }
    Some(Span::styled(
        format!("bg: {}", state.bg_task_count),
        Style::default().fg(t.active()),
    ))
}

/// Build the running subagent count segment (`agents: N`), visible only
/// while one or more subagents are active.
fn build_subagent_status_span(state: &AppState, t: &Theme) -> Option<Span<'static>> {
    if state.active_subagent_count == 0 {
        return None;
    }
    Some(Span::styled(
        format!("agents: {}", state.active_subagent_count),
        Style::default().fg(t.active()),
    ))
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

/// Truncate a span list to fit within `max_width` display columns, appending
/// an ellipsis if truncation occurs.
fn truncate_spans(spans: Vec<Span<'static>>, max_width: usize, t: &Theme) -> Vec<Span<'static>> {
    let max = max_width.saturating_sub(1);
    let mut acc = 0;
    let mut result: Vec<Span<'static>> = Vec::new();

    for span in spans {
        let span_width = span.width();
        if acc + span_width <= max {
            result.push(span);
            acc += span_width;
        } else {
            let remaining = max - acc;
            if remaining > 0 {
                let partial = take_display_width(&span.content, remaining);
                result.push(Span::styled(partial, span.style));
            }
            result.push(Span::styled("\u{2026}", Style::default().fg(t.muted())));
            break;
        }
    }

    result
}

/// Take the longest char prefix whose display width fits within `max_width`.
/// A wide char that would straddle the boundary is dropped entirely.
fn take_display_width(value: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        width += ch_width;
        result.push(ch);
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
        let state = AppState::new();
        let spans = build_context_spans(&state, &state.theme);
        assert!(spans.is_empty(), "no spans when window is 0 and no tokens");
    }

    #[test]
    fn test_prompt_template_status_span_is_always_visible() {
        let mut state = AppState::new();
        assert_eq!(
            build_prompt_status_span(&state, &state.theme)
                .content
                .as_ref(),
            "prompt:default"
        );

        state.prompt_template_status = crate::tui::state::PromptTemplateStatus::Template {
            id: "review".into(),
            name: "Reviewer".into(),
        };
        assert_eq!(
            build_prompt_status_span(&state, &state.theme)
                .content
                .as_ref(),
            "prompt:Reviewer"
        );
    }

    #[test]
    fn test_running_spans_hidden_when_idle() {
        let state = AppState::new();
        assert!(build_running_spans(&state, &state.theme).is_empty());
    }

    #[test]
    fn test_running_spans_show_spinner_frame_and_follow_tick() {
        let mut state = AppState::new();
        state.is_streaming = true;

        let spans = build_running_spans(&state, &state.theme);
        assert_eq!(spans.len(), 1);
        let text = spans[0].content.as_ref();
        assert!(text.contains("running"), "expected running label: {text}");
        assert!(
            text.contains(SPINNER_FRAMES[0]),
            "tick 0 must show the first frame: {text}"
        );

        // The animation tick drives the frame.
        state.tick_counter = 1;
        let spans = build_running_spans(&state, &state.theme);
        let text = spans[0].content.as_ref();
        assert!(text.contains(SPINNER_FRAMES[1]), "tick 1 frame: {text}");

        // Frame index wraps around the frame table.
        state.tick_counter = SPINNER_FRAMES.len() as u64;
        let spans = build_running_spans(&state, &state.theme);
        assert!(spans[0].content.contains(SPINNER_FRAMES[0]));
    }

    fn follow_up(text: &str) -> y_service::FollowUpMessage {
        y_service::FollowUpMessage {
            id: format!("fu-{text}"),
            text: text.to_string(),
            created_at: 0,
            status: y_service::FollowUpStatus::default(),
        }
    }

    #[test]
    fn test_queue_status_span_hidden_when_queue_empty() {
        let state = AppState::new();
        assert!(build_queue_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_queue_status_span_shows_pending_count() {
        let mut state = AppState::new();
        state.follow_up_queue.push(follow_up("one"));
        state.follow_up_queue.push(follow_up("two"));

        let span = build_queue_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "queue: 2");
    }

    #[test]
    fn test_bg_and_agents_spans_hidden_at_zero() {
        let state = AppState::new();
        assert!(build_bg_task_status_span(&state, &state.theme).is_none());
        assert!(build_subagent_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_bg_task_span_shows_running_count() {
        let mut state = AppState::new();
        state.bg_task_count = 2;

        let span = build_bg_task_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "bg: 2");
        // The subagent segment stays hidden while its count is zero.
        assert!(build_subagent_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_subagent_span_shows_running_count() {
        let mut state = AppState::new();
        state.active_subagent_count = 3;

        let span = build_subagent_status_span(&state, &state.theme).unwrap();
        assert_eq!(span.content.as_ref(), "agents: 3");
        // The background task segment stays hidden while its count is zero.
        assert!(build_bg_task_status_span(&state, &state.theme).is_none());
    }

    #[test]
    fn test_bg_and_agents_spans_shown_together() {
        let mut state = AppState::new();
        state.bg_task_count = 1;
        state.active_subagent_count = 4;

        assert_eq!(
            build_bg_task_status_span(&state, &state.theme)
                .unwrap()
                .content
                .as_ref(),
            "bg: 1"
        );
        assert_eq!(
            build_subagent_status_span(&state, &state.theme)
                .unwrap()
                .content
                .as_ref(),
            "agents: 4"
        );
    }

    #[test]
    fn test_build_context_spans_with_usage() {
        let mut state = AppState::new();
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
        let mut state = AppState::new();
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

    #[test]
    fn test_truncate_spans_keeps_multibyte_bar_that_fits() {
        let t = Theme::default();
        // 12 box-drawing chars: 12 display cells but 36 bytes. A byte-counted
        // implementation would wrongly truncate this.
        let bar: String = "\u{2501}".repeat(6) + &"\u{2500}".repeat(6);
        let spans = vec![Span::raw(bar.clone())];
        let result = truncate_spans(spans, 13, &t);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_ref(), bar.as_str());
    }

    #[test]
    fn test_truncate_spans_cuts_by_display_width() {
        let t = Theme::default();
        // Each CJK char is 2 cells wide; width budget 7 allows 3 chars (6
        // cells) plus the ellipsis.
        let spans = vec![Span::raw("你好世界再")];
        let result = truncate_spans(spans, 7, &t);
        let total_width: usize = result.iter().map(Span::width).sum();
        assert!(
            total_width <= 7,
            "truncated result exceeds width budget: {total_width}"
        );
        assert_eq!(result[0].content.as_ref(), "你好世");
        assert_eq!(result[1].content.as_ref(), "\u{2026}");
    }

    #[test]
    fn test_take_display_width_drops_straddling_wide_char() {
        // 3 ASCII chars (3 cells) + 1 CJK char (2 cells) = 5 cells; a budget
        // of 4 must drop the wide char instead of splitting it.
        assert_eq!(take_display_width("abc好de", 4), "abc");
        assert_eq!(take_display_width("abc好de", 5), "abc好");
        assert_eq!(take_display_width("ab", 10), "ab");
    }
}
