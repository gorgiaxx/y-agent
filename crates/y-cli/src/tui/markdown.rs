//! Markdown-to-ratatui renderer with syntax highlighting.
//!
//! Converts a markdown string into a `Vec<Line>` suitable for ratatui rendering.
//! Uses `pulldown-cmark` for parsing and `syntect` + `two-face` for code block
//! syntax highlighting.
//!
//! Supported elements:
//! - Headings (H1-H6) with bold + accent color
//! - Code blocks (fenced) with syntax highlighting
//! - Inline code with distinct background
//! - Bold, italic, strikethrough
//! - Bullet and ordered lists (nested, up to 3 levels)
//! - Blockquotes with left bar accent
//! - Horizontal rules
//! - Links (underlined, show URL)
//! - Plain paragraphs with word wrapping

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

// ---------------------------------------------------------------------------
// Color palette (matches chat.rs for consistency)
// ---------------------------------------------------------------------------

const COLOR_HEADING: Color = Color::Rgb(130, 220, 130);
const COLOR_CODE_FG: Color = Color::Rgb(200, 200, 220);
const COLOR_CODE_BG: Color = Color::Rgb(40, 42, 54);
const COLOR_INLINE_CODE_FG: Color = Color::Rgb(220, 180, 120);
const COLOR_INLINE_CODE_BG: Color = Color::Rgb(50, 52, 64);
const COLOR_BOLD: Color = Color::Rgb(240, 240, 255);
const COLOR_ITALIC: Color = Color::Rgb(200, 200, 230);
const COLOR_LINK: Color = Color::Rgb(100, 180, 255);
const COLOR_BLOCKQUOTE_BAR: Color = Color::Rgb(100, 120, 160);
const COLOR_BLOCKQUOTE_TEXT: Color = Color::Rgb(180, 180, 200);
const COLOR_HR: Color = Color::Rgb(60, 60, 80);
const COLOR_LIST_BULLET: Color = Color::Rgb(120, 160, 200);
/// Table border / separator color.
const COLOR_TABLE_BORDER: Color = Color::Rgb(80, 90, 120);
/// Table header text color.
const COLOR_TABLE_HEADER: Color = Color::Rgb(200, 210, 240);

// ---------------------------------------------------------------------------
// Syntax highlighting singleton
// ---------------------------------------------------------------------------

/// Lazily-initialized syntax set and theme for code highlighting.
struct HighlightState {
    syntax_set: SyntaxSet,
    theme: Theme,
}

fn highlight_state() -> &'static HighlightState {
    use std::sync::OnceLock;
    static STATE: OnceLock<HighlightState> = OnceLock::new();
    STATE.get_or_init(|| {
        let syntax_set = two_face::syntax::extra_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set.themes["base16-ocean.dark"].clone();
        HighlightState { syntax_set, theme }
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a markdown string into ratatui `Line`s.
///
/// `width` is the available column width for word wrapping.
pub fn render_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, opts);
    let mut renderer = MdRenderer::new(width);
    renderer.process(parser);
    renderer.finish()
}

// ---------------------------------------------------------------------------
// Renderer style flags (avoids excessive bools in struct)
// ---------------------------------------------------------------------------

/// Bit flags for inline style state in the markdown renderer.
#[derive(Clone, Copy, Default)]
struct StyleFlags(u8);

impl StyleFlags {
    const BOLD: u8 = 1 << 0;
    const ITALIC: u8 = 1 << 1;
    const STRIKETHROUGH: u8 = 1 << 2;
    const INLINE_CODE: u8 = 1 << 3;
    const CODE_BLOCK: u8 = 1 << 4;
    const LINK: u8 = 1 << 5;

    fn set(&mut self, flag: u8) {
        self.0 |= flag;
    }
    fn unset(&mut self, flag: u8) {
        self.0 &= !flag;
    }
    fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }
}

// ---------------------------------------------------------------------------
// Internal renderer state machine
// ---------------------------------------------------------------------------

