//! Search overlay for the visible session transcript.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use super::picker::{preview, visible_range, PickerItem, PickerState};
use crate::tui::state::{ChatMessage, MessageRole};
use crate::tui::theme::Theme;

#[derive(Debug, Clone)]
struct TranscriptEntry {
    message_index: usize,
    role: MessageRole,
    content: String,
    search_lower: String,
}

impl PickerItem for TranscriptEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.search_lower.contains(query_lower)
    }
}

#[derive(Debug, Clone, Default)]
pub struct TranscriptSearchState {
    core: PickerState<TranscriptEntry>,
}

impl TranscriptSearchState {
    pub fn new(messages: &[ChatMessage]) -> Self {
        Self {
            core: PickerState::new(
                messages
                    .iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, message)| !message.content.trim().is_empty())
                    .map(|(message_index, message)| TranscriptEntry {
                        message_index,
                        role: message.role,
                        content: message.content.clone(),
                        search_lower: format!("{:?} {}", message.role, message.content)
                            .to_lowercase(),
                    })
                    .collect(),
            ),
        }
    }

    pub fn selected(&self) -> Option<(usize, &str)> {
        self.core
            .selected_item()
            .map(|entry| (entry.message_index, entry.content.as_str()))
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

pub fn render(frame: &mut Frame, area: Rect, search: &TranscriptSearchState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" Search transcript ")
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
    let width = rows[1].width.saturating_sub(16) as usize;
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
                format!(
                    " {:<9} {}",
                    role_label(entry.role),
                    preview(&entry.content.replace('\n', " "), width)
                ),
                style,
            )))
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items), rows[1]);
    frame.render_widget(
        Paragraph::new(" Type to filter  Enter jump  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[2],
    );
}

fn role_label(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "you",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Tool => "tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn message(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.into(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn transcript_search_matches_role_and_content_newest_first() {
        let mut search = TranscriptSearchState::new(&[
            message(MessageRole::User, "old needle"),
            message(MessageRole::Assistant, "new answer"),
        ]);
        assert_eq!(search.selected().map(|(_, text)| text), Some("new answer"));
        for character in "needle".chars() {
            search.push_char(character);
        }
        assert_eq!(search.filtered_len(), 1);
        assert_eq!(search.selected(), Some((0, "old needle")));
    }
}
