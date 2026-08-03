//! Prompt backtrack selector for branching from an earlier user message.

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::state::{AppState, MessageRole};
use crate::tui::theme::Theme;

use super::picker::{preview, visible_range};

/// Cache entry: `(messages.len(), session id, user-message indices)`.
type UserIndexCacheEntry = (usize, Option<String>, Rc<Vec<usize>>);

thread_local! {
    /// Cached indices of user messages, keyed by `(messages.len(), session id)`.
    ///
    /// Heuristic: a message's role never changes once pushed, so the set of
    /// user-message indices only changes when the list length changes
    /// (append, clear, truncate) or a different session is loaded. There is
    /// no message-list generation counter on `AppState`, so this pair is the
    /// cheapest sound invalidation key available.
    static USER_INDEX_CACHE: RefCell<Option<UserIndexCacheEntry>> =
        const { RefCell::new(None) };
}

/// Indices of user messages in `state.messages`, cached across frames.
fn user_message_indices(state: &AppState) -> Rc<Vec<usize>> {
    let key = (state.messages.len(), state.current_session_id.clone());
    USER_INDEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((len, session_id, indices)) = &*cache {
            if (*len, session_id.clone()) == key {
                return indices.clone();
            }
        }
        let indices = Rc::new(
            state
                .messages
                .iter()
                .enumerate()
                .filter(|(_, message)| message.role == MessageRole::User)
                .map(|(index, _)| index)
                .collect::<Vec<_>>(),
        );
        *cache = Some((key.0, key.1, indices.clone()));
        indices
    })
}

/// Render the full-screen prompt backtrack selector.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_focused()))
        .title(" Backtrack to a prompt ")
        .title_style(
            Style::default()
                .fg(theme.title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(" Select a user prompt. Confirming creates a new branch; the original session is preserved.")
            .style(Style::default().fg(theme.muted()))
            .wrap(Wrap { trim: true }),
        rows[0],
    );

    let prompts = user_message_indices(state);
    let selected_position = state
        .selected_message
        .and_then(|selected| prompts.binary_search(&selected).ok())
        .unwrap_or_else(|| prompts.len().saturating_sub(1));
    let visible = visible_range(prompts.len(), selected_position, rows[1].height as usize);
    let preview_width = usize::from(rows[1].width.saturating_sub(25)).max(8);
    let items: Vec<ListItem> = visible
        .map(|position| {
            let message_index = prompts[position];
            let message = &state.messages[message_index];
            let selected = state.selected_message == Some(message_index);
            let style = if selected {
                Style::default()
                    .fg(theme.panel_bg())
                    .bg(theme.selected())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.normal())
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    " {:>3}  {}  {}",
                    position + 1,
                    message.timestamp.format("%Y-%m-%d %H:%M"),
                    prompt_preview(&message.content, preview_width)
                ),
                style,
            )))
        })
        .collect();
    frame.render_widget(List::new(items), rows[1]);

    let details = state.selected_user_message().map_or_else(
        || "No user prompt selected.".to_string(),
        |message| {
            format!(
                "Selected prompt\n{}\n\nEnter creates a branch before this prompt and restores it to the input editor.",
                prompt_preview(&message.content, 180)
            )
        },
    );
    frame.render_widget(
        Paragraph::new(details)
            .style(Style::default().fg(theme.text()))
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted())),
            ),
        rows[2],
    );

    frame.render_widget(
        Paragraph::new(" Enter edit  r retry  q quote  b fork  y copy  t tools  d diff  Esc close")
            .style(Style::default().fg(theme.muted())),
        rows[3],
    );
}

fn prompt_preview(value: &str, max_chars: usize) -> String {
    let preview = preview(value, max_chars);
    if preview.is_empty() {
        "(empty prompt)".to_string()
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::{ChatMessage, MessageRole};
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

    fn state_with(roles: &[MessageRole]) -> AppState {
        let mut state = AppState::new();
        for (i, role) in roles.iter().enumerate() {
            state.messages.push(message(*role, &format!("msg {i}")));
        }
        state
    }

    #[test]
    fn prompt_preview_flattens_and_truncates_content() {
        assert_eq!(prompt_preview(" first\n  second ", 40), "first second");
        assert_eq!(prompt_preview("abcdefgh", 6), "abc...");
        assert_eq!(prompt_preview("  ", 10), "(empty prompt)");
    }

    #[test]
    fn visible_range_keeps_selection_on_screen() {
        assert_eq!(visible_range(10, 8, 4), 5..9);
        assert_eq!(visible_range(3, 2, 5), 0..3);
    }

    // T-BACKTRACK-CACHE-01: indices list only user messages, in order.
    #[test]
    fn user_message_indices_filters_user_roles() {
        let state = state_with(&[
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::System,
            MessageRole::User,
        ]);
        let indices = user_message_indices(&state);
        assert_eq!(&*indices, &[0, 3]);
    }

    // T-BACKTRACK-CACHE-02: repeated calls with unchanged messages reuse the
    // cached allocation instead of rebuilding the list every frame.
    #[test]
    fn user_message_indices_reuses_cache_for_unchanged_state() {
        let state = state_with(&[MessageRole::User, MessageRole::Assistant]);
        let first = user_message_indices(&state);
        let second = user_message_indices(&state);
        assert!(
            Rc::ptr_eq(&first, &second),
            "unchanged state should hit the cache"
        );
    }

    // T-BACKTRACK-CACHE-03: appending a message invalidates the cache.
    #[test]
    fn user_message_indices_rebuilds_after_length_change() {
        let mut state = state_with(&[MessageRole::User, MessageRole::Assistant]);
        let before = user_message_indices(&state);
        state.messages.push(message(MessageRole::User, "msg 2"));
        let after = user_message_indices(&state);
        assert!(
            !Rc::ptr_eq(&before, &after),
            "length change should rebuild the cache"
        );
        assert_eq!(&*after, &[0, 2]);
    }

    // T-BACKTRACK-CACHE-04: a session switch with the same message count
    // still invalidates the cache (session id is part of the key).
    #[test]
    fn user_message_indices_rebuilds_on_session_switch() {
        let mut state = state_with(&[MessageRole::User, MessageRole::Assistant]);
        let before = user_message_indices(&state);
        state.current_session_id = Some("other-session".into());
        let after = user_message_indices(&state);
        assert!(
            !Rc::ptr_eq(&before, &after),
            "session id change should rebuild the cache"
        );
    }

    // T-BACKTRACK-RENDER-01: render does not panic with cached indices and
    // a selected user message.
    #[test]
    fn render_with_selection_does_not_panic() {
        let mut state = state_with(&[MessageRole::User, MessageRole::Assistant, MessageRole::User]);
        state.selected_message = Some(2);
        let theme = Theme::default();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &state, &theme);
            })
            .unwrap();
        // Render twice to exercise the cache-hit path as well.
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &state, &theme);
            })
            .unwrap();
    }
}
