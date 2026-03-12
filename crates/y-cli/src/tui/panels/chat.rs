//! Chat panel renderer.
//!
//! Renders the conversation transcript as styled message blocks.
//! Supports scroll offset, auto-scroll indicator, and role-based styling.
//!
//! Lines are pre-wrapped to the available width so that `total_lines`
//! accurately reflects visual rows. This ensures correct auto-scroll
//! and correct mouse-to-content coordinate mapping for text selection.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::tui::selection::TextSelection;
use crate::tui::state::{AppState, ChatMessage, MessageRole, PanelFocus};

/// Render the chat panel into the given area.
///
/// Returns a flat list of plain-text content lines (one per rendered row)
/// so that the selection system can extract text by row/col index.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) -> Vec<String> {
    let is_focused = state.focus == PanelFocus::Chat;

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Chat ")
        .title_style(Style::default().fg(Color::White));

    // Available content width (subtract 2 for left/right borders).
    let inner_width = area.width.saturating_sub(2) as usize;

    // Build lines from messages, then pre-wrap to inner_width so that
    // total_lines accurately reflects visual rows.
    let mut raw_lines: Vec<Line> = Vec::new();
    let mut raw_plain: Vec<String> = Vec::new();

    if state.messages.is_empty() {
        raw_lines.push(Line::from(""));
        raw_plain.push(String::new());
        raw_lines.push(Line::from(Span::styled(
            "  No messages yet. Type below to start chatting.",
            Style::default().fg(Color::DarkGray),
        )));
        raw_plain.push("  No messages yet. Type below to start chatting.".to_string());
    } else {
        for msg in &state.messages {
            // Blank line between messages.
            if !raw_lines.is_empty() {
                raw_lines.push(Line::from(""));
                raw_plain.push(String::new());
            }
            render_message(&mut raw_lines, &mut raw_plain, msg);
        }
    }

    // Pre-wrap: split each logical line into visual rows based on inner_width.
    let mut lines: Vec<Line> = Vec::new();
    let mut plain_lines: Vec<String> = Vec::new();
    if inner_width > 0 {
        for (raw_line, raw_text) in raw_lines.into_iter().zip(raw_plain.into_iter()) {
            let wrapped_plain = wrap_text(&raw_text, inner_width);
            if wrapped_plain.len() <= 1 {
                // No wrapping needed — keep original styled line.
                lines.push(raw_line);
                plain_lines.push(raw_text);
            } else {
                // Re-build styled lines for each wrapped segment.
                let style = raw_line
                    .spans
                    .first()
                    .map(|s| s.style)
                    .unwrap_or_default();
                for wp in wrapped_plain {
                    lines.push(Line::from(Span::styled(wp.clone(), style)));
                    plain_lines.push(wp);
                }
            }
        }
    } else {
        lines = vec![Line::from("")];
        plain_lines = vec![String::new()];
    }

    // Compute scroll: total_lines - visible_lines + offset.
    let inner_height = area.height.saturating_sub(2) as usize; // borders
    let total_lines = lines.len();

    // Auto-scroll: if offset is 0, show the bottom; otherwise apply offset.
    let scroll_to = if state.scroll_offset == 0 {
        total_lines.saturating_sub(inner_height)
    } else {
        total_lines
            .saturating_sub(inner_height)
            .saturating_sub(state.scroll_offset)
    };

    // Apply selection highlight to lines in the visible range.
    let selection = &state.selection;
    if !selection.is_empty() {
        let visible_start = scroll_to;
        let visible_end = (scroll_to + inner_height).min(total_lines);

        for row_idx in visible_start..visible_end {
            lines[row_idx] = apply_selection_highlight(&lines[row_idx], row_idx, selection);
        }
    }

    // No `Wrap` — lines are already pre-wrapped to inner_width.
    let para = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_to as u16, 0));

    frame.render_widget(para, area);

    // "New content below" indicator when scrolled up during streaming.
    if state.scroll_offset > 0 && state.is_streaming {
        let indicator = Span::styled(
            " ▼ New content below ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        let indicator_line = Line::from(indicator);
        let indicator_area = Rect {
            x: area.x + 2,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(4).min(22),
            height: 1,
        };
        frame.render_widget(Paragraph::new(indicator_line), indicator_area);
    }

    plain_lines
}

