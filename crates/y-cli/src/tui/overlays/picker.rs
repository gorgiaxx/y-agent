//! Shared scaffolding for the searchable picker overlays.
//!
//! [`PickerState`] owns the query string, the indices of items matching the
//! query, and the selection cursor, so each concrete picker only supplies its
//! items and a match predicate via the [`PickerItem`] trait. `visible_range`,
//! `truncate`, and `preview` are shared rendering helpers.

use std::ops::Range;

/// An item that can be filtered by the picker query.
pub trait PickerItem {
    /// Whether the item matches `query_lower`.
    ///
    /// The query is already lowercased by the picker and is guaranteed to be
    /// non-empty; implementations typically compare against precomputed
    /// lowercase fields so per-keystroke filtering only runs `contains`.
    fn matches(&self, query_lower: &str) -> bool;
}

/// Generic state for a searchable list picker: the items, the indices of
/// items matching the current query, and the selected cursor within that
/// filtered view.
#[derive(Debug, Clone)]
pub struct PickerState<T: PickerItem> {
    items: Vec<T>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
}

impl<T: PickerItem> Default for PickerState<T> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<T: PickerItem> PickerState<T> {
    /// Create a picker over `items` with an empty query (everything matches).
    pub fn new(items: Vec<T>) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            items,
            filtered,
            selected: 0,
            query: String::new(),
        }
    }

    /// All items, in their original order.
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Indices of the items matching the current query.
    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    /// Number of items matching the current query.
    pub fn filtered_len(&self) -> usize {
        self.filtered.len()
    }

    /// Cursor position within the filtered view.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Set the cursor position within the filtered view, e.g. to preselect an
    /// active entry before the picker opens.
    ///
    /// Out-of-range values are clamped by the next `select_*` call and by
    /// [`visible_range`] at render time.
    pub fn set_selected(&mut self, selected: usize) {
        self.selected = selected;
    }

    /// The current query as typed by the user.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The selected item, if the filtered view is non-empty.
    pub fn selected_item(&self) -> Option<&T> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    /// Move the selection up by one row.
    pub fn select_prev(&mut self) {
        self.page_prev(1);
    }

    /// Move the selection down by one row.
    pub fn select_next(&mut self) {
        self.page_next(1);
    }

    /// Move the selection up by `page` rows, clamping at the first match.
    pub fn page_prev(&mut self, page: usize) {
        self.selected = self.selected.saturating_sub(page.max(1));
    }

    /// Move the selection down by `page` rows, clamping at the last match.
    pub fn page_next(&mut self, page: usize) {
        self.selected = self
            .selected
            .saturating_add(page.max(1))
            .min(self.filtered.len().saturating_sub(1));
    }

    /// Append a character to the query and recompute the filter.
    pub fn push_char(&mut self, character: char) {
        self.query.push(character);
        self.update_filter();
    }

    /// Remove the last query character and recompute the filter.
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.update_filter();
    }

    /// Recompute the filtered indices and reset the cursor to the first match.
    fn update_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        if query.is_empty() {
            self.filtered = (0..self.items.len()).collect();
        } else {
            self.filtered = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.matches(&query))
                .map(|(index, _)| index)
                .collect();
        }
        self.selected = 0;
    }
}

/// Rows of the filtered list visible in a viewport of `height` rows, keeping
/// the selected row on screen.
pub fn visible_range(item_count: usize, selected: usize, height: usize) -> Range<usize> {
    if item_count == 0 || height == 0 {
        return 0..0;
    }
    let selected = selected.min(item_count - 1);
    let start = selected.saturating_add(1).saturating_sub(height);
    let end = start.saturating_add(height).min(item_count);
    start..end
}

/// Truncate `value` to at most `max_chars` characters, ending with an
/// ellipsis when truncated.
pub fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!(
            "{}...",
            truncated
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    } else {
        truncated
    }
}

