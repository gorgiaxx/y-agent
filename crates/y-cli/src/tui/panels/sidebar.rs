//! Sidebar panel renderer.
//!
//! Shows a tabbed view of sessions or agents with list navigation.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Tabs};
use ratatui::Frame;

use crate::tui::state::{AppState, PanelFocus, SidebarView};

/// Render the sidebar panel into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focus == PanelFocus::Sidebar;

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    // Split sidebar: tabs (1 line at top) + list (remaining).
    // We'll render everything inside the block manually.
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 {
        return; // Too small for sidebar content.
    }

    // Tab bar.
    let tab_titles = vec!["Sessions", "Agents"];
    let selected = match state.sidebar_view {
        SidebarView::Sessions => 0,
        SidebarView::Agents => 1,
    };

    let tabs = Tabs::new(tab_titles)
        .select(selected)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider("│");

    let tab_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(tabs, tab_area);

    // List area.
    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };

    let visible_height = list_area.height as usize;

    let items: Vec<ListItem> = match state.sidebar_view {
        SidebarView::Sessions => {
            if state.sessions.is_empty() {
                vec![ListItem::new(Line::from(Span::styled(
                    "  No sessions yet",
                    Style::default().fg(Color::DarkGray),
                )))]
            } else {
                // Compute scroll offset to keep the selected item visible.
                let selected_idx = state.selected_session_index.unwrap_or(0);
                let scroll_offset = if visible_height == 0 {
                    0
                } else if selected_idx >= visible_height {
                    // Selected item is beyond the visible window — scroll down.
                    selected_idx - visible_height + 1
                } else {
                    0
                };

                state
                    .sessions
                    .iter()
                    .enumerate()
                    .skip(scroll_offset)
                    .take(visible_height)
                    .map(|(i, s)| {
                        let is_selected = state.selected_session_index == Some(i);
                        let is_current = state.current_session_id.as_deref() == Some(s.id.as_str());

                        // Build display label: title or truncated ID.
                        // Use char-boundary-aware truncation to avoid panics
                        // on multi-byte UTF-8 (e.g., CJK characters).
                        let label = if s.title.is_empty() {
                            // Show first 8 chars of ID.
                            let short: String = s.id.chars().take(8).collect();
                            if s.id.chars().count() > 8 {
                                format!("{short}…")
                            } else {
                                short
                            }
                        } else {
                            // Truncate title to fit sidebar width.
                            let max_chars = inner.width.saturating_sub(5) as usize;
                            let char_count = s.title.chars().count();
                            if char_count > max_chars {
                                let truncated: String =
                                    s.title.chars().take(max_chars.saturating_sub(1)).collect();
                                format!("{truncated}…")
                            } else {
                                s.title.clone()
                            }
                        };

                        // Active indicator.
                        let prefix = if is_current { "● " } else { "  " };
                        let suffix = format!(" ({})", s.message_count);

                        let style = if is_selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else if is_current {
                            Style::default().fg(Color::Green)
                        } else {
                            Style::default().fg(Color::White)
                        };

                        ListItem::new(Line::from(vec![
                            Span::styled(prefix, style),
                            Span::styled(label, style),
                            Span::styled(suffix, Style::default().fg(Color::DarkGray)),
                        ]))
                    })
                    .collect()
            }
        }
        SidebarView::Agents => {
            vec![ListItem::new(Line::from(Span::styled(
                "  No agents configured",
                Style::default().fg(Color::DarkGray),
            )))]
        }
    };

    let list = List::new(items).style(Style::default().fg(Color::White));

    frame.render_widget(list, list_area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T-TUI-02-05: Sidebar tab switch between Sessions and Agents.
    #[test]
    fn test_sidebar_view_tab_index() {
        let state_sessions = AppState::new(); // default is Sessions

        let mut state_agents = AppState::new();
        state_agents.toggle_sidebar_view(); // Sessions → Agents

        // Sessions is tab 0.
        let idx_sessions = match state_sessions.sidebar_view {
            SidebarView::Sessions => 0,
            SidebarView::Agents => 1,
        };
        assert_eq!(idx_sessions, 0);

        // Agents is tab 1.
        let idx_agents = match state_agents.sidebar_view {
            SidebarView::Sessions => 0,
            SidebarView::Agents => 1,
        };
        assert_eq!(idx_agents, 1);
    }
}