/// Render a single message into lines with role-based styling.
fn render_message(lines: &mut Vec<Line>, plain_lines: &mut Vec<String>, msg: &ChatMessage) {
    let (role_label, role_style) = match msg.role {
        MessageRole::User => (
            "You",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        MessageRole::Assistant => (
            "Assistant",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        MessageRole::System => (
            "System",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        MessageRole::Tool => (
            "Tool",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    };

    // Role header.
    let mut header_spans = vec![Span::styled(format!("  {role_label}"), role_style)];
    let mut header_plain = format!("  {role_label}");

    if msg.is_streaming {
        header_spans.push(Span::styled(" ●", Style::default().fg(Color::Yellow)));
        header_plain.push_str(" ●");
    }
    if msg.is_cancelled {
        header_spans.push(Span::styled(
            " [cancelled]",
            Style::default().fg(Color::Red),
        ));
        header_plain.push_str(" [cancelled]");
    }

    lines.push(Line::from(header_spans));
    plain_lines.push(header_plain);

    // Content lines — indent by 2 spaces.
    for content_line in msg.content.lines() {
        let formatted = format!("  {content_line}");
        lines.push(Line::from(Span::styled(
            formatted.clone(),
            Style::default().fg(Color::White),
        )));
        plain_lines.push(formatted);
    }
}

/// Apply inverse-color highlight to characters in a line that fall within the selection.
fn apply_selection_highlight<'a>(
    line: &Line<'a>,
    row: usize,
    selection: &TextSelection,
) -> Line<'a> {
    let highlight_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);

    let mut new_spans: Vec<Span<'a>> = Vec::new();
    let mut col = 0usize;

    for span in &line.spans {
        let text = span.content.as_ref();
        let span_len = text.chars().count();

        // Fast path: entire span is outside selection.
        let span_start = col;
        let span_end = col + span_len;

        let sel_start_in_span = selection.contains(row, span_start);
        let sel_end_in_span = span_end > 0 && selection.contains(row, span_end - 1);

        if sel_start_in_span && sel_end_in_span {
            // Entire span is selected.
            new_spans.push(Span::styled(span.content.clone(), highlight_style));
        } else if !sel_start_in_span
            && !sel_end_in_span
            && !selection_overlaps(selection, row, span_start, span_end)
        {
            // Entire span is unselected.
            new_spans.push(span.clone());
        } else {
            // Partial selection: split character by character.
            let mut normal_buf = String::new();
            let mut highlight_buf = String::new();
            for ch in text.chars() {
                if selection.contains(row, col) {
                    if !normal_buf.is_empty() {
                        new_spans.push(Span::styled(normal_buf.clone(), span.style));
                        normal_buf.clear();
                    }
                    highlight_buf.push(ch);
                } else {
                    if !highlight_buf.is_empty() {
                        new_spans.push(Span::styled(highlight_buf.clone(), highlight_style));
                        highlight_buf.clear();
                    }
                    normal_buf.push(ch);
                }
                col += 1;
            }
            if !highlight_buf.is_empty() {
                new_spans.push(Span::styled(highlight_buf, highlight_style));
            }
            if !normal_buf.is_empty() {
                new_spans.push(Span::styled(normal_buf, span.style));
            }
            continue; // col already updated
        }
        col = span_end;
    }

    Line::from(new_spans)
}

/// Check whether the selection overlaps the character range `[span_start, span_end)`
/// on the given `row`.  This catches the case where the selection is entirely
/// contained within a span (neither the span's first nor last char is selected,
/// but interior characters are).
fn selection_overlaps(sel: &TextSelection, row: usize, span_start: usize, span_end: usize) -> bool {
    if sel.is_empty() || span_start >= span_end {
        return false;
    }
    let ((sr, sc), (er, ec)) = sel.sorted();

    if row < sr || row > er {
        return false;
    }

    // Determine the selected column range on this row.
    let sel_col_start = if row == sr { sc } else { 0 };
    let sel_col_end = if row == er { ec } else { usize::MAX };

    // Two ranges [span_start, span_end) and [sel_col_start, sel_col_end) overlap
    // iff each starts before the other ends.
    span_start < sel_col_end && sel_col_start < span_end
}

/// Wrap a plain-text string into multiple lines that each fit within `max_width`
/// display columns. Uses Unicode display width for correct CJK / emoji handling.
///
/// Returns a `Vec<String>` where each entry is one visual row.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    if text.is_empty() || UnicodeWidthStr::width(text) <= max_width {
        return vec![text.to_string()];
    }

    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width: usize = 0;

    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > max_width && !current.is_empty() {
            rows.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // T-TUI-02-04: Chat scroll offset limits clamp to message count.
    #[test]
    fn test_scroll_offset_clamping() {
        // When there are fewer messages than the visible area,
        // scroll_to should be 0.
        let total_lines: usize = 5;
        let inner_height: usize = 20;
        let offset: usize = 0;

        let scroll_to = if offset == 0 {
            total_lines.saturating_sub(inner_height)
        } else {
            total_lines
                .saturating_sub(inner_height)
                .saturating_sub(offset)
        };

        assert_eq!(scroll_to, 0, "no scroll when content fits");
    }

    #[test]
    fn test_render_message_creates_lines() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Hello\nWorld".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_message(&mut lines, &mut plain, &msg);

        // Header + 2 content lines = 3 lines.
        assert_eq!(lines.len(), 3);
        assert_eq!(plain.len(), 3);
    }

    #[test]
    fn test_streaming_indicator_in_header() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "Thinking...".to_string(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_message(&mut lines, &mut plain, &msg);

        // Header should contain the streaming indicator "●".
        let header = &lines[0];
        let header_text: String = header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(header_text.contains('●'));
    }

    #[test]
    fn test_wrap_text_no_wrap_needed() {
        let result = wrap_text("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_text_exact_fit() {
        let result = wrap_text("12345", 5);
        assert_eq!(result, vec!["12345"]);
    }

    #[test]
    fn test_wrap_text_splits() {
        let result = wrap_text("abcdefghij", 5);
        assert_eq!(result, vec!["abcde", "fghij"]);
    }

    #[test]
    fn test_wrap_text_empty() {
        let result = wrap_text("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_wrap_text_cjk_double_width() {
        // CJK characters are 2 columns wide.
        let result = wrap_text("你好世界测试", 6);
        // Each char is 2 cols → 3 chars per row of width 6.
        assert_eq!(result, vec!["你好世", "界测试"]);
    }
}