struct MdRenderer {
    lines: Vec<Line<'static>>,
    /// Current line being built (accumulated spans).
    current_spans: Vec<Span<'static>>,
    /// Current column position for word wrapping.
    col: usize,
    /// Available width for wrapping.
    width: usize,
    /// Inline style flags (bold, italic, strikethrough, code, link).
    flags: StyleFlags,
    /// Language hint for the current code block.
    code_lang: String,
    /// Accumulated code block text.
    code_buffer: String,
    /// List nesting depth (0 = not in list).
    list_depth: usize,
    /// Whether the current list is ordered (per depth level).
    list_ordered: Vec<bool>,
    /// Item counters for ordered lists (per depth level).
    list_counters: Vec<u64>,
    /// Blockquote nesting depth.
    blockquote_depth: usize,
    /// Current heading level (0 = not in heading).
    heading_level: u8,
    /// Link URL being accumulated.
    link_url: String,
    /// Table state: accumulated rows (each row is a vec of cell strings).
    table_rows: Vec<Vec<String>>,
    /// Number of header rows in the current table (typically 1).
    table_header_count: usize,
    /// Current cell text being accumulated.
    table_cell_buf: String,
    /// Current row cells being accumulated.
    table_row_buf: Vec<String>,
    /// Whether we are inside a table.
    in_table: bool,
    /// Whether we are inside the table header section.
    in_table_head: bool,
}

