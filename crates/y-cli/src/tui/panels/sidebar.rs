//! Sidebar panel renderer.
//!
//! Shows a sessions-only list with navigation and "New Session" action,
//! aligned with the GUI sidebar layout.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::state::{AppState, PanelFocus};

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

const COLOR_BORDER_FOCUSED: Color = Color::Rgb(120, 180, 255);
const COLOR_BORDER_UNFOCUSED: Color = Color::Rgb(50, 50, 65);
const COLOR_TITLE: Color = Color::Rgb(180, 180, 200);
const COLOR_SELECTED: Color = Color::Rgb(120, 180, 255);
const COLOR_ACTIVE: Color = Color::Rgb(130, 220, 130);
const COLOR_NORMAL: Color = Color::Rgb(180, 180, 200);
const COLOR_MUTED: Color = Color::Rgb(90, 90, 110);
const COLOR_NEW_SESSION: Color = Color::Rgb(100, 160, 255);
const COLOR_EMPTY: Color = Color::Rgb(80, 80, 100);
const COLOR_PANEL_BG: Color = Color::Rgb(22, 22, 30);

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the sidebar panel into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == PanelFocus::Sidebar;

    let border_style = if is_focused {
        Style::default().fg(COLOR_BORDER_FOCUSED)
    } else {
        Style::default().fg(COLOR_BORDER_UNFOCUSED)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Sessions ")
        .title_style(Style::default().fg(COLOR_TITLE));

    let inner = block.inner(area);
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(COLOR_PANEL_BG)),
        area,
    );
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    // "New Session" action at the top (always visible).
    let new_session_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let new_session_line = Line::from(vec![
        Span::styled("  + ", Style::default().fg(COLOR_NEW_SESSION)),
        Span::styled(
            "New Session",
            Style::default()
                .fg(COLOR_NEW_SESSION)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(new_session_line), new_session_area);

    // Separator line.
    if inner.height < 3 {
        return;
    }
    let sep_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 1,
    };
    let sep_char: String = "\u{2500}".repeat(inner.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            sep_char,
            Style::default().fg(COLOR_BORDER_UNFOCUSED),
        ))),
        sep_area,
    );

    // Session list area.
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };

    let visible_height = list_area.height as usize;

    if state.sessions.is_empty() {
        let items = vec![ListItem::new(Line::from(Span::styled(
            "  No sessions yet",
            Style::default().fg(COLOR_EMPTY),
        )))];
        let list = List::new(items);
        frame.render_widget(list, list_area);
        return;
    }

    // Compute scroll offset to keep the selected item visible.
    let selected_idx = state.selected_session_index.unwrap_or(0);
    let scroll_offset = if visible_height == 0 {
        0
    } else if selected_idx >= visible_height {
        selected_idx - visible_height + 1
    } else {
        0
    };

    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, s)| {
            let is_selected = state.selected_session_index == Some(i);
            let is_current = state.current_session_id.as_deref() == Some(s.id.as_str());

            // Build display label: title or truncated ID.
            let label = if s.title.is_empty() {
                let short: String = s.id.chars().take(8).collect();
                if s.id.chars().count() > 8 {
                    format!("{short}\u{2026}")
                } else {
                    short
                }
            } else {
                let max_chars = inner.width.saturating_sub(6) as usize;
                let char_count = s.title.chars().count();
                if char_count > max_chars {
                    let truncated: String =
                        s.title.chars().take(max_chars.saturating_sub(1)).collect();
                    format!("{truncated}\u{2026}")
                } else {
                    s.title.clone()
                }
            };

            // Active indicator.
            let prefix = if is_current { " * " } else { "   " };
            let suffix = format!(" ({})", s.message_count);

            let style = if is_selected {
                Style::default()
                    .fg(COLOR_SELECTED)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(COLOR_ACTIVE)
            } else {
                Style::default().fg(COLOR_NORMAL)
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(label, style),
                Span::styled(suffix, Style::default().fg(COLOR_MUTED)),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, list_area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::SidebarView;

    // T-TUI-02-05: Sidebar view is sessions-only.
    #[test]
    fn test_sidebar_view_is_sessions() {
        let state = AppState::new();
        assert_eq!(state.sidebar_view, SidebarView::Sessions);
    }
}
