use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::state::SessionListItem;
use crate::tui::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct SessionPickerState {
    sessions: Vec<SessionListItem>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
    current_session_id: Option<String>,
}

impl SessionPickerState {
    pub fn new(sessions: Vec<SessionListItem>, current_session_id: Option<&str>) -> Self {
        let filtered = (0..sessions.len()).collect();
        Self {
            sessions,
            filtered,
            selected: 0,
            query: String::new(),
            current_session_id: current_session_id.map(str::to_string),
        }
    }

    pub fn filtered_len(&self) -> usize {
        self.filtered.len()
    }

    pub fn selected_session(&self) -> Option<&SessionListItem> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.sessions.get(*index))
    }

    pub fn selected_is_current(&self) -> bool {
        self.selected_session()
            .is_some_and(|session| self.current_session_id.as_deref() == Some(session.id.as_str()))
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub fn push_char(&mut self, character: char) {
        self.query.push(character);
        self.update_filter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.update_filter();
    }

    fn update_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                query.is_empty()
                    || session.id.to_ascii_lowercase().contains(&query)
                    || session.title.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
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
            Span::styled(&picker.query, Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.input_border_focused())),
        ])),
        rows[0],
    );

    let visible = visible_range(
        picker.filtered.len(),
        picker.selected,
        rows[1].height as usize,
    );
    let items: Vec<ListItem> = visible
        .map(|position| {
            let session = &picker.sessions[picker.filtered[position]];
            let selected = position == picker.selected;
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

fn visible_range(item_count: usize, selected: usize, height: usize) -> std::ops::Range<usize> {
    if item_count == 0 || height == 0 {
        return 0..0;
    }
    let selected = selected.min(item_count - 1);
    let start = selected.saturating_add(1).saturating_sub(height);
    let end = start.saturating_add(height).min(item_count);
    start..end
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!(
            "{}...",
            truncated
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        truncated
    }
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
}
