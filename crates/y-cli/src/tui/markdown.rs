//! Markdown-to-ratatui renderer with syntax highlighting.
//!
//! Converts a markdown string into a `Vec<Line>` suitable for ratatui rendering.
//! Uses `pulldown-cmark` for parsing and `syntect` + `two-face` for code block
//! syntax highlighting.
//!
//! Supported elements:
//! - Headings (H1-H6) with bold + accent color
//! - Code blocks (fenced) with syntax highlighting, a line-number gutter,
//!   and a uniform background band
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
use syntect::highlighting::{Theme as SyntectTheme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::tui::theme::Theme as TuiTheme;

// ---------------------------------------------------------------------------
// Terminal-aware color palette (lazy-initialized from TuiTheme)
// ---------------------------------------------------------------------------

fn tui_theme() -> &'static TuiTheme {
    use std::sync::OnceLock;
    static THEME: OnceLock<TuiTheme> = OnceLock::new();
    THEME.get_or_init(TuiTheme::default)
}

fn color_heading() -> Color {
    tui_theme().user_accent()
}
fn color_code_bg() -> Color {
    tui_theme().code_bg()
}
fn color_inline_code_fg() -> Color {
    tui_theme().code_fg()
}
fn color_inline_code_bg() -> Color {
    tui_theme().code_bg()
}
fn color_bold() -> Color {
    tui_theme().text()
}
fn color_link() -> Color {
    tui_theme().assistant_accent()
}
fn color_blockquote_bar() -> Color {
    tui_theme().blockquote()
}
fn color_blockquote_text() -> Color {
    tui_theme().normal()
}
fn color_hr() -> Color {
    tui_theme().hr()
}
fn color_list_bullet() -> Color {
    tui_theme().assistant_accent()
}
fn color_table_border() -> Color {
    tui_theme().muted()
}
fn color_table_header() -> Color {
    tui_theme().text()
}
fn color_line_number() -> Color {
    tui_theme().muted()
}

// ---------------------------------------------------------------------------
// Rendered line (display + copy text)
// ---------------------------------------------------------------------------

/// One rendered markdown line: the styled line for display plus the plain
/// text that selection/copy should extract.
pub struct RenderedLine {
    /// Styled line for display.
    pub line: Line<'static>,
    /// Text handed to the plain-line buffer (selection, clipboard).
    /// Decorative spans — the code-block line-number gutter, the block
    /// indent, and background band padding — are excluded.
    pub copy_text: String,
    /// Leading display chars excluded from `copy_text` (the code-block
    /// gutter width). Selection coordinates are display-based; extraction
    /// subtracts this offset to land in copy space.
    pub copy_offset: usize,
}

// ---------------------------------------------------------------------------
// Syntax highlighting singleton
// ---------------------------------------------------------------------------

/// Lazily-initialized syntax set and theme for code highlighting.
struct HighlightState {
    syntax_set: SyntaxSet,
    theme: SyntectTheme,
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
pub fn render_markdown(text: &str, width: usize) -> Vec<RenderedLine> {
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
    /// Per-line copy-text overrides, parallel to `lines`. `None` means the
    /// copy text is the concatenation of the line's span contents.
    copy_overrides: Vec<Option<(String, usize)>>,
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
            copy_overrides: Vec::new(),
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
                if self.blockquote_depth == 0 {
                    self.ensure_blank_line();
                }
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
                if self.blockquote_depth == 0 {
                    self.ensure_blank_line();
                }
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
                    if self.blockquote_depth == 0 {
                        self.ensure_blank_line();
                    }
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
                if self.blockquote_depth == 0 {
                    self.ensure_blank_line();
                }
                self.blockquote_depth += 1;
            }
            Tag::Paragraph => {
                // Separate paragraphs from preceding blocks unless we are in
                // a list item or inside a table (tables suppress spacing).
                if self.list_depth == 0 && !self.in_table {
                    self.ensure_blank_line();
                }
            }
            Tag::Table(_alignments) => {
                self.flush_line();
                self.ensure_blank_line();
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
                // pulldown-cmark emits header cells directly inside TableHead
                // without a wrapping TableRow, so flush the header row here.
                if !self.table_row_buf.is_empty() {
                    self.table_rows
                        .push(std::mem::take(&mut self.table_row_buf));
                }
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
            self.current_spans.push(Span::styled(
                bullet,
                Style::default().fg(color_list_bullet()),
            ));
            self.col += bullet_len;
        }

