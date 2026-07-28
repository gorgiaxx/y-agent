//! Reverse-search overlay for persisted prompt history.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use super::picker::{preview, visible_range, PickerItem, PickerState};
use crate::tui::theme::Theme;

#[derive(Debug, Clone)]
struct HistoryEntry {
    text: String,
    text_lower: String,
}

impl PickerItem for HistoryEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.text_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HistorySearchState {
    core: PickerState<HistoryEntry>,
}

impl HistorySearchState {
    pub fn new(history: &[String]) -> Self {
        Self {
            core: PickerState::new(
                history
                    .iter()
                    .rev()
                    .map(|text| HistoryEntry {
                        text: text.clone(),
                        text_lower: text.to_lowercase(),
                    })
                    .collect(),
            ),
        }
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.core.selected_item().map(|entry| entry.text.as_str())
    }

    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
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

pub fn render(frame: &mut Frame, area: Rect, search: &HistorySearchState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" Reverse prompt search ")
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
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Search: ", Style::default().fg(theme.muted())),
            Span::styled(search.core.query(), Style::default().fg(theme.text())),
            Span::styled("_", Style::default().fg(theme.input_border_focused())),
        ])),
        rows[0],
    );
    let width = rows[1].width.saturating_sub(3) as usize;
    let visible = visible_range(
        search.filtered_len(),
        search.core.selected(),
        rows[1].height as usize,
    );
    let items = visible
        .map(|position| {
            let entry = &search.core.items()[search.core.filtered()[position]];
            let style = if position == search.core.selected() {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text())
            };
            ListItem::new(Line::from(Span::styled(
                format!(" {}", preview(&entry.text.replace('\n', " "), width)),
                style,
            )))
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(" No matching prompts").style(Style::default().fg(theme.muted())),
            rows[1],
        );
    } else {
        frame.render_widget(List::new(items), rows[1]);
    }
    frame.render_widget(
        Paragraph::new(" Type to filter  Up/Down navigate  Enter recall  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_newest_first_and_case_insensitive() {
        let mut search =
            HistorySearchState::new(&["older release prompt".into(), "Newest TUI Prompt".into()]);
        assert_eq!(search.selected_text(), Some("Newest TUI Prompt"));
        for character in "release".chars() {
            search.push_char(character);
        }
        assert_eq!(search.filtered_len(), 1);
        assert_eq!(search.selected_text(), Some("older release prompt"));
    }
}
