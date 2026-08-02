//! Command palette overlay: floating popup showing filtered command list.
//!
//! Activated when the user types `/` or `:` (enters Command mode). Shows a
//! fuzzy-filtered list projected from the primary composer's text. The popup
//! is anchored to the composer's left edge; each row is a three-column
//! layout — command (left), description (middle), shortcut (right-aligned,
//! relabeled for the host platform).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::commands::registry::CommandRegistry;
use crate::tui::keys::{platform_shortcut_label, KeyAction, KeyContext, Keymap};
use crate::tui::theme::Theme;

use super::picker::visible_range;

/// State for the command palette overlay.
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// Filter query projected from the primary composer.
    pub query: String,
    /// Index of selected item in filtered results.
    pub selected: usize,
    /// Cached filtered results (names only, for display).
    pub filtered_names: Vec<String>,
    /// Cached filtered descriptions.
    pub filtered_descriptions: Vec<String>,
    /// Cached filtered argument synopses (e.g. "<session-id>"), aligned with
    /// `filtered_names` so rendering needs no registry lookups.
    pub filtered_synopses: Vec<&'static str>,
    /// Semantic shortcut actions aligned with `filtered_names`.
    pub filtered_shortcuts: Vec<Option<KeyAction>>,
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
            query: String::new(),
            selected: 0,
            filtered_names: all.iter().map(|c| c.name.to_string()).collect(),
            filtered_descriptions: all.iter().map(|c| c.description.to_string()).collect(),
            filtered_synopses: all.iter().map(|c| c.args).collect(),
            filtered_shortcuts: all
                .iter()
                .map(super::super::commands::registry::CommandInfo::shortcut_action)
                .collect(),
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
        self.query.clear();
        self.selected = 0;
    }

    /// Whether the palette is in argument-completion mode.
    pub fn in_arg_mode(&self) -> bool {
        self.arg_command.is_some()
    }

    /// Rebuild completion state from the primary composer text.
    ///
    /// The palette never owns editable text: it is a projection of the
    /// leading slash command around the composer's real cursor and buffer.
    pub fn sync_from_composer(&mut self, composer_text: &str) -> bool {
        let Some(command_text) = composer_text.trim_start().strip_prefix('/') else {
            return false;
        };
        let separator = command_text.find(char::is_whitespace);
        let command_name = separator.map_or(command_text, |index| &command_text[..index]);
        let arguments = separator.map_or("", |index| command_text[index..].trim_start());

        let argument_command_matches = self.arg_command.as_deref().is_some_and(|command| {
            CommandRegistry::shared().resolve_alias(command_name) == command && separator.is_some()
        });
        if self.in_arg_mode() && !argument_command_matches {
            self.arg_command = None;
            self.arg_completions.clear();
            self.filtered_args.clear();
        }

        self.query = if argument_command_matches {
            arguments.to_string()
        } else {
            command_name.to_string()
        };
        self.selected = 0;
        self.update_filter();
        true
    }

    /// Update the filtered results based on current input prefix.
    pub fn update_filter(&mut self) {
        if self.in_arg_mode() {
            let query = self.query.to_lowercase();
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
        let results = if self.query.is_empty() {
            registry.all().iter().collect::<Vec<_>>()
        } else {
            registry.search(&self.query)
        };

        self.filtered_names = results.iter().map(|c| c.name.to_string()).collect();
        self.filtered_descriptions = results.iter().map(|c| c.description.to_string()).collect();
        self.filtered_synopses = results.iter().map(|c| c.args).collect();
        self.filtered_shortcuts = results.iter().map(|c| c.shortcut_action()).collect();

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

    /// Selected command name and argument synopsis for composer completion.
    pub fn selected_command_completion(&self) -> Option<(&str, &str)> {
        if self.in_arg_mode() {
            return None;
        }
        Some((
            self.filtered_names.get(self.selected)?.as_str(),
            self.filtered_synopses
                .get(self.selected)
                .copied()
                .unwrap_or(""),
        ))
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
}

/// Render the command palette overlay.
///
/// The palette is a completion popup anchored immediately above the primary
/// composer. It renders suggestions and key hints only; editable text and the
/// cursor remain owned by the composer.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    composer_area: Rect,
    palette: &CommandPaletteState,
    keymap: &Keymap,
    t: &Theme,
) {
    let item_count = if palette.in_arg_mode() {
        palette.filtered_args.len()
    } else {
        palette.filtered_names.len()
    };

    let available_height = composer_area.y.saturating_sub(area.y);
    let popup_width = composer_area.width.saturating_sub(2).min(72);
    if available_height < 5 || popup_width < 20 {
        return;
    }
    let popup_height = palette_height(item_count, area.height).min(available_height);
    // Anchored to the composer's left edge, floating above it.
    let x = composer_area.x;
    let y = composer_area.y.saturating_sub(popup_height);

    let popup_area = Rect::new(x, y, popup_width, popup_height);
    frame.render_widget(Clear, popup_area);

    let title = if let Some(cmd) = &palette.arg_command {
        format!(" /{cmd} arguments ")
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

    // Candidate list on top, shortcut guidance on the bottom row. The real
    // command text and cursor stay visible in the composer below the popup.
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(1),
    );
    let footer_area = Rect::new(inner.x, inner.y + list_area.height, inner.width, 1);

    if palette.in_arg_mode() {
        render_arg_list(frame, list_area, palette, t);
    } else {
        render_command_list(frame, list_area, palette, keymap, t);
    }

    frame.render_widget(
        Paragraph::new(palette_footer(keymap)).style(Style::default().fg(t.muted())),
        footer_area,
    );
}

fn palette_footer(keymap: &Keymap) -> String {
    let hint = |action, fallback: &str| {
        keymap
            .primary_shortcut_in_context(KeyContext::Command, action)
            .unwrap_or_else(|| fallback.to_string())
    };
    format!(
        " {}/{} select  {} complete  {} run  {} close",
        hint(KeyAction::ScrollUp, "Up"),
        hint(KeyAction::ScrollDown, "Down"),
        hint(KeyAction::CompleteCommand, "Tab"),
        hint(KeyAction::Submit, "Enter"),
        hint(KeyAction::ReturnToNormal, "Esc"),
    )
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
    keymap: &Keymap,
    t: &Theme,
) {
    let range = visible_range(
        palette.filtered_names.len(),
        palette.selected,
        list_area.height as usize,
    );
    // Column geometry spans ALL filtered rows (not just the visible window)
    // so columns do not jump while scrolling.
    let columns = palette_columns(palette, keymap);
    let row_width = list_area.width as usize;

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
            let shortcut = palette
                .filtered_shortcuts
                .get(i)
                .copied()
                .flatten()
                .and_then(|action| keymap.primary_shortcut(action))
                .map(|label| platform_shortcut_label(&label));

            let command_style = if i == palette.selected {
                Style::default()
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text())
            };

            let detail_style = if i == palette.selected {
                Style::default()
                    .fg(t.panel_bg())
                    .bg(t.input_border_focused())
            } else {
                Style::default().fg(t.muted())
            };

            ListItem::new(palette_row(
                &command_label(name, args),
                desc,
                shortcut.as_deref(),
                &columns,
                row_width,
                command_style,
                detail_style,
            ))
        })
        .collect();

    frame.render_widget(List::new(items), list_area);
}

