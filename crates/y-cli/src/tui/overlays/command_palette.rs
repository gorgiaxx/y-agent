//! Command palette overlay: floating popup showing filtered command list.
//!
//! Activated when the user types `/` or `:` (enters Command mode). Shows a
//! fuzzy-filtered list of available commands that updates on each keystroke.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::commands::registry::CommandRegistry;
use crate::tui::theme::Theme;

use super::picker::visible_range;

/// Outcome of a Backspace press inside the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteBackspace {
    /// Removed the last character of the filter input.
    Popped,
    /// Filter input was already empty in argument mode: argument
    /// completion closed, returning to the command list.
    ExitArgMode,
    /// Filter input was already empty at the command list: the `/` that
    /// opened the palette is deleted, so the palette itself should close.
    Close,
}

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
    /// Cached filtered argument synopses (e.g. "<session-id>"), aligned with
    /// `filtered_names` so rendering needs no registry lookups.
    pub filtered_synopses: Vec<&'static str>,
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
        let all = CommandRegistry::shared().all();
        Self {
            input: String::new(),
            selected: 0,
            filtered_names: all.iter().map(|c| c.name.to_string()).collect(),
            filtered_descriptions: all.iter().map(|c| c.description.to_string()).collect(),
            filtered_synopses: all.iter().map(|c| c.args).collect(),
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
        let registry = CommandRegistry::shared();
        let results = if self.input.is_empty() {
            registry.all().iter().collect::<Vec<_>>()
        } else {
            registry.search(&self.input)
        };

        self.filtered_names = results.iter().map(|c| c.name.to_string()).collect();
        self.filtered_descriptions = results.iter().map(|c| c.description.to_string()).collect();
        self.filtered_synopses = results.iter().map(|c| c.args).collect();

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
        self.selected = 0;
        self.update_filter();
    }

    /// Pop the last character from the input.
    pub fn pop_char(&mut self) {
        self.input.pop();
        self.selected = 0;
        self.update_filter();
    }

    /// Handle a Backspace press, including the empty-input edges: leaving
    /// argument mode, or signalling that the palette itself should close.
    pub fn backspace(&mut self) -> PaletteBackspace {
        if !self.input.is_empty() {
            self.pop_char();
            return PaletteBackspace::Popped;
        }
        if self.in_arg_mode() {
            *self = Self::new();
            return PaletteBackspace::ExitArgMode;
        }
        PaletteBackspace::Close
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

    let popup_height = palette_height(item_count, area.height);
    let popup_width = area.width.saturating_sub(4).clamp(30, 72);

    let x = area.x + 2;
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

    let prefix = "/";
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

fn palette_height(item_count: usize, area_height: u16) -> u16 {
    let max_height = (area_height / 2).clamp(5, 15);
    u16::try_from(item_count)
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .clamp(5, max_height)
}

fn render_command_list(
    frame: &mut Frame,
    list_area: Rect,
    palette: &CommandPaletteState,
    t: &Theme,
) {
    let range = visible_range(
        palette.filtered_names.len(),
        palette.selected,
        list_area.height as usize,
    );
    // All metadata comes from the palette's cached vectors: no registry
    // lookups or command searches during rendering.
    let items: Vec<ListItem> = range
        .map(|i| {
            let name = &palette.filtered_names[i];
            let desc = palette
                .filtered_descriptions
                .get(i)
                .map_or("", std::string::String::as_str);
            let args = palette.filtered_synopses.get(i).copied().unwrap_or("");

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
                Span::styled(format!(" /{name} {args}"), style),
                Span::styled(format!("  {desc}"), desc_style),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items), list_area);
}

fn render_arg_list(frame: &mut Frame, list_area: Rect, palette: &CommandPaletteState, t: &Theme) {
    let range = visible_range(
        palette.filtered_args.len(),
        palette.selected,
        list_area.height as usize,
    );
    let items: Vec<ListItem> = range
        .map(|i| {
            let (id, desc) = &palette.filtered_args[i];
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
        let message = match palette.arg_command.as_deref() {
            Some("goal") => " Type an objective and press Enter",
            _ => " No matches",
        };
        let empty = ListItem::new(Line::from(Span::styled(
            message,
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

    // T-PALETTE-BACKSPACE-01: Backspace on an empty command filter signals
    // that the palette should close (the `/` that opened it is deleted).
    #[test]
    fn test_backspace_on_empty_command_input_closes_palette() {
        let mut palette = CommandPaletteState::new();
        assert_eq!(palette.backspace(), PaletteBackspace::Close);
    }

    // T-PALETTE-BACKSPACE-02: Backspace on an empty argument filter leaves
    // argument mode and returns to the command list instead of closing.
    #[test]
    fn test_backspace_on_empty_arg_input_exits_arg_mode() {
        let mut palette = CommandPaletteState::new();
        palette.enter_arg_mode(
            "model".to_string(),
            vec![("gpt-5".to_string(), String::new())],
        );
        assert_eq!(palette.backspace(), PaletteBackspace::ExitArgMode);
        assert!(!palette.in_arg_mode());
        assert!(palette.input.is_empty());
    }

    // T-PALETTE-BACKSPACE-03: Backspace pops typed characters first; only an
    // already-empty filter closes the palette.
    #[test]
    fn test_backspace_pops_typed_characters_before_closing() {
        let mut palette = CommandPaletteState::new();
        palette.push_char('n');
        assert_eq!(palette.backspace(), PaletteBackspace::Popped);
        assert!(palette.input.is_empty());
        assert_eq!(palette.backspace(), PaletteBackspace::Close);
    }

    #[test]
    fn test_freeform_argument_mode_preserves_typed_goal() {
        let mut palette = CommandPaletteState::new();
        palette.enter_arg_mode("goal".to_string(), Vec::new());
        for ch in "ship release".chars() {
            palette.push_char(ch);
        }

        assert_eq!(palette.input, "ship release");
        assert!(palette.selected_arg().is_none());
    }

    #[test]
    fn test_freeform_palette_keeps_room_for_input_and_hint() {
        assert_eq!(palette_height(0, 24), 5);
    }

    #[test]
    fn test_palette_viewport_keeps_selected_item_visible() {
        assert_eq!(visible_range(20, 0, 5), 0..5);
        assert_eq!(visible_range(20, 4, 5), 0..5);
        assert_eq!(visible_range(20, 5, 5), 1..6);
        assert_eq!(visible_range(20, 19, 5), 15..20);
    }

    #[test]
    fn test_typing_resets_palette_selection_to_first_match() {
        let mut palette = CommandPaletteState::new();
        palette.select_next();
        palette.select_next();
        assert_eq!(palette.selected, 2);

        palette.push_char('r');
        assert_eq!(palette.selected, 0);
    }

    // T-PALETTE-CACHE-01: cached synopses stay aligned with filtered names.
    #[test]
    fn test_filtered_synopses_align_with_names() {
        let mut palette = CommandPaletteState::new();
        assert_eq!(
            palette.filtered_names.len(),
            palette.filtered_synopses.len()
        );
        let new_pos = palette
            .filtered_names
            .iter()
            .position(|n| n == "new")
            .expect("'new' command present");
        assert_eq!(palette.filtered_synopses[new_pos], "[label]");

        for ch in "sw".chars() {
            palette.push_char(ch);
        }
        assert_eq!(
            palette.filtered_names.len(),
            palette.filtered_synopses.len()
        );
        let sw_pos = palette
            .filtered_names
            .iter()
            .position(|n| n == "switch")
            .expect("'switch' matches 'sw'");
        assert_eq!(palette.filtered_synopses[sw_pos], "<session-id|label>");
    }
}
