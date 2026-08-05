//! Text selection model for the chat panel.
//!
//! Tracks a mouse-drag text selection within the chat panel's content area.
//! Inspired by Zellij's `selection.rs` — positions are in content-space
//! (row/col relative to the rendered chat lines, not terminal coordinates).

/// A text selection within the chat panel.
#[derive(Debug, Clone, Default)]
pub struct TextSelection {
    /// Start position (row, col) in content-space.
    pub start: (usize, usize),
    /// End position (row, col) in content-space.
    pub end: (usize, usize),
    /// Whether the user is currently dragging.
    pub active: bool,
}

impl TextSelection {
    /// Begin a new selection at the given content-space position.
    pub fn start(&mut self, row: usize, col: usize) {
        self.start = (row, col);
        self.end = (row, col);
        self.active = true;
    }

    /// Update the selection endpoint during drag.
    pub fn update(&mut self, row: usize, col: usize) {
        if self.active {
            self.end = (row, col);
        }
    }

    /// Finish the selection (mouse released).
    pub fn finish(&mut self) {
        self.active = false;
    }

    /// Reset / clear the selection.
    pub fn reset(&mut self) {
        self.start = (0, 0);
        self.end = (0, 0);
        self.active = false;
    }

    /// Whether the selection is empty (zero-length).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Return (start, end) sorted so start <= end.
    pub fn sorted(&self) -> ((usize, usize), (usize, usize)) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    /// Check if the given (row, col) is within the selection.
    pub fn contains(&self, row: usize, col: usize) -> bool {
        if self.is_empty() {
            return false;
        }
        let ((sr, sc), (er, ec)) = self.sorted();

        if sr == er {
            // Single-line selection.
            return row == sr && col >= sc && col < ec;
        }

        if row == sr {
            return col >= sc;
        }
        if row == er {
            return col < ec;
        }
        row > sr && row < er
    }
}

/// One rendered chat row's text mirrors, bridging selection coordinates and
/// clipboard text.
///
/// Selection coordinates are character indices into `display` — exactly what
/// is on screen, decorations included. `copy` is what extraction returns;
/// decorative spans such as the code-block gutter and background padding are
/// excluded from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionRow {
    /// Concatenated span contents of the rendered row (what is on screen).
    pub display: String,
    /// Clipboard text of the raw, unwrapped source line.
    pub copy: String,
    /// Display char index within the raw source line at which this row
    /// starts; nonzero only for wrapped continuation rows.
    pub display_start: usize,
    /// Leading display chars of the raw source line excluded from `copy`
    /// (e.g. the code-block gutter width).
    pub copy_offset: usize,
}

impl SelectionRow {
    /// A row whose clipboard text is exactly what is on screen.
    pub fn simple(text: String) -> Self {
        Self {
            display: text.clone(),
            copy: text,
            display_start: 0,
            copy_offset: 0,
        }
    }
}

impl From<String> for SelectionRow {
    fn from(text: String) -> Self {
        Self::simple(text)
    }
}