/// Collapse every whitespace run in `value` to a single space, then truncate
/// to `max_chars` characters like [`truncate`].
pub fn preview(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&normalized, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test item with a precomputed lowercase haystack, mirroring the Entry
    /// wrappers used by the concrete pickers.
    #[derive(Debug, Clone)]
    struct TestEntry {
        label: String,
        label_lower: String,
    }

    impl TestEntry {
        fn new(label: &str) -> Self {
            Self {
                label: label.to_string(),
                label_lower: label.to_ascii_lowercase(),
            }
        }
    }

    impl PickerItem for TestEntry {
        fn matches(&self, query_lower: &str) -> bool {
            self.label_lower.contains(query_lower)
        }
    }

    fn picker(labels: &[&str]) -> PickerState<TestEntry> {
        PickerState::new(labels.iter().map(|label| TestEntry::new(label)).collect())
    }

    #[test]
    fn new_picker_matches_everything() {
        let state = picker(&["alpha", "beta"]);
        assert_eq!(state.filtered(), &[0, 1]);
        assert_eq!(state.selected(), 0);
        assert!(state.query().is_empty());
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let mut state = picker(&["alpha", "beta", "gamma"]);

        state.select_prev();
        assert_eq!(state.selected(), 0);

        state.select_next();
        state.select_next();
        state.select_next();
        assert_eq!(state.selected(), 2);

        state.select_prev();
        assert_eq!(state.selected(), 1);
    }

    #[test]
    fn page_navigation_clamps_at_both_ends() {
        let mut state = picker(&["a", "b", "c", "d", "e"]);

        state.page_next(3);
        assert_eq!(state.selected(), 3);
        state.page_next(10);
        assert_eq!(state.selected(), 4);
        state.page_prev(2);
        assert_eq!(state.selected(), 2);
        state.page_prev(10);
        assert_eq!(state.selected(), 0);
    }

    #[test]
    fn page_navigation_treats_zero_page_as_one_row() {
        let mut state = picker(&["a", "b", "c"]);
        state.page_next(0);
        assert_eq!(state.selected(), 1);
        state.page_prev(0);
        assert_eq!(state.selected(), 0);
    }

    #[test]
    fn navigation_clamps_after_filter_narrows() {
        let mut state = picker(&["alpha", "beta"]);
        state.select_next();
        assert_eq!(state.selected(), 1);

        for character in "alpha".chars() {
            state.push_char(character);
        }
        assert_eq!(state.selected(), 0);
        state.select_next();
        assert_eq!(state.selected(), 0, "single match must clamp the cursor");
    }

    #[test]
    fn filter_recomputes_on_edit_and_resets_cursor() {
        let mut state = picker(&["alpha", "beta", "alpine"]);
        state.select_next();

        state.push_char('a');
        state.push_char('l');
        assert_eq!(state.filtered(), &[0, 2]);
        assert_eq!(state.selected(), 0, "editing the query resets the cursor");

        state.pop_char();
        state.pop_char();
        assert_eq!(state.filtered(), &[0, 1, 2]);
    }

    #[test]
    fn filter_matches_case_insensitively() {
        let mut state = picker(&["Alpha", "beta"]);
        for character in "ALPHA".chars() {
            state.push_char(character);
        }
        assert_eq!(state.filtered(), &[0]);
    }

    #[test]
    fn selected_item_follows_filtered_view() {
        let mut state = picker(&["alpha", "beta"]);
        for character in "beta".chars() {
            state.push_char(character);
        }
        assert_eq!(
            state.selected_item().map(|entry| entry.label.as_str()),
            Some("beta")
        );

        state.push_char('z');
        assert!(state.selected_item().is_none());
    }

    #[test]
    fn set_selected_preselects_a_row() {
        let mut state = picker(&["alpha", "beta"]);
        state.set_selected(1);
        assert_eq!(
            state.selected_item().map(|entry| entry.label.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn default_picker_is_empty() {
        let state: PickerState<TestEntry> = PickerState::default();
        assert_eq!(state.filtered_len(), 0);
        assert!(state.selected_item().is_none());
    }

    #[test]
    fn visible_range_edge_cases() {
        assert_eq!(visible_range(0, 0, 5), 0..0, "no items");
        assert_eq!(visible_range(10, 3, 0), 0..0, "no viewport height");
        assert_eq!(
            visible_range(3, 2, 5),
            0..3,
            "viewport taller than the list"
        );
        assert_eq!(
            visible_range(10, 99, 4),
            6..10,
            "selection past the end clamps"
        );
        assert_eq!(visible_range(20, 0, 5), 0..5);
        assert_eq!(visible_range(20, 4, 5), 0..5);
        assert_eq!(visible_range(20, 5, 5), 1..6);
        assert_eq!(visible_range(20, 19, 5), 15..20);
    }

    #[test]
    fn truncate_appends_ellipsis_only_when_needed() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactlyten", 10), "exactlyten");
        assert_eq!(truncate("a much longer title", 10), "a much ...");
        assert_eq!(truncate("abcdefgh", 6), "abc...");
        assert_eq!(truncate("abcd", 2), "...");
    }

    #[test]
    fn preview_collapses_whitespace_before_truncating() {
        assert_eq!(preview(" first\n  second ", 40), "first second");
        assert_eq!(preview("abcdefgh", 6), "abc...");
        assert_eq!(preview("  ", 10), "");
    }
}