/// Column geometry shared by all palette rows.
struct PaletteColumns {
    /// Display width of the widest `/{name} {args}` label.
    command_width: usize,
    /// Display width of the widest shortcut label.
    shortcut_width: usize,
}

/// Compute the column widths across all filtered rows.
fn palette_columns(palette: &CommandPaletteState, keymap: &Keymap) -> PaletteColumns {
    let command_width = palette
        .filtered_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let args = palette.filtered_synopses.get(i).copied().unwrap_or("");
            UnicodeWidthStr::width(command_label(name, args).as_str())
        })
        .max()
        .unwrap_or(0);
    let shortcut_width = palette
        .filtered_shortcuts
        .iter()
        .filter_map(|action| action.and_then(|a| keymap.primary_shortcut(a)))
        .map(|label| UnicodeWidthStr::width(platform_shortcut_label(&label).as_str()))
        .max()
        .unwrap_or(0);
    PaletteColumns {
        command_width,
        shortcut_width,
    }
}

/// Command column label: `/name args`, without a dangling space when the
/// command takes no arguments.
fn command_label(name: &str, args: &str) -> String {
    if args.is_empty() {
        format!("/{name}")
    } else {
        format!("/{name} {args}")
    }
}

/// Build one palette row: command left, description middle, shortcut
/// right-aligned at the row's right edge (one-cell margin).
///
/// Padding spans carry the detail style so the selected-row highlight band
/// stays uniform across the whole row.
fn palette_row(
    command: &str,
    description: &str,
    shortcut: Option<&str>,
    columns: &PaletteColumns,
    row_width: usize,
    command_style: Style,
    detail_style: Style,
) -> Line<'static> {
    let command_width = UnicodeWidthStr::width(command);
    let command_pad = columns.command_width.saturating_sub(command_width);

    // Layout budget: " " + command column + "  " + description + gap
    // + shortcut column + " " (right margin).
    let desc_start = 1 + columns.command_width + 2;
    let shortcut_cell = if columns.shortcut_width == 0 {
        0
    } else {
        columns.shortcut_width + 1
    };
    let desc_budget = row_width.saturating_sub(desc_start + 1 + shortcut_cell);
    let description = truncate_to_width(description, desc_budget);
    let desc_width = UnicodeWidthStr::width(description.as_str());

    let shortcut = shortcut.unwrap_or("");
    let used = desc_start + desc_width;
    let gap = row_width.saturating_sub(used + shortcut_cell + 1);
    let shortcut_pad = columns
        .shortcut_width
        .saturating_sub(UnicodeWidthStr::width(shortcut));

    let mut spans = vec![
        Span::styled(
            format!(" {command}{}", " ".repeat(command_pad)),
            command_style,
        ),
        Span::styled("  ", detail_style),
        Span::styled(description, detail_style),
    ];
    if gap > 0 {
        spans.push(Span::styled(" ".repeat(gap), detail_style));
    }
    if columns.shortcut_width > 0 {
        spans.push(Span::styled(
            format!("{}{shortcut} ", " ".repeat(shortcut_pad)),
            detail_style,
        ));
    }
    Line::from(spans)
}

