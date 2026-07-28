use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use super::picker::{truncate, visible_range, PickerItem, PickerState};
use crate::tui::state::SessionListItem;
use crate::tui::theme::Theme;

/// A session plus its precomputed lowercase search fields, built once at
/// load time so per-keystroke filtering only runs `contains`.
#[derive(Debug, Clone)]
struct SessionPickerEntry {
    session: SessionListItem,
    id_lower: String,
    title_lower: String,
}

impl SessionPickerEntry {
    fn new(session: SessionListItem) -> Self {
        Self {
            id_lower: session.id.to_ascii_lowercase(),
            title_lower: session.title.to_ascii_lowercase(),
            session,
        }
    }
}

impl PickerItem for SessionPickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.id_lower.contains(query_lower) || self.title_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionPickerState {
    core: PickerState<SessionPickerEntry>,
    current_session_id: Option<String>,
}

impl SessionPickerState {
    pub fn new(sessions: Vec<SessionListItem>, current_session_id: Option<&str>) -> Self {
        Self {
            core: PickerState::new(sessions.into_iter().map(SessionPickerEntry::new).collect()),
            current_session_id: current_session_id.map(str::to_string),
        }
    }

    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    pub fn selected_session(&self) -> Option<&SessionListItem> {
        self.core.selected_item().map(|entry| &entry.session)
    }

    pub fn selected_is_current(&self) -> bool {
        self.selected_session()
            .is_some_and(|session| self.current_session_id.as_deref() == Some(session.id.as_str()))
    }

    pub fn select_prev(&mut self) {
        self.core.select_prev();
    }

    pub fn select_next(&mut self) {
        self.core.select_next();
    }

    pub fn push_char(&mut self, character: char) {
        self.core.push_char(character);
    }

    pub fn pop_char(&mut self) {
        self.core.pop_char();
    }
}

pub fn render(frame: &mut Frame, area: Rect, picker: &SessionPickerState, theme: &Theme) {
    let area = picker_area(area);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" Resume a previous session ")
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(7),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Search: ", Style::default().fg(theme.muted())),
            Span::styled(picker.core.query(), Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.input_border_focused())),
        ])),
        rows[0],
    );

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        rows[1].height as usize,
    );
    let items: Vec<ListItem> = visible
        .map(|position| {
            let session = &picker.core.items()[picker.core.filtered()[position]].session;
            let selected = position == picker.core.selected();
            let current = if picker.current_session_id.as_deref() == Some(session.id.as_str()) {
                " current"
            } else {
                ""
            };
            let title = if session.title.trim().is_empty() {
                "Untitled session"
            } else {
                session.title.as_str()
            };
            let short_id: String = session.id.chars().take(8).collect();
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text())
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    " {}  {:<28} {:>4} msgs  {short_id}{current}",
                    session.updated_at.format("%Y-%m-%d %H:%M"),
                    truncate(title, 28),
                    session.message_count,
                ),
                style,
            )))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(" No matching sessions").style(Style::default().fg(theme.muted())),
            rows[1],
        );
    } else {
        frame.render_widget(List::new(items), rows[1]);
    }

    let details = picker.selected_session().map_or_else(
        || "No session selected".to_string(),
        |session| {
            let title = if session.title.trim().is_empty() {
                "Untitled session"
            } else {
                session.title.as_str()
            };
            format!(
                "Title: {title}\nSession: {}\nUpdated: {}\nMessages: {}{}",
                session.id,
                session.updated_at.format("%Y-%m-%d %H:%M:%S UTC"),
                session.message_count,
                if picker.selected_is_current() {
                    "\nStatus: current session"
                } else {
                    ""
                }
            )
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().fg(theme.text()))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted()))
                    .title(" Session details "),
            ),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(" Up/Down navigate  Type to search  Enter resume  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[3],
    );
}

fn picker_area(area: Rect) -> Rect {
    area
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn session(id: &str, title: &str) -> SessionListItem {
        SessionListItem {
            id: id.into(),
            title: title.into(),
            updated_at: Utc::now(),
            message_count: 4,
        }
    }

    #[test]
    fn test_session_picker_searches_title_and_id() {
        let mut picker = SessionPickerState::new(
            vec![
                session("alpha-1234", "Release work"),
                session("beta-5678", "TUI redesign"),
            ],
            None,
        );

        for character in "beta".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_session().unwrap().title, "TUI redesign");
    }

    #[test]
    fn test_session_picker_marks_current_session() {
        let picker = SessionPickerState::new(
            vec![session("alpha-1234", "Release work")],
            Some("alpha-1234"),
        );

        assert!(picker.selected_is_current());
    }

    #[test]
    fn test_session_picker_uses_entire_terminal_area() {
        let area = Rect::new(2, 1, 100, 35);
        assert_eq!(picker_area(area), area);
    }

    #[test]
    fn test_session_picker_matches_case_insensitively() {
        let mut picker = SessionPickerState::new(
            vec![
                session("alpha-1234", "Release work"),
                session("beta-5678", "TUI redesign"),
            ],
            None,
        );

        for character in "RELEASE".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_session().unwrap().id, "alpha-1234");
    }

    #[test]
    fn test_session_picker_non_ascii_query_does_not_panic() {
        let mut picker = SessionPickerState::new(
            vec![
                session("alpha-1234", "发布工作"),
                session("beta-5678", "TUI redesign"),
            ],
            None,
        );

        for character in "发布".chars() {
            picker.push_char(character);
        }

        assert_eq!(picker.filtered_len(), 1);
        assert_eq!(picker.selected_session().unwrap().id, "alpha-1234");

        picker.pop_char();
        picker.pop_char();
        assert_eq!(picker.filtered_len(), 2);
    }
}