/// Extract selected text from rendered chat rows.
///
/// The selection coordinates are char indices into each row's `display` text;
/// extraction maps them back into `copy` space so decorations (code gutter,
/// band padding) never reach the clipboard.
pub fn extract_text(rows: &[SelectionRow], selection: &TextSelection) -> String {
    if selection.is_empty() || rows.is_empty() {
        return String::new();
    }

    let ((sr, sc), (er, ec)) = selection.sorted();
    let mut result = Vec::new();

    let end_bound = er.min(rows.len().saturating_sub(1));
    for (row, line) in rows.iter().enumerate().take(end_bound + 1).skip(sr) {
        let display_len = line.display.chars().count();
        let start_col = if row == sr { sc } else { 0 };
        let end_col = if row == er {
            ec.min(display_len)
        } else {
            display_len
        };

        // Map the row-relative display range onto the raw source line, then
        // drop the decorative prefix to land in copy space.
        let copy_chars: Vec<char> = line.copy.chars().collect();
        let copy_start = (line.display_start + start_col).saturating_sub(line.copy_offset);
        let copy_end = (line.display_start + end_col)
            .saturating_sub(line.copy_offset)
            .min(copy_chars.len());

        if copy_start >= copy_chars.len() && start_col > 0 {
            // The selection starts beyond the row's copyable content (e.g. in
            // the background padding); skip the row instead of emitting a
            // spurious blank line. Rows covered from their left edge fall
            // through so interior blank lines survive extraction.
            continue;
        }

        let selected: String = copy_chars[copy_start..copy_end.max(copy_start)]
            .iter()
            .collect();
        result.push(selected.trim_end().to_string());
    }

    result.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_start_and_finish() {
        let mut sel = TextSelection::default();
        assert!(sel.is_empty());

        sel.start(5, 3);
        assert!(sel.active);
        assert!(!sel.is_empty() || sel.start == sel.end);

        sel.update(5, 10);
        assert!(!sel.is_empty());

        sel.finish();
        assert!(!sel.active);
    }

    #[test]
    fn test_selection_sorted() {
        let sel = TextSelection {
            start: (10, 5),
            end: (3, 2),
            ..Default::default()
        };
        let (s, e) = sel.sorted();
        assert_eq!(s, (3, 2));
        assert_eq!(e, (10, 5));
    }

    #[test]
    fn test_selection_contains_single_line() {
        let sel = TextSelection {
            start: (2, 3),
            end: (2, 8),
            ..Default::default()
        };

        assert!(sel.contains(2, 3));
        assert!(sel.contains(2, 5));
        assert!(sel.contains(2, 7));
        assert!(!sel.contains(2, 8)); // exclusive end
        assert!(!sel.contains(2, 2));
        assert!(!sel.contains(1, 5));
        assert!(!sel.contains(3, 5));
    }

    #[test]
    fn test_selection_contains_multi_line() {
        let sel = TextSelection {
            start: (1, 5),
            end: (3, 4),
            ..Default::default()
        };

        // Row 1: col >= 5
        assert!(!sel.contains(1, 4));
        assert!(sel.contains(1, 5));
        assert!(sel.contains(1, 20));

        // Row 2: fully selected
        assert!(sel.contains(2, 0));
        assert!(sel.contains(2, 100));

        // Row 3: col < 4
        assert!(sel.contains(3, 0));
        assert!(sel.contains(3, 3));
        assert!(!sel.contains(3, 4));

        // Outside
        assert!(!sel.contains(0, 0));
        assert!(!sel.contains(4, 0));
    }

    #[test]
    fn test_extract_text_single_line() {
        let lines = vec![
            SelectionRow::simple("Hello, world!".to_string()),
            SelectionRow::simple("Second line".to_string()),
        ];
        let sel = TextSelection {
            start: (0, 7),
            end: (0, 12),
            ..Default::default()
        };
        assert_eq!(extract_text(&lines, &sel), "world");
    }

    #[test]
    fn test_extract_text_multi_line() {
        let lines = vec![
            SelectionRow::simple("Line zero".to_string()),
            SelectionRow::simple("First line here".to_string()),
            SelectionRow::simple("Second line here".to_string()),
            SelectionRow::simple("Third line here".to_string()),
        ];
        let sel = TextSelection {
            start: (1, 6),
            end: (3, 5),
            ..Default::default()
        };
        let text = extract_text(&lines, &sel);
        assert_eq!(text, "line here\nSecond line here\nThird");
    }

    #[test]
    fn test_extract_text_skips_out_of_range_start_row() {
        let lines = vec![
            SelectionRow::simple("short".to_string()),
            SelectionRow::simple("Second line here".to_string()),
        ];
        let sel = TextSelection {
            start: (0, 10),
            end: (1, 6),
            ..Default::default()
        };
        // Row 0's start column is beyond its content; it must not emit a
        // spurious leading blank line.
        assert_eq!(extract_text(&lines, &sel), "Second");
    }

    #[test]
    fn test_extract_text_preserves_interior_blank_lines() {
        let lines = vec![
            SelectionRow::simple("aaa".to_string()),
            SelectionRow::default(),
            SelectionRow::simple("ccc".to_string()),
        ];
        let sel = TextSelection {
            start: (0, 0),
            end: (2, 3),
            ..Default::default()
        };
        assert_eq!(extract_text(&lines, &sel), "aaa\n\nccc");
    }

    #[test]
    fn test_extract_text_empty_selection() {
        let lines = vec![SelectionRow::simple("Hello".to_string())];
        let sel = TextSelection::default();
        assert_eq!(extract_text(&lines, &sel), "");
    }

    /// Code-block rows display a gutter (`  1 │ `) that the clipboard must
    /// never include; display coordinates map back onto the raw code line.
    #[test]
    fn test_extract_text_code_gutter_stays_out_of_clipboard() {
        let row = SelectionRow {
            display: "  1 │ fn main()".to_string(),
            copy: "fn main()".to_string(),
            display_start: 0,
            copy_offset: 6,
        };

        // Full-row drag (from the gutter to past the line end).
        let full = TextSelection {
            start: (0, 0),
            end: (0, 15),
            ..Default::default()
        };
        assert_eq!(extract_text(std::slice::from_ref(&row), &full), "fn main()");

        // Drag starting on the code itself.
        let code_only = TextSelection {
            start: (0, 6),
            end: (0, 8),
            ..Default::default()
        };
        assert_eq!(extract_text(&[row], &code_only), "fn");
    }

    /// A wrapped continuation row slices the raw copy text by its display
    /// start, after subtracting the decorative prefix.
    #[test]
    fn test_extract_text_wrapped_code_continuation_row() {
        let rows = vec![
            SelectionRow {
                display: "  1 │ fn ma".to_string(),
                copy: "fn main() {}".to_string(),
                display_start: 0,
                copy_offset: 6,
            },
            SelectionRow {
                display: "in() {}".to_string(),
                copy: "fn main() {}".to_string(),
                display_start: 11,
                copy_offset: 6,
            },
        ];
        let sel = TextSelection {
            start: (0, 0),
            end: (1, 6),
            ..Default::default()
        };
        assert_eq!(extract_text(&rows, &sel), "fn ma\nin() {");
    }

    #[test]
    fn test_reset_clears() {
        let mut sel = TextSelection::default();
        sel.start(5, 3);
        sel.update(10, 20);
        sel.reset();
        assert!(sel.is_empty());
        assert!(!sel.active);
    }
}