        // Blockquote prefix.
        if self.blockquote_depth > 0 && self.current_spans.is_empty() && self.col == 0 {
            let prefix = "| ".repeat(self.blockquote_depth);
            let prefix_len = prefix.len();
            self.current_spans.push(Span::styled(
                prefix,
                Style::default().fg(color_blockquote_bar()),
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
                        Style::default().fg(color_blockquote_bar()),
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
            .fg(color_inline_code_fg())
            .bg(color_inline_code_bg());
        let text = format!(" {code} ");
        let len = unicode_display_width(&text);
        self.current_spans.push(Span::styled(text, style));
        self.col += len;
    }

    fn push_rule(&mut self) {
        self.flush_line();
        if self.blockquote_depth == 0 {
            self.ensure_blank_line();
        }
        let rule_text = "\u{2500}".repeat(self.width.min(60));
        self.push_line(Line::from(Span::styled(
            rule_text,
            Style::default().fg(color_hr()),
        )));
    }

    fn current_style(&self) -> Style {
        if self.heading_level > 0 {
            let mut style = Style::default()
                .fg(color_heading())
                .add_modifier(Modifier::BOLD);
            if self.heading_level <= 2 {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            return style;
        }

        if self.flags.has(StyleFlags::LINK) {
            return Style::default()
                .fg(color_link())
                .add_modifier(Modifier::UNDERLINED);
        }

        if self.blockquote_depth > 0 {
            let mut style = Style::default().fg(color_blockquote_text());
            if self.flags.has(StyleFlags::ITALIC) {
                style = style.add_modifier(Modifier::ITALIC);
            }
            return style;
        }

        let mut style = Style::default();
        if self.flags.has(StyleFlags::BOLD) {
            style = style.fg(color_bold()).add_modifier(Modifier::BOLD);
        }
        if self.flags.has(StyleFlags::ITALIC) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.flags.has(StyleFlags::STRIKETHROUGH) {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }

    /// Push a blank separator line unless the output is empty or already
    /// ends with a blank line, so adjacent block components stay visually
    /// separated without producing double blanks.
    fn ensure_blank_line(&mut self) {
        let ends_blank = self
            .lines
            .last()
            .is_some_and(|line| line.spans.iter().all(|span| span.content.is_empty()));
        if !self.lines.is_empty() && !ends_blank {
            self.push_line(Line::from(""));
        }
    }

    /// Push a finished display line; its copy text is the concatenation of
    /// its span contents.
    fn push_line(&mut self, line: Line<'static>) {
        self.copy_overrides.push(None);
        self.lines.push(line);
    }

    /// Push a finished display line with an explicit copy text that excludes
    /// decorative spans (gutter, indent, background padding). `copy_offset`
    /// is the number of leading display chars those decorations occupy.
    fn push_line_with_copy(&mut self, line: Line<'static>, copy_text: String, copy_offset: usize) {
        self.copy_overrides.push(Some((copy_text, copy_offset)));
        self.lines.push(line);
    }

    fn flush_line(&mut self) {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            self.push_line(Line::from(spans));
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
        let code = std::mem::take(&mut self.code_buffer);
        let highlighted = highlight_code(&hs.syntax_set, &hs.theme, syntax, &code);
        let raw_lines: Vec<&str> = code.lines().collect();

        // Gutter: 2-space block indent + right-aligned line number + separator.
        let number_width = raw_lines.len().max(1).to_string().len();
        let gutter_width = 2 + number_width + 3; // "  " + number + " │ "

        // Uniform background band: as wide as the widest code line, plus one
        // extra column so the band never ends exactly at the text edge.
        let max_code_width = highlighted
            .iter()
            .map(|spans| spans.iter().map(Span::width).sum::<usize>())
            .max()
            .unwrap_or(0);
        let band_width = gutter_width + max_code_width + 1;

        // Language label.
        if !lang.is_empty() {
            self.push_line(Line::from(Span::styled(
                format!("  {lang}"),
                Style::default()
                    .fg(Color::Rgb(100, 100, 130))
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        let gutter_style = Style::default().fg(color_line_number()).bg(color_code_bg());
        let pad_style = Style::default().bg(color_code_bg());

        for (index, line_spans) in highlighted.iter().enumerate() {
            let gutter = format!("  {:>number_width$} │ ", index + 1);
            let copy_offset = gutter.chars().count();
            let code_width: usize = line_spans.iter().map(Span::width).sum();
            let mut spans = vec![Span::styled(gutter, gutter_style)];
            spans.extend(line_spans.iter().cloned());
            let pad_width = band_width - gutter_width - code_width;
            spans.push(Span::styled(" ".repeat(pad_width), pad_style));
            let copy_text = raw_lines
                .get(index)
                .copied()
                .unwrap_or_default()
                .to_string();
            self.push_line_with_copy(Line::from(spans), copy_text, copy_offset);
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
        let table_rows = std::mem::take(&mut self.table_rows);
        let header_count = self.table_header_count;

        // Determine column count and compute column widths.
        let col_count = table_rows.iter().map(Vec::len).max().unwrap_or(0);
        if col_count == 0 {
            return;
        }

        let mut col_widths: Vec<usize> = vec![0; col_count];
        for row in &table_rows {
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

        let border_style = Style::default().fg(color_table_border());
        let header_style = Style::default()
            .fg(color_table_header())
            .add_modifier(Modifier::BOLD);
        let cell_style = Style::default().fg(Color::Rgb(200, 200, 220));

        for (row_idx, row) in table_rows.iter().enumerate() {
            let is_header = row_idx < header_count;
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
            self.push_line(Line::from(spans));

            // Render separator line after the header row(s).
            if is_header && row_idx + 1 == header_count {
                let mut sep_spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
                for (col_idx, width) in col_widths.iter().enumerate() {
                    if col_idx > 0 {
                        sep_spans.push(Span::styled("-+-".to_string(), border_style));
                    }
                    sep_spans.push(Span::styled("-".repeat(*width), border_style));
                }
                self.push_line(Line::from(sep_spans));
            }
        }
    }

    fn finish(mut self) -> Vec<RenderedLine> {
        self.flush_line();
        self.lines
            .into_iter()
            .zip(self.copy_overrides)
            .map(|(line, copy_override)| {
                let (copy_text, copy_offset) = copy_override.unwrap_or_else(|| {
                    (line.spans.iter().map(|s| s.content.as_ref()).collect(), 0)
                });
                RenderedLine {
                    line,
                    copy_text,
                    copy_offset,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Syntax highlighting via syntect
// ---------------------------------------------------------------------------

fn highlight_code(
    syntax_set: &SyntaxSet,
    theme: &SyntectTheme,
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
                Span::styled(
                    text.to_string(),
                    Style::default().fg(fg).bg(color_code_bg()),
                )
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
            .flat_map(|l| l.line.spans.iter())
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
            l.line
                .spans
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
            .any(|l| l.line.spans.iter().any(|s| s.style.bg.is_some()));
        assert!(has_bg, "code block should have background color");
    }

    // T-MD-11: Code block lines carry a line-number gutter.
    #[test]
    fn test_code_block_line_numbers() {
        let md = "```rust\nfn main() {}\nprintln!();\n```";
        let lines = render_markdown(md, 80);
        let code_lines: Vec<_> = lines
            .iter()
            .filter(|l| l.copy_text.starts_with("fn main") || l.copy_text.starts_with("println"))
            .collect();
        assert_eq!(code_lines.len(), 2, "expected two code lines");
        for (index, line) in code_lines.iter().enumerate() {
            let gutter = &line.line.spans[0];
            let number = (index + 1).to_string();
            assert!(
                gutter.content.contains(number.as_str()),
                "line {} must show its number in the gutter: {:?}",
                index + 1,
                gutter.content
            );
            assert_eq!(
                gutter.style.bg,
                Some(color_code_bg()),
                "gutter must sit on the code background band"
            );
        }
    }

    // T-MD-12: Copy text excludes the gutter, block indent, and band padding.
    #[test]
    fn test_code_block_copy_text_excludes_gutter() {
        let md = "```rust\nfn main() {}\n  let x = 1;\n```";
        let lines = render_markdown(md, 80);
        let copies: Vec<&str> = lines.iter().map(|l| l.copy_text.as_str()).collect();
        assert!(
            copies.contains(&"fn main() {}"),
            "copy text must be the raw code line: {copies:?}"
        );
        assert!(
            copies.contains(&"  let x = 1;"),
            "copy text must keep the code's own leading whitespace: {copies:?}"
        );
        for line in &lines {
            assert!(
                !line.copy_text.contains('\u{2502}'),
                "copy text must not contain the gutter separator: {:?}",
                line.copy_text
            );
        }

        // The offset lets selection extraction skip the display gutter:
        // display char at `copy_offset` is the first char of the copy text.
        let code_line = lines
            .iter()
            .find(|l| l.copy_text == "fn main() {}")
            .expect("code line present");
        assert!(code_line.copy_offset > 0);
        let display: String = code_line
            .line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            display
                .chars()
                .skip(code_line.copy_offset)
                .take(2)
                .collect::<String>(),
            "fn"
        );
        // Non-code lines carry no decorative prefix.
        for line in lines
            .iter()
            .filter(|l| l.copy_offset == 0 && !l.copy_text.is_empty())
        {
            let display: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(display.starts_with(line.copy_text.as_str()));
        }
    }

    // T-MD-13: The background band is uniform across code lines and extends
    // past the longest text line instead of stopping at each line's end.
    #[test]
    fn test_code_block_uniform_background_band() {
        let md = "```rust\nshort\na much much longer code line here\n```";
        let lines = render_markdown(md, 80);
        let code_lines: Vec<_> = lines
            .iter()
            .filter(|l| {
                l.line
                    .spans
                    .first()
                    .is_some_and(|s| s.content.contains('\u{2502}'))
            })
            .collect();
        assert_eq!(code_lines.len(), 2, "expected two code lines");

        let widths: Vec<usize> = code_lines
            .iter()
            .map(|l| l.line.spans.iter().map(Span::width).sum::<usize>())
            .collect();
        assert_eq!(
            widths[0], widths[1],
            "background band must be uniform across lines: {widths:?}"
        );

        // The band must be wider than the longest text line (gutter + text).
        let longest_text: usize = code_lines
            .iter()
            .map(|l| {
                let gutter_w = Span::width(&l.line.spans[0]);
                let text_w = unicode_display_width(l.copy_text.as_str());
                gutter_w + text_w
            })
            .max()
            .unwrap();
        assert!(
            widths[0] > longest_text,
            "band {} must extend past the longest text line {}",
            widths[0],
            longest_text
        );

        // The trailing padding span carries the band background.
        for line in &code_lines {
            let pad = line.line.spans.last().unwrap();
            assert_eq!(pad.style.bg, Some(color_code_bg()));
        }
    }

    // T-MD-04: Inline code renders distinctly.
    #[test]
    fn test_inline_code() {
        let lines = render_markdown("Use `foo()` here", 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.line.spans.iter())
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
            .flat_map(|l| l.line.spans.iter())
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
            .flat_map(|l| l.line.spans.iter())
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
            .any(|l| l.line.spans.iter().any(|s| s.content.contains('\u{2500}')));
        assert!(has_rule, "should contain horizontal rule character");
    }

    // T-MD-08: Bold text has BOLD modifier.
    #[test]
    fn test_bold() {
        let lines = render_markdown("**bold text**", 80);
        let has_bold = lines.iter().any(|l| {
            l.line.spans.iter().any(|s| {
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

    /// Concatenate each rendered line's span contents for block-layout
    /// assertions.
    fn line_texts(lines: &[RenderedLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    // T-MD-14: Distinct block components are separated by a blank line, even
    // when the source markdown has no blank line between them.
    #[test]
    fn test_blank_line_separates_distinct_blocks() {
        let md = "intro paragraph\n# heading\n```rust\ncode_line\n```\n- bullet";
        let texts = line_texts(&render_markdown(md, 80));
        let find = |needle: &str| {
            texts
                .iter()
                .position(|t| t.contains(needle))
                .unwrap_or_else(|| panic!("missing {needle}: {texts:?}"))
        };
        let markers = [
            find("intro paragraph"),
            find("heading"),
            find("code_line"),
            find("bullet"),
        ];
        for pair in markers.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(
                b > a + 1 && texts[a + 1].trim().is_empty(),
                "expected a blank line between rows {a} and {b}: {texts:?}"
            );
        }
        // No leading blank line at the document start.
        assert!(
            !texts[0].trim().is_empty(),
            "document must not start with a blank line: {texts:?}"
        );
    }

    // T-MD-15: Blockquotes and tables are separated from neighboring blocks.
    #[test]
    fn test_blank_line_around_blockquote_and_table() {
        let texts = line_texts(&render_markdown("before\n\n> quote\n\nafter", 80));
        let q = texts.iter().position(|t| t.contains("quote")).unwrap();
        assert!(
            texts[q - 1].trim().is_empty(),
            "blank before quote: {texts:?}"
        );
        let a = texts.iter().position(|t| t.contains("after")).unwrap();
        assert!(
            texts[a - 1].trim().is_empty(),
            "blank after quote: {texts:?}"
        );

        let texts = line_texts(&render_markdown(
            "before\n\n| h |\n| - |\n| c |\n\nafter",
            80,
        ));
        let h = texts.iter().position(|t| t.contains('h')).unwrap();
        assert!(
            texts[h - 1].trim().is_empty(),
            "blank before table: {texts:?}"
        );
    }

    // T-MD-16: A horizontal rule is separated from the preceding block.
    #[test]
    fn test_blank_line_before_horizontal_rule() {
        let texts = line_texts(&render_markdown("above\n\n---\n\nbelow", 80));
        let r = texts.iter().position(|t| t.contains('\u{2500}')).unwrap();
        assert!(
            texts[r - 1].trim().is_empty(),
            "blank before rule: {texts:?}"
        );
    }
}