impl MdRenderer {
    fn new(width: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            col: 0,
            width: width.max(20),
            flags: StyleFlags::default(),
            code_lang: String::new(),
            code_buffer: String::new(),
            list_depth: 0,
            list_ordered: Vec::new(),
            list_counters: Vec::new(),
            blockquote_depth: 0,
            heading_level: 0,
            link_url: String::new(),
            table_rows: Vec::new(),
            table_header_count: 0,
            table_cell_buf: String::new(),
            table_row_buf: Vec::new(),
            in_table: false,
            in_table_head: false,
        }
    }

    fn process(&mut self, parser: Parser<'_>) {
        for event in parser {
            match event {
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                Event::Text(text) => self.push_text(&text),
                Event::Code(code) => self.push_inline_code(&code),
                Event::SoftBreak => self.push_text(" "),
                Event::HardBreak => self.flush_line(),
                Event::Rule => self.push_rule(),
                _ => {}
            }
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                self.heading_level = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    pulldown_cmark::HeadingLevel::H3 => 3,
                    pulldown_cmark::HeadingLevel::H4 => 4,
                    pulldown_cmark::HeadingLevel::H5 => 5,
                    pulldown_cmark::HeadingLevel::H6 => 6,
                };
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.flags.set(StyleFlags::CODE_BLOCK);
                self.code_buffer.clear();
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => String::new(),
                };
            }
            Tag::Emphasis => self.flags.set(StyleFlags::ITALIC),
            Tag::Strong => self.flags.set(StyleFlags::BOLD),
            Tag::Strikethrough => self.flags.set(StyleFlags::STRIKETHROUGH),
            Tag::Link { dest_url, .. } => {
                self.flags.set(StyleFlags::LINK);
                self.link_url = dest_url.to_string();
            }
            Tag::List(start) => {
                if self.list_depth == 0 {
                    self.flush_line();
                }
                self.list_depth += 1;
                let ordered = start.is_some();
                self.list_ordered.push(ordered);
                self.list_counters.push(start.unwrap_or(1));
            }
            Tag::Item => {
                self.flush_line();
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                self.blockquote_depth += 1;
            }
            Tag::Paragraph => {
                // Add blank line before paragraph unless we are in a list item
                // or inside a table (tables suppress paragraph spacing).
                if self.list_depth == 0 && !self.in_table && !self.lines.is_empty() {
                    self.lines.push(Line::from(""));
                }
            }
            Tag::Table(_alignments) => {
                self.flush_line();
                self.in_table = true;
                self.table_rows.clear();
                self.table_header_count = 0;
            }
            Tag::TableHead => {
                self.in_table_head = true;
            }
            Tag::TableRow => {
                self.table_row_buf.clear();
            }
            Tag::TableCell => {
                self.table_cell_buf.clear();
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush_line();
                self.heading_level = 0;
            }
            TagEnd::CodeBlock => {
                self.render_code_block();
                self.flags.unset(StyleFlags::CODE_BLOCK);
                self.code_lang.clear();
                self.code_buffer.clear();
            }
            TagEnd::Emphasis => self.flags.unset(StyleFlags::ITALIC),
            TagEnd::Strong => self.flags.unset(StyleFlags::BOLD),
            TagEnd::Strikethrough => self.flags.unset(StyleFlags::STRIKETHROUGH),
            TagEnd::Link => {
                // Append URL indicator after link text.
                if !self.link_url.is_empty() {
                    let url_span = Span::styled(
                        format!(" ({})", self.link_url),
                        Style::default().fg(Color::Rgb(80, 80, 100)),
                    );
                    self.current_spans.push(url_span);
                }
                self.flags.unset(StyleFlags::LINK);
                self.link_url.clear();
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.list_ordered.pop();
                self.list_counters.pop();
                if self.list_depth == 0 {
                    self.flush_line();
                }
            }
            TagEnd::Item => {
                self.flush_line();
                // Increment ordered list counter.
                if let Some(counter) = self.list_counters.last_mut() {
                    *counter += 1;
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::Paragraph => {
                self.flush_line();
            }
            TagEnd::Table => {
                self.render_table();
                self.in_table = false;
                self.table_rows.clear();
                self.table_header_count = 0;
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.table_header_count = self.table_rows.len();
            }
            TagEnd::TableRow => {
                self.table_rows
                    .push(std::mem::take(&mut self.table_row_buf));
            }
            TagEnd::TableCell => {
                self.table_row_buf
                    .push(std::mem::take(&mut self.table_cell_buf));
            }
            _ => {}
        }
    }

    fn push_text(&mut self, text: &str) {
        if self.flags.has(StyleFlags::CODE_BLOCK) {
            self.code_buffer.push_str(text);
            return;
        }
        // Inside a table cell: accumulate text into the cell buffer.
        if self.in_table {
            self.table_cell_buf.push_str(text);
            return;
        }

        let style = self.current_style();

        // Prepend list bullet/number if this is the first text in an item.
        if self.list_depth > 0 && self.current_spans.is_empty() && self.col == 0 {
            let indent = "  ".repeat(self.list_depth.saturating_sub(1));
            let is_ordered = self.list_ordered.last().copied().unwrap_or(false);
            let bullet = if is_ordered {
                let num = self.list_counters.last().copied().unwrap_or(1);
                format!("{indent}{num}. ")
            } else {
                format!("{indent}- ")
            };
            let bullet_len = bullet.len();
            self.current_spans
                .push(Span::styled(bullet, Style::default().fg(COLOR_LIST_BULLET)));
            self.col += bullet_len;
        }

        // Blockquote prefix.
        if self.blockquote_depth > 0 && self.current_spans.is_empty() && self.col == 0 {
            let prefix = "| ".repeat(self.blockquote_depth);
            let prefix_len = prefix.len();
            self.current_spans.push(Span::styled(
                prefix,
                Style::default().fg(COLOR_BLOCKQUOTE_BAR),
            ));
            self.col += prefix_len;
        }

        // Word-wrap the text.
        for word in text.split_inclusive(' ') {
            let word_len = unicode_display_width(word);
            if self.col + word_len > self.width && self.col > 0 {
                self.flush_line();
                // Re-apply indentation for continuation lines.
                if self.list_depth > 0 {
                    let indent = "  ".repeat(self.list_depth.saturating_sub(1)) + "  ";
                    let indent_len = indent.len();
                    self.current_spans.push(Span::raw(indent));
                    self.col += indent_len;
                }
                if self.blockquote_depth > 0 {
                    let prefix = "| ".repeat(self.blockquote_depth);
                    let prefix_len = prefix.len();
                    self.current_spans.push(Span::styled(
                        prefix,
                        Style::default().fg(COLOR_BLOCKQUOTE_BAR),
                    ));
                    self.col += prefix_len;
                }
            }
            self.current_spans
                .push(Span::styled(word.to_string(), style));
            self.col += word_len;
        }
    }

    fn push_inline_code(&mut self, code: &str) {
        // Inside a table cell: accumulate as text (styling not applicable).
        if self.in_table {
            self.table_cell_buf.push_str(code);
            return;
        }
        let style = Style::default()
            .fg(COLOR_INLINE_CODE_FG)
            .bg(COLOR_INLINE_CODE_BG);
        let text = format!(" {code} ");
        let len = unicode_display_width(&text);
        self.current_spans.push(Span::styled(text, style));
        self.col += len;
    }

    fn push_rule(&mut self) {
        self.flush_line();
        let rule_text = "\u{2500}".repeat(self.width.min(60));
        self.lines.push(Line::from(Span::styled(
            rule_text,
            Style::default().fg(COLOR_HR),
        )));
    }

    fn current_style(&self) -> Style {
        if self.heading_level > 0 {
            let mut style = Style::default()
                .fg(COLOR_HEADING)
                .add_modifier(Modifier::BOLD);
            if self.heading_level <= 2 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            return style;
        }

        if self.flags.has(StyleFlags::LINK) {
            return Style::default()
                .fg(COLOR_LINK)
                .add_modifier(Modifier::UNDERLINED);
        }

        if self.blockquote_depth > 0 {
            let mut style = Style::default().fg(COLOR_BLOCKQUOTE_TEXT);
            if self.flags.has(StyleFlags::ITALIC) {
                style = style.add_modifier(Modifier::ITALIC);
            }
            return style;
        }

        let mut style = Style::default();
        if self.flags.has(StyleFlags::BOLD) {
            style = style.fg(COLOR_BOLD).add_modifier(Modifier::BOLD);
        }
        if self.flags.has(StyleFlags::ITALIC) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.flags.has(StyleFlags::STRIKETHROUGH) {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }

    fn flush_line(&mut self) {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            self.lines.push(Line::from(spans));
        }
        self.col = 0;
    }

    fn render_code_block(&mut self) {
        let hs = highlight_state();
        let lang = self.code_lang.trim();
        let syntax = if lang.is_empty() {
            hs.syntax_set.find_syntax_plain_text()
        } else {
            hs.syntax_set
                .find_syntax_by_token(lang)
                .unwrap_or_else(|| hs.syntax_set.find_syntax_plain_text())
        };

        // Try syntax-highlighted rendering.
        let highlighted = highlight_code(&hs.syntax_set, &hs.theme, syntax, &self.code_buffer);

        // Language label.
        if !lang.is_empty() {
            self.lines.push(Line::from(Span::styled(
                format!("  {lang}"),
                Style::default()
                    .fg(Color::Rgb(100, 100, 130))
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        for line_spans in highlighted {
            let mut spans = vec![Span::styled("  ", Style::default().bg(COLOR_CODE_BG))];
            spans.extend(line_spans);
            self.lines.push(Line::from(spans));
        }
    }

    /// Render accumulated table rows into styled lines.
    ///
    /// Layout:
    /// ```text
    ///   Header1  | Header2  | Header3
    ///   ---------+----------+---------
    ///   Cell1    | Cell2    | Cell3
    /// ```
    fn render_table(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }

        // Determine column count and compute column widths.
        let col_count = self.table_rows.iter().map(Vec::len).max().unwrap_or(0);
        if col_count == 0 {
            return;
        }

        let mut col_widths: Vec<usize> = vec![0; col_count];
        for row in &self.table_rows {
            for (i, cell) in row.iter().enumerate() {
                let w = unicode_display_width(cell.trim());
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }

        // Clamp column widths so the total table fits within the available width.
        // Reserve 3 chars per separator (" | ") and 2 for leading indent.
        let separators_width = if col_count > 1 {
            (col_count - 1) * 3
        } else {
            0
        };
        let indent_width = 2;
        let available = self.width.saturating_sub(indent_width + separators_width);
        let total_col_width: usize = col_widths.iter().sum();
        if total_col_width > available && available > 0 {
            let scale = available as f64 / total_col_width as f64;
            for w in &mut col_widths {
                *w = ((*w as f64 * scale).floor() as usize).max(3);
            }
        }

        let border_style = Style::default().fg(COLOR_TABLE_BORDER);
        let header_style = Style::default()
            .fg(COLOR_TABLE_HEADER)
            .add_modifier(Modifier::BOLD);
        let cell_style = Style::default().fg(Color::Rgb(200, 200, 220));

        for (row_idx, row) in self.table_rows.iter().enumerate() {
            let is_header = row_idx < self.table_header_count;
            let style = if is_header { header_style } else { cell_style };

            let mut spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
            for (col_idx, width) in col_widths.iter().enumerate() {
                if col_idx > 0 {
                    spans.push(Span::styled(" | ".to_string(), border_style));
                }
                let cell_text = row.get(col_idx).map_or("", |s| s.trim());
                let padded = pad_or_truncate(cell_text, *width);
                spans.push(Span::styled(padded, style));
            }
            self.lines.push(Line::from(spans));

            // Render separator line after the header row(s).
            if is_header && row_idx + 1 == self.table_header_count {
                let mut sep_spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
                for (col_idx, width) in col_widths.iter().enumerate() {
                    if col_idx > 0 {
                        sep_spans.push(Span::styled("-+-".to_string(), border_style));
                    }
                    sep_spans.push(Span::styled("-".repeat(*width), border_style));
                }
                self.lines.push(Line::from(sep_spans));
            }
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        self.lines
    }
}

// ---------------------------------------------------------------------------
// Syntax highlighting via syntect
// ---------------------------------------------------------------------------

fn highlight_code(
    syntax_set: &SyntaxSet,
    theme: &Theme,
    syntax: &syntect::parsing::SyntaxReference,
    code: &str,
) -> Vec<Vec<Span<'static>>> {
    use syntect::easy::HighlightLines;

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut result = Vec::new();

    for line in code.lines() {
        let ranges = highlighter
            .highlight_line(line, syntax_set)
            .unwrap_or_default();

        let spans: Vec<Span<'static>> = ranges
            .into_iter()
            .map(|(style, text)| {
                let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                Span::styled(text.to_string(), Style::default().fg(fg).bg(COLOR_CODE_BG))
            })
            .collect();

        result.push(spans);
    }

    result
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the display width of a string using unicode widths.
fn unicode_display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

/// Pad a string to `width` display columns, or truncate with ellipsis.
fn pad_or_truncate(s: &str, width: usize) -> String {
    let display_w = unicode_display_width(s);
    if display_w <= width {
        let padding = width - display_w;
        format!("{s}{}", " ".repeat(padding))
    } else if width > 3 {
        // Truncate by characters until we fit.
        let mut buf = String::new();
        let mut w = 0;
        for ch in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if w + cw > width - 3 {
                break;
            }
            buf.push(ch);
            w += cw;
        }
        let remaining = width - w;
        format!("{buf}{}", ".".repeat(remaining))
    } else {
        ".".repeat(width)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // T-MD-01: Plain text renders as single line.
    #[test]
    fn test_plain_text() {
        let lines = render_markdown("Hello world", 80);
        assert!(!lines.is_empty());
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("Hello world"));
    }

    // T-MD-02: Heading renders with bold modifier.
    #[test]
    fn test_heading() {
        let lines = render_markdown("# Title", 80);
        assert!(!lines.is_empty());
        let has_bold = lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        });
        assert!(has_bold, "heading should be bold");
    }

    // T-MD-03: Code block renders with background color.
    #[test]
    fn test_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let lines = render_markdown(md, 80);
        assert!(lines.len() >= 2, "code block should produce lines");
        // Code lines should have a background color set.
        let has_bg = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.style.bg.is_some()));
        assert!(has_bg, "code block should have background color");
    }

    // T-MD-04: Inline code renders distinctly.
    #[test]
    fn test_inline_code() {
        let lines = render_markdown("Use `foo()` here", 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("foo()"));
    }

    // T-MD-05: Bullet list renders with dash prefix.
    #[test]
    fn test_bullet_list() {
        let md = "- item one\n- item two";
        let lines = render_markdown(md, 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("- "), "bullet list should have dash prefix");
    }

    // T-MD-06: Ordered list renders with numbers.
    #[test]
    fn test_ordered_list() {
        let md = "1. first\n2. second";
        let lines = render_markdown(md, 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            text.contains("1. "),
            "ordered list should have number prefix"
        );
    }

    // T-MD-07: Horizontal rule renders.
    #[test]
    fn test_horizontal_rule() {
        let md = "above\n\n---\n\nbelow";
        let lines = render_markdown(md, 80);
        let has_rule = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains('\u{2500}')));
        assert!(has_rule, "should contain horizontal rule character");
    }

    // T-MD-08: Bold text has BOLD modifier.
    #[test]
    fn test_bold() {
        let lines = render_markdown("**bold text**", 80);
        let has_bold = lines.iter().any(|l| {
            l.spans.iter().any(|s| {
                s.style.add_modifier.contains(Modifier::BOLD) && s.content.contains("bold")
            })
        });
        assert!(has_bold, "bold text should have BOLD modifier");
    }

    // T-MD-09: Word wrapping at width boundary.
    #[test]
    fn test_word_wrap() {
        let md = "word1 word2 word3 word4 word5";
        let lines = render_markdown(md, 15);
        assert!(lines.len() > 1, "text should wrap at narrow width");
    }

    // T-MD-10: Empty input produces empty output.
    #[test]
    fn test_empty() {
        let lines = render_markdown("", 80);
        assert!(lines.is_empty());
    }
}