/// Truncate text to `max_width` display columns, ending with an ellipsis
/// when truncation occurs.
fn truncate_to_width(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "\u{2026}".to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width + 1 > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('\u{2026}');
    out
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
            Some("todo") => " Type TODO text and press Enter",
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

        palette.sync_from_composer("/ne");
        // Should narrow to commands starting with "ne" or matching "ne" in description.
        assert!(palette.filtered_names.len() < initial_count);
        assert!(palette.filtered_names.contains(&"new".to_string()));
    }

    #[test]
    fn test_palette_filters_from_composer_owned_slash_text() {
        let mut palette = CommandPaletteState::new();

        assert!(palette.sync_from_composer("/pla"));
        assert_eq!(palette.query, "pla");
        assert_eq!(palette.selected_command(), Some("plan"));

        assert!(!palette.sync_from_composer("plain text"));
    }

    #[test]
    fn test_argument_filter_is_derived_from_composer_text() {
        let mut palette = CommandPaletteState::new();
        palette.enter_arg_mode("goal".to_string(), Vec::new());

        assert!(palette.sync_from_composer("/goal ship release"));
        assert_eq!(palette.query, "ship release");
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
    fn test_freeform_argument_mode_preserves_typed_goal() {
        let mut palette = CommandPaletteState::new();
        palette.enter_arg_mode("goal".to_string(), Vec::new());
        palette.sync_from_composer("/goal ship release");

        assert_eq!(palette.query, "ship release");
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

        palette.sync_from_composer("/r");
        assert_eq!(palette.selected, 0);
    }

    // T-PALETTE-LAYOUT-01: autocomplete is anchored above the real composer
    // and does not render a second input row or cursor.
    #[test]
    fn test_list_renders_above_composer_without_duplicate_input() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let palette = CommandPaletteState::new();
        let first_command = palette.filtered_names[0].clone();
        let keymap = Keymap::default();
        let theme = Theme::default();
        let composer = Rect::new(0, 20, 80, 3);
        terminal
            .draw(|frame| render(frame, frame.area(), composer, &palette, &keymap, &theme))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row_has = |y: u16, needle: &str| {
            let row: String = (0..80)
                .filter_map(|x| buffer.cell((x, y)).map(ratatui::buffer::Cell::symbol))
                .collect();
            row.contains(needle)
        };

        let item_row = (0..24).find(|&y| row_has(y, &format!("/{first_command}")));
        let item_row = item_row.expect("palette must render command candidates");
        assert!(item_row < composer.y);
        assert!(!(0..24).any(|y| row_has(y, "\u{2588}")));
    }

    #[test]
    fn test_slash_text_remains_visible_in_primary_composer() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut palette = CommandPaletteState::new();
        palette.sync_from_composer("/pla");
        let mut textarea = tui_textarea::TextArea::new(vec!["/pla".to_string()]);
        textarea.move_cursor(tui_textarea::CursorMove::End);
        let keymap = Keymap::default();
        let theme = Theme::default();
        let composer = Rect::new(0, 20, 80, 3);

        terminal
            .draw(|frame| {
                crate::tui::panels::input::render(
                    frame,
                    composer,
                    crate::tui::state::PanelFocus::Input,
                    crate::tui::state::InteractionMode::Command,
                    false,
                    false,
                    0,
                    &mut textarea,
                    &keymap,
                    &theme,
                );
                render(frame, frame.area(), composer, &palette, &keymap, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let composer_row: String = (0..80)
            .filter_map(|x| {
                buffer
                    .cell((x, composer.y + 1))
                    .map(ratatui::buffer::Cell::symbol)
            })
            .collect();
        assert!(
            composer_row.contains("/pla"),
            "composer row: {composer_row:?}"
        );
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

        palette.sync_from_composer("/sw");
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

    #[test]
    fn test_command_shortcuts_stay_aligned_with_filtered_rows() {
        let mut palette = CommandPaletteState::new();
        assert_eq!(
            palette.filtered_names.len(),
            palette.filtered_shortcuts.len()
        );

        palette.sync_from_composer("/queue");
        let queue = palette
            .filtered_names
            .iter()
            .position(|name| name == "queue")
            .unwrap();
        assert_eq!(
            palette.filtered_shortcuts[queue],
            Some(KeyAction::OpenQueue)
        );
    }

    /// Display column where `needle` starts within a rendered line.
    fn column_of(line: &Line, needle: &str) -> Option<usize> {
        let mut x = 0usize;
        for span in &line.spans {
            if let Some(offset) = span.content.find(needle) {
                return Some(x + offset);
            }
            x += Span::width(span);
        }
        None
    }

    // T-PALETTE-COLUMNS-01: rows lay out as three aligned columns — command
    // left, description middle, shortcut right-aligned at the row end.
    #[test]
    fn test_rows_align_command_description_shortcut_columns() {
        let columns = PaletteColumns {
            command_width: 12,
            shortcut_width: 7,
        };
        let style = Style::default();
        let width = 50;
        let short_cmd = palette_row(
            "/new",
            "New session",
            Some("Ctrl+Q"),
            &columns,
            width,
            style,
            style,
        );
        let long_cmd = palette_row(
            "/backtrack",
            "Jump back",
            Some("F2"),
            &columns,
            width,
            style,
            style,
        );

        // Description column starts right after the padded command column.
        let desc_col = 1 + 12 + 2;
        assert_eq!(column_of(&short_cmd, "New session"), Some(desc_col));
        assert_eq!(column_of(&long_cmd, "Jump back"), Some(desc_col));

        // Shortcut column is right-aligned: both shortcuts END one cell
        // before the row edge (the right margin).
        let end_a = column_of(&short_cmd, "Ctrl+Q").unwrap() + "Ctrl+Q".len();
        let end_b = column_of(&long_cmd, "F2").unwrap() + "F2".len();
        assert_eq!(end_a, width - 2, "Ctrl+Q must end at the right edge");
        assert_eq!(end_b, width - 2, "F2 must end at the right edge");

        // Total row width stays within the budget.
        let row_width: usize = short_cmd.spans.iter().map(Span::width).sum();
        assert!(row_width <= width, "row overflows: {row_width} > {width}");
    }

    // T-PALETTE-COLUMNS-02: over-long descriptions truncate with an ellipsis
    // instead of pushing the shortcut column off the row.
    #[test]
    fn test_long_description_truncates_with_ellipsis() {
        let columns = PaletteColumns {
            command_width: 10,
            shortcut_width: 5,
        };
        let style = Style::default();
        let row = palette_row(
            "/cmd",
            "a very long description that cannot possibly fit the row",
            Some("F3"),
            &columns,
            30,
            style,
            style,
        );
        let row_width: usize = row.spans.iter().map(Span::width).sum();
        assert!(row_width <= 30, "row overflows: {row_width}");
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('\u{2026}'), "expected ellipsis: {text:?}");
        assert!(text.contains("F3"), "shortcut must survive: {text:?}");
    }

    // T-PALETTE-ALIGN-01: the popup is anchored to the composer's left edge.
    #[test]
    fn test_popup_left_aligned_with_composer() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let palette = CommandPaletteState::new();
        let keymap = Keymap::default();
        let theme = Theme::default();
        let composer = Rect::new(10, 20, 60, 3);
        terminal
            .draw(|frame| render(frame, frame.area(), composer, &palette, &keymap, &theme))
            .unwrap();

        let buffer = terminal.backend().buffer();
        // The popup's top-left corner sits at the composer's left edge.
        let corner_row = (0..composer.y).find(|&y| {
            buffer
                .cell((composer.x, y))
                .is_some_and(|cell| cell.symbol() == "\u{250C}")
        });
        assert!(
            corner_row.is_some(),
            "popup must be left-aligned with the composer"
        );
    }
}
