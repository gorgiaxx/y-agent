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

/// Extract selected text from rendered chat lines.
///
/// `lines` should be the flat list of content-line strings (without ANSI
/// styling) that the chat panel has rendered. The selection coordinates
/// are indices into this list.
pub fn extract_text(lines: &[String], selection: &TextSelection) -> String {
    if selection.is_empty() || lines.is_empty() {
        return String::new();
    }

    let ((sr, sc), (er, ec)) = selection.sorted();
    let mut result = Vec::new();

    for row in sr..=er.min(lines.len().saturating_sub(1)) {
        let line = &lines[row];
        let chars: Vec<char> = line.chars().collect();
        let start_col = if row == sr { sc } else { 0 };
        let end_col = if row == er {
            ec.min(chars.len())
        } else {
            chars.len()
        };

        if start_col >= chars.len() {
            result.push(String::new());
            continue;
        }

        let selected: String = chars[start_col..end_col.min(chars.len())].iter().collect();
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
        let mut sel = TextSelection::default();
        sel.start = (10, 5);
        sel.end = (3, 2);
        let (s, e) = sel.sorted();
        assert_eq!(s, (3, 2));
        assert_eq!(e, (10, 5));
    }

    #[test]
    fn test_selection_contains_single_line() {
        let mut sel = TextSelection::default();
        sel.start = (2, 3);
        sel.end = (2, 8);

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
        let mut sel = TextSelection::default();
        sel.start = (1, 5);
        sel.end = (3, 4);

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
            "Hello, world!".to_string(),
            "Second line".to_string(),
        ];
        let mut sel = TextSelection::default();
        sel.start = (0, 7);
        sel.end = (0, 12);
        assert_eq!(extract_text(&lines, &sel), "world");
    }

    #[test]
    fn test_extract_text_multi_line() {
        let lines = vec![
            "Line zero".to_string(),
            "First line here".to_string(),
            "Second line here".to_string(),
            "Third line here".to_string(),
        ];
        let mut sel = TextSelection::default();
        sel.start = (1, 6);
        sel.end = (3, 5);
        let text = extract_text(&lines, &sel);
        assert_eq!(text, "line here\nSecond line here\nThird");
    }

    #[test]
    fn test_extract_text_empty_selection() {
        let lines = vec!["Hello".to_string()];
        let sel = TextSelection::default();
        assert_eq!(extract_text(&lines, &sel), "");
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
