//! Follow-up queue overlay: lists the active run's queued follow-ups and
//! lets the operator delete entries or promote them to the pending steer.
//!
//! Built on the shared [`PickerState`] core like the other picker overlays,
//! but key-driven without a search row: queues are short FIFO lists where
//! filtering adds no value.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use super::picker::{preview, visible_range, PickerItem, PickerState};
use crate::tui::keys::{KeyAction, Keymap};
use crate::tui::theme::Theme;

/// A follow-up plus its precomputed lowercase match text, mirroring the
/// entry wrappers of the other pickers.
#[derive(Debug, Clone)]
struct QueuePickerEntry {
    item: y_service::FollowUpMessage,
    text_lower: String,
}

impl QueuePickerEntry {
    fn new(item: y_service::FollowUpMessage) -> Self {
        Self {
            text_lower: item.text.to_ascii_lowercase(),
            item,
        }
    }
}

impl PickerItem for QueuePickerEntry {
    fn matches(&self, query_lower: &str) -> bool {
        self.text_lower.contains(query_lower)
    }
}

/// State for the follow-up queue overlay.
#[derive(Debug, Clone, Default)]
pub struct QueuePickerState {
    core: PickerState<QueuePickerEntry>,
}

impl QueuePickerState {
    /// Populate the picker from the projected follow-up queue (FIFO order).
    pub fn new(items: Vec<y_service::FollowUpMessage>) -> Self {
        Self {
            core: PickerState::new(items.into_iter().map(QueuePickerEntry::new).collect()),
        }
    }

    /// Number of visible queue entries.
    pub fn filtered_len(&self) -> usize {
        self.core.filtered_len()
    }

    /// Cursor position within the visible entries.
    pub fn selected(&self) -> usize {
        self.core.selected()
    }

    /// Preselect a row, e.g. to preserve the cursor across repopulation.
    /// Out-of-range values are clamped by the next navigation call.
    pub fn set_selected(&mut self, selected: usize) {
        self.core.set_selected(selected);
    }

    /// The selected follow-up, if the queue is non-empty.
    pub fn selected_item(&self) -> Option<&y_service::FollowUpMessage> {
        self.core.selected_item().map(|entry| &entry.item)
    }

    pub fn select_prev(&mut self) {
        self.core.select_prev();
    }

    pub fn select_next(&mut self) {
        self.core.select_next();
    }
}

/// Compact label for a follow-up's queue status.
fn status_label(status: y_service::FollowUpStatus) -> &'static str {
    match status {
        y_service::FollowUpStatus::Pending => "pending",
        y_service::FollowUpStatus::Steering => "steering",
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    picker: &QueuePickerState,
    keymap: &Keymap,
    theme: &Theme,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.input_border_focused()))
        .title(" TODO queue ")
        .title_style(
            Style::default()
                .fg(theme.input_title())
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let visible = visible_range(
        picker.filtered_len(),
        picker.core.selected(),
        rows[0].height as usize,
    );
    // Row layout: " NN. [status]   preview" -- the preview takes the rest.
    let preview_width = (rows[0].width as usize).saturating_sub(18);
    let items: Vec<ListItem> = visible
        .map(|position| {
            let entry = &picker.core.items()[picker.core.filtered()[position]];
            let selected = position == picker.core.selected();
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
                    " {:>2}. [{:<8}]  {}",
                    position + 1,
                    status_label(entry.item.status),
                    preview(&entry.item.text, preview_width),
                ),
                style,
            )))
        })
        .collect();
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(" (TODO queue is empty)").style(Style::default().fg(theme.muted())),
            rows[0],
        );
    } else {
        frame.render_widget(List::new(items), rows[0]);
    }

    frame.render_widget(
        Paragraph::new(queue_footer(keymap)).style(Style::default().fg(theme.muted())),
        rows[1],
    );
}

fn queue_footer(keymap: &Keymap) -> String {
    let mut hints = vec!["Up/Down navigate".to_string()];
    for (action, label) in [
        (KeyAction::QueueDelete, "delete"),
        (KeyAction::QueueSteer, "steer"),
        (KeyAction::QueueRecall, "edit"),
    ] {
        if let Some(shortcut) = keymap.primary_shortcut(action) {
            hints.push(format!("{shortcut} {label}"));
        }
    }
    hints.push("Esc close".to_string());
    hints.join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn follow_up(
        id: &str,
        text: &str,
        status: y_service::FollowUpStatus,
    ) -> y_service::FollowUpMessage {
        y_service::FollowUpMessage {
            id: id.into(),
            text: text.into(),
            created_at: 0,
            status,
        }
    }

    #[test]
    fn test_queue_picker_navigates_fifo_order() {
        let mut picker = QueuePickerState::new(vec![
            follow_up("fu-1", "first", y_service::FollowUpStatus::Pending),
            follow_up("fu-2", "second", y_service::FollowUpStatus::Pending),
        ]);

        assert_eq!(picker.filtered_len(), 2);
        assert_eq!(
            picker.selected_item().map(|item| item.id.as_str()),
            Some("fu-1")
        );

        picker.select_next();
        assert_eq!(
            picker.selected_item().map(|item| item.id.as_str()),
            Some("fu-2")
        );
        // Clamp at the last entry.
        picker.select_next();
        assert_eq!(
            picker.selected_item().map(|item| item.id.as_str()),
            Some("fu-2")
        );

        picker.select_prev();
        picker.select_prev();
        assert_eq!(
            picker.selected_item().map(|item| item.id.as_str()),
            Some("fu-1")
        );
    }

    #[test]
    fn test_queue_picker_empty_queue_has_no_selection() {
        let picker = QueuePickerState::new(Vec::new());
        assert_eq!(picker.filtered_len(), 0);
        assert!(picker.selected_item().is_none());
    }

    #[test]
    fn test_queue_picker_preserves_steering_status() {
        let picker = QueuePickerState::new(vec![follow_up(
            "fu-1",
            "steer me",
            y_service::FollowUpStatus::Steering,
        )]);
        assert_eq!(
            picker.selected_item().map(|item| item.status),
            Some(y_service::FollowUpStatus::Steering)
        );
        assert_eq!(
            status_label(y_service::FollowUpStatus::Steering),
            "steering"
        );
        assert_eq!(status_label(y_service::FollowUpStatus::Pending), "pending");
    }

    #[test]
    fn test_queue_footer_uses_effective_shortcuts() {
        let keymap = crate::tui::keys::Keymap::default();
        let footer = queue_footer(&keymap);

        assert!(footer.contains("d delete"));
        assert!(footer.contains("s steer"));
        assert!(footer.contains("e edit"));
        assert!(footer.contains("Esc close"));
    }

    #[test]
    fn test_queue_picker_set_selected_preselects_and_clamps() {
        let mut picker = QueuePickerState::new(vec![
            follow_up("fu-1", "first", y_service::FollowUpStatus::Pending),
            follow_up("fu-2", "second", y_service::FollowUpStatus::Pending),
        ]);

        picker.set_selected(1);
        assert_eq!(picker.selected(), 1);

        // Repopulation with a shorter queue clamps via the next navigation.
        picker = QueuePickerState::new(vec![follow_up(
            "fu-1",
            "first",
            y_service::FollowUpStatus::Pending,
        )]);
        picker.set_selected(0);
        picker.select_next();
        assert_eq!(picker.selected(), 0);
    }
}
