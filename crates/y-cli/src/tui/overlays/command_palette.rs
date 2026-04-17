//! Command palette overlay: floating popup showing filtered command list.
//!
//! Activated when the user types `:` (enters Command mode). Shows a
//! fuzzy-filtered list of available commands that updates on each keystroke.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::commands::registry::{CommandInfo, CommandRegistry};
use crate::tui::theme::Theme;

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
    /// When set, the palette is in argument-completion mode for this command.
    pub arg_command: Option<String>,
    /// Available argument completions (e.g. provider IDs for `/model`).
    pub arg_completions: Vec<(String, String)>,
    /// Filtered argument completions based on current input.
    pub filtered_args: Vec<(String, String)>,
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
            arg_command: None,
            arg_completions: Vec::new(),
            filtered_args: Vec::new(),
        }
    }

    /// Enter argument-completion mode for a command.
    pub fn enter_arg_mode(&mut self, command: String, completions: Vec<(String, String)>) {
        self.arg_command = Some(command);
        self.arg_completions = completions.clone();
        self.filtered_args = completions;
        self.input.clear();
        self.selected = 0;
    }

    /// Whether the palette is in argument-completion mode.
    pub fn in_arg_mode(&self) -> bool {
        self.arg_command.is_some()
    }

    /// Update the filtered results based on current input prefix.
    pub fn update_filter(&mut self) {
        if self.in_arg_mode() {
            let query = self.input.to_lowercase();
            self.filtered_args = if query.is_empty() {
                self.arg_completions.clone()
            } else {
                self.arg_completions
                    .iter()
                    .filter(|(id, desc)| {
                        id.to_lowercase().starts_with(&query)
                            || id.to_lowercase().contains(&query)
                            || desc.to_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect()
            };
            if self.selected >= self.filtered_args.len() {
                self.selected = self.filtered_args.len().saturating_sub(1);
            }
            return;
        }
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
        let max = if self.in_arg_mode() {
            self.filtered_args.len()
        } else {
            self.filtered_names.len()
        };
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    /// Get the currently selected command name (if any).
    pub fn selected_command(&self) -> Option<&str> {
        if self.in_arg_mode() {
            return None;
        }
        self.filtered_names
            .get(self.selected)
            .map(std::string::String::as_str)
    }

    /// Get the currently selected argument value (if in arg mode).
    pub fn selected_arg(&self) -> Option<&str> {
        if !self.in_arg_mode() {
            return None;
        }
        self.filtered_args
            .get(self.selected)
            .map(|(id, _)| id.as_str())
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
pub fn render(frame: &mut Frame, area: Rect, palette: &CommandPaletteState, t: &Theme) {
    let item_count = if palette.in_arg_mode() {
        palette.filtered_args.len()
    } else {
        palette.filtered_names.len()
    };

    let max_height = (area.height / 2).clamp(5, 15);
    let popup_height = (u16::try_from(item_count).unwrap_or(0) + 3).min(max_height);
    let popup_width = area.width.clamp(30, 55);

    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + area.height.saturating_sub(popup_height + 4);

    let popup_area = Rect::new(x, y, popup_width, popup_height);
    frame.render_widget(Clear, popup_area);

    let title = if let Some(cmd) = &palette.arg_command {
        format!(" /{cmd} ")
    } else {
        " Commands ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.input_border_focused()))
        .title(title)
        .title_style(
            Style::default()
                .fg(t.input_title())
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if inner.height < 2 {
        return;
    }

    let prefix = if palette.in_arg_mode() { "/" } else { ":" };
    let display_input = if let Some(cmd) = &palette.arg_command {
        format!("{cmd} {}", palette.input)
    } else {
        palette.input.clone()
    };
    let input_line = Line::from(vec![
        Span::styled(prefix, Style::default().fg(t.warning())),
        Span::styled(display_input, Style::default().fg(t.text())),
        Span::styled("\u{2588}", Style::default().fg(t.input_border_focused())),
    ]);
    let input_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(Paragraph::new(input_line), input_area);

    let list_area = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );

    if palette.in_arg_mode() {
        render_arg_list(frame, list_area, palette, t);
    } else {
        render_command_list(frame, list_area, palette, t);
    }
}

fn render_command_list(
    frame: &mut Frame,
    list_area: Rect,
    palette: &CommandPaletteState,
    t: &Theme,
) {
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
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text())
            };

            let desc_style = if i == palette.selected {
                Style::default()
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
            } else {
                Style::default().fg(t.muted())
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" /{name}"), style),
                Span::styled(format!("  {desc}"), desc_style),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items), list_area);
}

fn render_arg_list(frame: &mut Frame, list_area: Rect, palette: &CommandPaletteState, t: &Theme) {
    let items: Vec<ListItem> = palette
        .filtered_args
        .iter()
        .enumerate()
        .map(|(i, (id, desc))| {
            let style = if i == palette.selected {
                Style::default()
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text())
            };

            let desc_style = if i == palette.selected {
                Style::default()
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
            } else {
                Style::default().fg(t.muted())
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {id}"), style),
                Span::styled(format!("  {desc}"), desc_style),
            ]))
        })
        .collect();

    if items.is_empty() {
        let empty = ListItem::new(Line::from(Span::styled(
            " No matches",
            Style::default().fg(t.muted()),
        )));
        frame.render_widget(List::new(vec![empty]), list_area);
    } else {
        frame.render_widget(List::new(items), list_area);
    }
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
