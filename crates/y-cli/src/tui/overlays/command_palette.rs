//! Command palette overlay: floating popup showing filtered command list.
//!
//! Activated when the user types `:` (enters Command mode). Shows a
//! fuzzy-filtered list of available commands that updates on each keystroke.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::commands::registry::{CommandInfo, CommandRegistry};

/// State for the command palette overlay.
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// Current input text (prefix being typed).
    pub input: String,
    /// Index of selected item in filtered results.
    pub selected: usize,
    /// Cached filtered results (names only, for display).
    pub filtered_names: Vec<String>,
    /// Cached filtered descriptions.
    pub filtered_descriptions: Vec<String>,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPaletteState {
    pub fn new() -> Self {
        let registry = CommandRegistry::new();
        let all = registry.all();
        let names: Vec<String> = all.iter().map(|c| c.name.to_string()).collect();
        let descs: Vec<String> = all.iter().map(|c| c.description.to_string()).collect();
        Self {
            input: String::new(),
            selected: 0,
            filtered_names: names,
            filtered_descriptions: descs,
        }
    }

    /// Update the filtered results based on current input prefix.
    pub fn update_filter(&mut self) {
        let registry = CommandRegistry::new();
        let results: Vec<&CommandInfo> = if self.input.is_empty() {
            registry.all().iter().collect()
        } else {
            registry.search(&self.input)
        };

        self.filtered_names = results.iter().map(|c| c.name.to_string()).collect();
        self.filtered_descriptions = results.iter().map(|c| c.description.to_string()).collect();

        // Clamp selected index.
        if self.selected >= self.filtered_names.len() {
            self.selected = self.filtered_names.len().saturating_sub(1);
        }
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if self.selected + 1 < self.filtered_names.len() {
            self.selected += 1;
        }
    }

    /// Get the currently selected command name (if any).
    pub fn selected_command(&self) -> Option<&str> {
        self.filtered_names
            .get(self.selected)
            .map(std::string::String::as_str)
    }

    /// Push a character to the input.
    pub fn push_char(&mut self, ch: char) {
        self.input.push(ch);
        self.update_filter();
    }

    /// Pop the last character from the input.
    pub fn pop_char(&mut self) {
        self.input.pop();
        self.update_filter();
    }
}

/// Render the command palette overlay.
///
/// The palette is a floating popup anchored to the bottom of the screen,
/// positioned above the input area.
pub fn render(frame: &mut Frame, area: Rect, palette: &CommandPaletteState) {
    // Calculate popup size: min 5 rows, up to half the screen.
    let max_height = (area.height / 2).max(5).min(15);
    let popup_height = (palette.filtered_names.len() as u16 + 3).min(max_height);
    let popup_width = area.width.min(50).max(30);

    // Position: centered horizontally, above the bottom quarter.
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + area.height.saturating_sub(popup_height + 4);

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the area behind the popup.
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Commands ")
        .title_style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if inner.height < 2 {
        return;
    }

    // Input line.
    let input_line = Line::from(vec![
        Span::styled(":", Style::default().fg(Color::Yellow)),
        Span::styled(&palette.input, Style::default().fg(Color::White)),
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]);
    let input_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(Paragraph::new(input_line), input_area);

    // List area.
    let list_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );

    let items: Vec<ListItem> = palette
        .filtered_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let desc = palette
                .filtered_descriptions
                .get(i)
                .map_or("", std::string::String::as_str);

            let style = if i == palette.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let desc_style = if i == palette.selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" /{name}"), style),
                Span::styled(format!("  {desc}"), desc_style),
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

    // T-TUI-04-03: Command palette fuzzy filter narrows correctly.
    #[test]
    fn test_palette_filter() {
        let mut palette = CommandPaletteState::new();
        let initial_count = palette.filtered_names.len();
        assert!(initial_count >= 15);

        palette.push_char('n');
        palette.push_char('e');
        // Should narrow to commands starting with "ne" or matching "ne" in description.
        assert!(palette.filtered_names.len() < initial_count);
        assert!(palette.filtered_names.contains(&"new".to_string()));
    }

    #[test]
    fn test_palette_select_navigation() {
        let mut palette = CommandPaletteState::new();
        assert_eq!(palette.selected, 0);

        palette.select_next();
        assert_eq!(palette.selected, 1);

        palette.select_next();
        assert_eq!(palette.selected, 2);

        palette.select_prev();
        assert_eq!(palette.selected, 1);

        palette.select_prev();
        palette.select_prev(); // Should clamp to 0.
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn test_palette_selected_command() {
        let palette = CommandPaletteState::new();
        assert!(palette.selected_command().is_some());
    }

    #[test]
    fn test_palette_backspace() {
        let mut palette = CommandPaletteState::new();
        palette.push_char('q');
        let narrow_count = palette.filtered_names.len();

        palette.pop_char();
        assert!(palette.filtered_names.len() > narrow_count);
    }
}
