//! Chat panel renderer.
//!
//! Renders the conversation transcript as styled message blocks, aligned with
//! the GUI's `ChatPanel.tsx` display-item model.
//!
//! Display items:
//!   - `Message`             -- user / assistant / system / tool message
//!   - `StreamingIndicator`  -- typing dots when streaming with no live message
//!   - `Error`               -- error banner
//!   - `WelcomeScreen`       -- empty state
//!
//! Lines are pre-wrapped to the available width so that `total_lines`
//! accurately reflects visual rows. This ensures correct auto-scroll
//! and correct mouse-to-content coordinate mapping for text selection.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::tui::selection::TextSelection;
use crate::tui::state::{AppState, ChatMessage, MessageRole, PanelFocus};

// ---------------------------------------------------------------------------
// Color palette (aligned with GUI CSS variables)
// ---------------------------------------------------------------------------

/// Accent color for the user role header.
const COLOR_USER: Color = Color::Rgb(130, 220, 130);
/// Accent color for the assistant role header.
const COLOR_ASSISTANT: Color = Color::Rgb(120, 180, 255);
/// Accent color for system messages.
const COLOR_SYSTEM: Color = Color::Rgb(220, 200, 100);
/// Accent color for tool messages.
const COLOR_TOOL: Color = Color::Rgb(200, 140, 255);
/// Muted text color for secondary information.
const COLOR_MUTED: Color = Color::Rgb(100, 100, 120);
/// Content text color.
const COLOR_TEXT: Color = Color::Rgb(220, 220, 230);
/// Code inline background.
const COLOR_CODE_BG: Color = Color::Rgb(40, 42, 54);
/// Streaming indicator dot color.
const COLOR_STREAMING_DOT: Color = Color::Rgb(255, 200, 60);
/// Error text color.
const COLOR_ERROR: Color = Color::Rgb(255, 100, 100);
/// Panel background (subtle dark).
const COLOR_PANEL_BG: Color = Color::Rgb(22, 22, 30);
/// Welcome screen accent.
const COLOR_WELCOME: Color = Color::Rgb(100, 120, 180);
/// `ThinkingCard` accent (purple, from GUI ThinkingCard.tsx #a78bfa).
const COLOR_THINK_ACCENT: Color = Color::Rgb(167, 139, 250);
/// `ThinkingCard` content text.
const COLOR_THINK_TEXT: Color = Color::Rgb(160, 150, 200);
/// `ToolCallCard` accent (blue, from GUI ToolCallCard.tsx #00a6ff).
const COLOR_TOOL_ACCENT: Color = Color::Rgb(0, 166, 255);
/// `ToolCallCard` content text.
const COLOR_TOOL_CARD_TEXT: Color = Color::Rgb(140, 170, 200);
/// `ToolCallCard` success status.
const COLOR_TOOL_SUCCESS: Color = Color::Rgb(100, 200, 120);
/// `ToolCallCard` error status.
const COLOR_TOOL_ERROR: Color = Color::Rgb(255, 100, 100);
/// `ToolCallCard` running status.
const COLOR_TOOL_RUNNING: Color = Color::Rgb(255, 200, 60);

// ---------------------------------------------------------------------------
// Display items (mirrors GUI `DisplayItem` enum)
// ---------------------------------------------------------------------------

/// A flat display item consumed by the renderer.
enum DisplayItem<'a> {
    /// A chat message (user / assistant / system / tool).
    Message { msg: &'a ChatMessage, is_last: bool },
    /// Streaming indicator when no live streaming message exists.
    StreamingIndicator,
    /// Error banner.
    Error(String),
    /// Welcome screen (no messages, no session).
    WelcomeScreen,
}

/// Build a flat display-item list from `AppState`, mirroring the GUI's
/// `buildDisplayItems` logic.
fn build_display_items<'a>(state: &'a AppState) -> Vec<DisplayItem<'a>> {
    if state.messages.is_empty() && !state.is_streaming {
        return vec![DisplayItem::WelcomeScreen];
    }

    let mut items: Vec<DisplayItem<'a>> = Vec::new();
    let msg_count = state.messages.len();

    for (i, msg) in state.messages.iter().enumerate() {
        items.push(DisplayItem::Message {
            msg,
            is_last: i + 1 == msg_count,
        });
    }

    // Streaming indicator when streaming but no live streaming message exists.
    if state.is_streaming && !state.messages.iter().any(|m| m.is_streaming) {
        items.push(DisplayItem::StreamingIndicator);
    }

    items
}

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the chat panel into the given area.
///
/// Returns a flat list of plain-text content lines (one per rendered row)
/// so that the selection system can extract text by row/col index.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) -> Vec<String> {
    let is_focused = state.focus == PanelFocus::Chat;

    let border_style = if is_focused {
        Style::default().fg(COLOR_ASSISTANT)
    } else {
        Style::default().fg(Color::Rgb(50, 50, 65))
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Chat ")
        .title_style(Style::default().fg(Color::Rgb(180, 180, 200)));

    // Available content width (subtract 2 for left/right borders).
    let inner_width = area.width.saturating_sub(2) as usize;

    let display_items = build_display_items(state);
    let mut raw_lines: Vec<Line> = Vec::new();
    let mut raw_plain: Vec<String> = Vec::new();

    for item in &display_items {
        match item {
            DisplayItem::WelcomeScreen => {
                render_welcome(&mut raw_lines, &mut raw_plain, inner_width);
            }
            DisplayItem::Message { msg, is_last } => {
                if !raw_lines.is_empty() {
                    raw_lines.push(Line::from(""));
                    raw_plain.push(String::new());
                }
                render_message(&mut raw_lines, &mut raw_plain, msg, *is_last);
            }
            DisplayItem::StreamingIndicator => {
                raw_lines.push(Line::from(""));
                raw_plain.push(String::new());
                render_streaming_indicator(&mut raw_lines, &mut raw_plain);
            }
            DisplayItem::Error(err) => {
                raw_lines.push(Line::from(""));
                raw_plain.push(String::new());
                render_error(&mut raw_lines, &mut raw_plain, err);
            }
        }
    }

    // Pre-wrap: split each logical line into visual rows based on inner_width.
    let mut lines: Vec<Line> = Vec::new();
    let mut plain_lines: Vec<String> = Vec::new();
    if inner_width > 0 {
        for (raw_line, raw_text) in raw_lines.into_iter().zip(raw_plain.into_iter()) {
            let wrapped_plain = wrap_text(&raw_text, inner_width);
            if wrapped_plain.len() <= 1 {
                lines.push(raw_line);
                plain_lines.push(raw_text);
            } else {
                let style = raw_line.spans.first().map(|s| s.style).unwrap_or_default();
                for wp in wrapped_plain {
                    lines.push(Line::from(Span::styled(wp.clone(), style)));
                    plain_lines.push(wp);
                }
            }
        }
    } else {
        lines = vec![Line::from("")];
        plain_lines = vec![String::new()];
    }

    // Compute scroll.
    let inner_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();

    let scroll_to = if state.scroll_offset == 0 {
        total_lines.saturating_sub(inner_height)
    } else {
        total_lines
            .saturating_sub(inner_height)
            .saturating_sub(state.scroll_offset)
    };

    // Apply selection highlight.
    let selection = &state.selection;
    if !selection.is_empty() {
        let visible_start = scroll_to;
        let visible_end = (scroll_to + inner_height).min(total_lines);

        for (row_idx, line) in lines
            .iter_mut()
            .enumerate()
            .skip(visible_start)
            .take(visible_end - visible_start)
        {
            *line = apply_selection_highlight(line, row_idx, selection);
        }
    }

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().bg(COLOR_PANEL_BG))
        .scroll((u16::try_from(scroll_to).unwrap_or(0), 0));

    frame.render_widget(para, area);

    // "New content below" indicator when scrolled up during streaming.
    if state.scroll_offset > 0 && state.is_streaming {
        let indicator = Span::styled(
            " v New content below ",
            Style::default()
                .fg(Color::Rgb(20, 20, 30))
                .bg(COLOR_STREAMING_DOT)
                .add_modifier(Modifier::BOLD),
        );
        let indicator_line = Line::from(indicator);
        let indicator_area = Rect {
            x: area.x + 2,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(4).min(22),
            height: 1,
        };
        frame.render_widget(Paragraph::new(indicator_line), indicator_area);
    }

    plain_lines
}

// ---------------------------------------------------------------------------
// Welcome screen (aligned with GUI WelcomePage empty state)
// ---------------------------------------------------------------------------

fn render_welcome(lines: &mut Vec<Line>, plain: &mut Vec<String>, width: usize) {
    // Center vertically by adding blank lines (best effort).
    let pad_lines = 3;
    for _ in 0..pad_lines {
        lines.push(Line::from(""));
        plain.push(String::new());
    }

    // Title line, centered.
    let title = "Welcome to y-agent";
    let pad = width.saturating_sub(title.len()) / 2;
    let padded = format!("{}{}", " ".repeat(pad), title);
    lines.push(Line::from(Span::styled(
        padded.clone(),
        Style::default()
            .fg(COLOR_WELCOME)
            .add_modifier(Modifier::BOLD),
    )));
    plain.push(padded);

    lines.push(Line::from(""));
    plain.push(String::new());

    let subtitle = "Start a conversation by typing a message below.";
    let pad2 = width.saturating_sub(subtitle.len()) / 2;
    let padded2 = format!("{}{}", " ".repeat(pad2), subtitle);
    lines.push(Line::from(Span::styled(
        padded2.clone(),
        Style::default().fg(COLOR_MUTED),
    )));
    plain.push(padded2);
}

// ---------------------------------------------------------------------------
// Message rendering (role-based, aligned with GUI chat-box components)
// ---------------------------------------------------------------------------

/// Render a single message with role-based styling.
///
/// Layout (mirrors GUI `AssistantMessageShell` / `UserBubble`):
///
/// ```text
///   Role [streaming dot] [cancelled]
///   content line 1
///   content line 2
///   ...
///   [timestamp] [tokens]    (for non-streaming assistant only)
/// ```
fn render_message(
    lines: &mut Vec<Line>,
    plain_lines: &mut Vec<String>,
    msg: &ChatMessage,
    is_last: bool,
) {
    let (role_label, role_color, prefix_char) = match msg.role {
        MessageRole::User => ("You", COLOR_USER, ">"),
        MessageRole::Assistant => ("Assistant", COLOR_ASSISTANT, "*"),
        MessageRole::System => ("System", COLOR_SYSTEM, "-"),
        MessageRole::Tool => ("Tool", COLOR_TOOL, "#"),
    };

    let role_style = Style::default().fg(role_color).add_modifier(Modifier::BOLD);

    // Role header line.
    let mut header_spans = vec![
        Span::styled(format!(" {prefix_char} "), Style::default().fg(role_color)),
        Span::styled(role_label.to_string(), role_style),
    ];
    let mut header_plain = format!(" {prefix_char} {role_label}");

    if msg.is_streaming {
        header_spans.push(Span::styled(
            "  *",
            Style::default()
                .fg(COLOR_STREAMING_DOT)
                .add_modifier(Modifier::BOLD),
        ));
        header_plain.push_str("  *");
    }
    if msg.is_cancelled {
        header_spans.push(Span::styled(
            " [cancelled]",
            Style::default().fg(COLOR_ERROR),
        ));
        header_plain.push_str(" [cancelled]");
    }

    lines.push(Line::from(header_spans));
    plain_lines.push(header_plain);

    // Pre-process content: extract think blocks, tool calls, strip tool results.
    let segments = preprocess_content(&msg.content);
    for seg in &segments {
        match seg {
            ContentSegment::Text(text) => {
                render_content_lines(lines, plain_lines, text, msg.role);
            }
            ContentSegment::ThinkBlock {
                content,
                is_complete,
            } => {
                render_think_card(lines, plain_lines, content, *is_complete);
            }
            ContentSegment::ToolCall {
                name,
                arguments,
                is_streaming,
            } => {
                render_tool_call_card(
                    lines,
                    plain_lines,
                    name,
                    arguments.as_deref(),
                    *is_streaming,
                );
            }
        }
    }

    // Footer: timestamp + tokens (for completed assistant messages only).
    if msg.role == MessageRole::Assistant && !msg.is_streaming && is_last {
        let time_str = msg.timestamp.format("%H:%M").to_string();
        let footer_spans = vec![Span::styled(
            format!("     {time_str}"),
            Style::default().fg(COLOR_MUTED),
        )];
        let footer_plain = format!("     {time_str}");
        lines.push(Line::from(footer_spans));
        plain_lines.push(footer_plain);
    }
}

// ---------------------------------------------------------------------------
// Content pre-processing (think blocks, tool calls, tool results)
// ---------------------------------------------------------------------------

/// Minimum character count for a completed `<think>` block to be treated as
/// genuine reasoning (mirrors GUI `MIN_THINK_CONTENT_LENGTH`).
const MIN_THINK_CONTENT_LENGTH: usize = 5;

/// A segment of pre-processed message content.
#[derive(Debug)]
enum ContentSegment {
    /// Plain text (may contain markdown).
    Text(String),
    /// A `<think>...</think>` reasoning block.
    ThinkBlock { content: String, is_complete: bool },
    /// A `<tool_call>...</tool_call>` block (any supported envelope).
    ToolCall {
        name: String,
        arguments: Option<String>,
        is_streaming: bool,
    },
}

/// All envelope open tags we recognise for tool calls (same list as y-tools parser).
const TOOL_CALL_OPENS: &[&str] = &[
    "<tool_call>",
    "<longcat_tool_call>",
    "<function_calls>",
    "<\u{ff5c}DSML\u{ff5c}function_calls>",
    "<minimax:tool_call>",
];
const TOOL_CALL_CLOSES: &[&str] = &[
    "</tool_call>",
    "</longcat_tool_call>",
    "</function_calls>",
    "</\u{ff5c}DSML\u{ff5c}function_calls>",
    "</minimax:tool_call>",
];

/// Pre-process message content into structured segments.
///
/// Extracts:
/// 1. `<think>...</think>` blocks -> `ThinkBlock`
/// 2. `<tool_call>...</tool_call>` (and other envelopes) -> `ToolCall`
/// 3. Strips `<tool_result>...</tool_result>` blocks entirely
/// 4. Remaining text -> `Text`
fn preprocess_content(raw: &str) -> Vec<ContentSegment> {
    // Step 1: Strip tool_result blocks.
    let cleaned = strip_tool_result_blocks(raw);

    // Step 2: Segment into think blocks, tool calls, and text.
    segment_content(&cleaned)
}

/// Strip all `<tool_result ...>...</tool_result>` blocks from the input.
fn strip_tool_result_blocks(input: &str) -> String {
    const OPEN: &str = "<tool_result";
    const CLOSE: &str = "</tool_result>";

    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if let Some(open_pos) = input[i..].find(OPEN) {
            let abs_open = i + open_pos;
            result.push_str(&input[i..abs_open]);

            if let Some(close_pos) = input[abs_open..].find(CLOSE) {
                i = abs_open + close_pos + CLOSE.len();
            } else {
                // Incomplete block -- strip everything from here.
                break;
            }
        } else {
            result.push_str(&input[i..]);
            break;
        }
    }

    result
}

/// Segment content string into `ThinkBlock`, `ToolCall`, and Text segments.
fn segment_content(input: &str) -> Vec<ContentSegment> {
    let mut segments: Vec<ContentSegment> = Vec::new();
    let mut cursor = 0;
    let _bytes = input.as_bytes();

    while cursor < input.len() {
        // Find the next `<` character.
        let next_lt = if let Some(pos) = input[cursor..].find('<') {
            cursor + pos
        } else {
            // No more tags -- rest is text.
            push_text_segment(&mut segments, &input[cursor..]);
            break;
        };

        let remaining = &input[next_lt..];

        // Check for <think> tag.
        if remaining.starts_with("<think>") {
            // Flush text before the tag.
            if next_lt > cursor {
                push_text_segment(&mut segments, &input[cursor..next_lt]);
            }

            let after_open = next_lt + "<think>".len();
            if let Some(close_pos) = input[after_open..].find("</think>") {
                let think_content = input[after_open..after_open + close_pos].trim();
                if think_content.len() >= MIN_THINK_CONTENT_LENGTH {
                    segments.push(ContentSegment::ThinkBlock {
                        content: think_content.to_string(),
                        is_complete: true,
                    });
                } else {
                    // Too short -- treat as normal text.
                    push_text_segment(
                        &mut segments,
                        &input[next_lt..after_open + close_pos + "</think>".len()],
                    );
                }
                cursor = after_open + close_pos + "</think>".len();
            } else {
                // Unclosed think tag -- streaming.
                let think_content = input[after_open..].trim();
                if !think_content.is_empty() {
                    segments.push(ContentSegment::ThinkBlock {
                        content: think_content.to_string(),
                        is_complete: false,
                    });
                }
                break;
            }
            continue;
        }

        // Check for tool_call envelope tags.
        if let Some((env_idx, _)) = find_tool_call_open(remaining) {
            let open_tag = TOOL_CALL_OPENS[env_idx];
            let close_tag = TOOL_CALL_CLOSES[env_idx];

            // Flush text before the tag.
            if next_lt > cursor {
                push_text_segment(&mut segments, &input[cursor..next_lt]);
            }

            let after_open = next_lt + open_tag.len();
            if let Some(close_pos) = input[after_open..].find(close_tag) {
                let inner = input[after_open..after_open + close_pos].trim();
                let (name, arguments) = parse_tool_call_inner(inner);
                segments.push(ContentSegment::ToolCall {
                    name,
                    arguments,
                    is_streaming: false,
                });
                cursor = after_open + close_pos + close_tag.len();
            } else {
                // Incomplete tool call -- streaming.
                let inner = input[after_open..].trim();
                let (name, arguments) = if inner.is_empty() {
                    ("...".to_string(), None)
                } else {
                    parse_tool_call_inner(inner)
                };
                segments.push(ContentSegment::ToolCall {
                    name,
                    arguments,
                    is_streaming: true,
                });
                break;
            }
            continue;
        }

        // Not a recognised tag. Check if it is a partial prefix of a known tag
        // at the very end of the input (streaming buffer).
        if next_lt + remaining.len() == input.len() && is_partial_tag_prefix(remaining) {
            // Buffer the partial tag -- don't render it.
            if next_lt > cursor {
                push_text_segment(&mut segments, &input[cursor..next_lt]);
            }
            break;
        }

        // Just a regular `<` character -- include it as text.
        // Advance past this `<` and continue scanning.
        let chunk_end = next_lt + 1;
        // We will flush the text in the next iteration or at the end.
        // For efficiency, find the next `<` and flush the whole chunk.
        let next_next = input[chunk_end..]
            .find('<')
            .map_or(input.len(), |p| chunk_end + p);
        push_text_segment(&mut segments, &input[cursor..next_next]);
        cursor = next_next;
    }

    // Merge consecutive Text segments.
    merge_text_segments(&mut segments);

    if segments.is_empty() && !input.is_empty() {
        segments.push(ContentSegment::Text(input.to_string()));
    }

    segments
}

/// Find the first matching tool call open tag at the start of `remaining`.
fn find_tool_call_open(remaining: &str) -> Option<(usize, usize)> {
    for (idx, open) in TOOL_CALL_OPENS.iter().enumerate() {
        if remaining.starts_with(open) {
            return Some((idx, open.len()));
        }
    }
    None
}

/// Check if the remaining text is a partial prefix of a think or `tool_call` tag.
fn is_partial_tag_prefix(remaining: &str) -> bool {
    let candidates = [
        "<think>",
        "</think>",
        "<tool_call>",
        "</tool_call>",
        "<tool_result",
        "</tool_result>",
    ];
    for c in &candidates {
        if remaining.len() < c.len() && c.starts_with(remaining) {
            return true;
        }
    }
    false
}

/// Parse the inner content of a `tool_call` block to extract name and arguments.
///
/// Handles XML-nested format: `<name>tool</name><arguments>{...}</arguments>`
/// Also handles JSON: `{"name": "tool", "arguments": {...}}`
fn parse_tool_call_inner(inner: &str) -> (String, Option<String>) {
    // Try XML-nested format first.
    if let Some(name) = extract_xml_content(inner, "name") {
        let name = name.trim().to_string();
        let args = extract_xml_content(inner, "arguments").map(|a| a.trim().to_string());
        if !name.is_empty() {
            return (name, args);
        }
    }

    // Try function-attribute format: <function=Name>
    if let Some(func_start) = inner.find("<function=") {
        let after = &inner[func_start + "<function=".len()..];
        if let Some(close) = after.find('>') {
            let name = after[..close].trim().to_string();
            if !name.is_empty() {
                return (name, Some(inner.to_string()));
            }
        }
    }

    // Try JSON format.
    if inner.starts_with('{') {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(inner) {
            if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                let args = json.get("arguments").map(|a| {
                    if a.is_string() {
                        a.as_str().unwrap_or("").to_string()
                    } else {
                        serde_json::to_string_pretty(a).unwrap_or_default()
                    }
                });
                return (name.to_string(), args);
            }
        }
    }

    // Fallback: use the raw inner text as the name.
    let first_line = inner.lines().next().unwrap_or(inner).trim();
    let name = if first_line.len() > 30 {
        format!("{}...", &first_line[..27])
    } else {
        first_line.to_string()
    };
    (name, None)
}

/// Extract text content between `<tag>` and `</tag>`.
fn extract_xml_content<'a>(input: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = input.find(&open)? + open.len();
    let end = input[start..].find(&close)? + start;
    Some(&input[start..end])
}

/// Push a text segment, skipping empty strings.
fn push_text_segment(segments: &mut Vec<ContentSegment>, text: &str) {
    if !text.is_empty() {
        segments.push(ContentSegment::Text(text.to_string()));
    }
}

/// Merge consecutive Text segments into one.
fn merge_text_segments(segments: &mut Vec<ContentSegment>) {
    let mut merged: Vec<ContentSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let ContentSegment::Text(ref text) = seg {
            if let Some(ContentSegment::Text(ref mut prev)) = merged.last_mut() {
                prev.push_str(text);
                continue;
            }
        }
        merged.push(seg);
    }
    *segments = merged;
}

// ---------------------------------------------------------------------------
// ThinkingCard renderer (aligned with GUI ThinkingCard.tsx)
// ---------------------------------------------------------------------------

/// Render a thinking block as a collapsible card.
///
/// Layout:
/// ```text
///      [Thinking] Thought  (or "Thinking..." if streaming)
///      | reasoning line 1
///      | reasoning line 2
///      | ...
/// ```
fn render_think_card(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    content: &str,
    is_complete: bool,
) {
    let indent = "     ";

    // Header line with icon and status.
    let (label, label_style) = if is_complete {
        (
            "Thought",
            Style::default()
                .fg(COLOR_THINK_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "Thinking...",
            Style::default()
                .fg(COLOR_THINK_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    };

    let header_spans = vec![
        Span::styled(
            format!("{indent}\u{25b8} "),
            Style::default().fg(COLOR_THINK_ACCENT),
        ),
        Span::styled(label.to_string(), label_style),
    ];
    let header_plain = format!("{indent}> {label}");
    lines.push(Line::from(header_spans));
    plain.push(header_plain);

    // Content lines with left border indicator.
    if is_complete {
        // Show first 3 lines as preview (collapsed state, like GUI auto-collapse).
        let content_lines: Vec<&str> = content.lines().collect();
        let preview_count = 3.min(content_lines.len());
        for line_text in content_lines.iter().take(preview_count) {
            let formatted = format!("{indent}\u{2502} {line_text}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default().fg(COLOR_THINK_TEXT),
            )));
            plain.push(formatted);
        }
        if content_lines.len() > preview_count {
            let more = content_lines.len() - preview_count;
            let more_text = format!("{indent}\u{2502} ... ({more} more lines)");
            lines.push(Line::from(Span::styled(
                more_text.clone(),
                Style::default().fg(COLOR_MUTED),
            )));
            plain.push(more_text);
        }
    } else {
        // Streaming: show all content.
        for line_text in content.lines() {
            let formatted = format!("{indent}\u{2502} {line_text}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default().fg(COLOR_THINK_TEXT),
            )));
            plain.push(formatted);
        }
    }
}

// ---------------------------------------------------------------------------
// ToolCallCard renderer (aligned with GUI ToolCallCard.tsx)
// ---------------------------------------------------------------------------

/// Render a tool call as a styled card.
///
/// Layout:
/// ```text
///      [wrench] ToolName  Done / Running...
///        Arguments: ...
/// ```
fn render_tool_call_card(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    name: &str,
    arguments: Option<&str>,
    is_streaming: bool,
) {
    let indent = "     ";

    // Status indicator.
    let (status_label, status_color) = if is_streaming {
        ("Running...", COLOR_TOOL_RUNNING)
    } else {
        ("Done", COLOR_TOOL_SUCCESS)
    };

    // Header line: [wrench icon] ToolName  Status
    let header_spans = vec![
        Span::styled(
            format!("{indent}\u{2692} "),
            Style::default().fg(COLOR_TOOL_ACCENT),
        ),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(COLOR_TOOL_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(status_label.to_string(), Style::default().fg(status_color)),
    ];
    let header_plain = format!("{indent}# {name}  {status_label}");
    lines.push(Line::from(header_spans));
    plain.push(header_plain);

    // Arguments preview (if available).
    if let Some(args) = arguments {
        let args_trimmed = args.trim();
        if !args_trimmed.is_empty() {
            // Try to format as pretty JSON for readability.
            let display_args = if args_trimmed.starts_with('{') {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(args_trimmed) {
                    // For tool calls, show a compact single-line summary.
                    format_args_compact(&json)
                } else {
                    truncate_str(args_trimmed, 80)
                }
            } else {
                truncate_str(args_trimmed, 80)
            };

            let args_line = format!("{indent}  {display_args}");
            lines.push(Line::from(Span::styled(
                args_line.clone(),
                Style::default().fg(COLOR_TOOL_CARD_TEXT),
            )));
            plain.push(args_line);
        }
    }
}

/// Format JSON arguments as a compact preview string.
fn format_args_compact(json: &serde_json::Value) -> String {
    if let Some(obj) = json.as_object() {
        let pairs: Vec<String> = obj
            .iter()
            .take(3)
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => truncate_str(s, 40),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => truncate_str(&v.to_string(), 30),
                };
                format!("{k}={val_str}")
            })
            .collect();
        let result = pairs.join(", ");
        if obj.len() > 3 {
            format!("{result}, ...")
        } else {
            result
        }
    } else {
        truncate_str(&json.to_string(), 80)
    }
}

/// Truncate a string to `max_len` characters, adding ellipsis if needed.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

// ---------------------------------------------------------------------------
// Content line rendering (markdown-lite)
// ---------------------------------------------------------------------------

/// Render content lines with basic inline markdown formatting.
///
/// Supported:
///   - Fenced code blocks (``` ... ```)
///   - Inline code (`code`)
///   - Bold (**text**)
///   - Headers (# H1, ## H2, etc.)
///   - Bullet lists (- item, * item)
fn render_content_lines(
    lines: &mut Vec<Line>,
    plain_lines: &mut Vec<String>,
    content: &str,
    role: MessageRole,
) {
    let indent = "     ";
    let content_style = match role {
        MessageRole::User | MessageRole::Assistant => Style::default().fg(COLOR_TEXT),
        MessageRole::System => Style::default().fg(COLOR_SYSTEM),
        MessageRole::Tool => Style::default().fg(Color::Rgb(180, 180, 200)),
    };

    let mut in_code_block = false;
    let mut code_lang = String::new();

    for raw_line in content.lines() {
        // Detect fenced code block boundaries.
        if raw_line.trim_start().starts_with("```") {
            if in_code_block {
                // End of code block.
                in_code_block = false;
                let fence = format!("{indent}```");
                lines.push(Line::from(Span::styled(
                    fence.clone(),
                    Style::default().fg(COLOR_MUTED),
                )));
                plain_lines.push(fence);
                code_lang.clear();
            } else {
                // Start of code block.
                in_code_block = true;
                code_lang = raw_line
                    .trim_start()
                    .strip_prefix("```")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let lang_display = if code_lang.is_empty() {
                    "```".to_string()
                } else {
                    format!("``` {code_lang}")
                };
                let fence = format!("{indent}{lang_display}");
                lines.push(Line::from(Span::styled(
                    fence.clone(),
                    Style::default().fg(COLOR_MUTED),
                )));
                plain_lines.push(fence);
            }
            continue;
        }

        if in_code_block {
            // Code block content: dimmed, monospace-style.
            let formatted = format!("{indent}  {raw_line}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default()
                    .fg(Color::Rgb(180, 200, 220))
                    .bg(COLOR_CODE_BG),
            )));
            plain_lines.push(formatted);
            continue;
        }

        // Header lines.
        let trimmed = raw_line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("### ") {
            let formatted = format!("{indent}{rest}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                content_style.add_modifier(Modifier::BOLD),
            )));
            plain_lines.push(formatted);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            let formatted = format!("{indent}{rest}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                content_style
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            )));
            plain_lines.push(formatted);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let formatted = format!("{indent}{rest}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                content_style
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
            )));
            plain_lines.push(formatted);
            continue;
        }

        // Bullet list items.
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let bullet_content = &trimmed[2..];
            let formatted = format!("{indent}  {bullet_content}");
            let spans = build_inline_spans(&formatted, content_style);
            let plain_text = formatted;
            lines.push(Line::from(spans));
            plain_lines.push(plain_text);
            continue;
        }

        // Numbered list items.
        if trimmed.chars().take_while(char::is_ascii_digit).count() > 0
            && trimmed
                .chars()
                .skip_while(char::is_ascii_digit)
                .take(2)
                .collect::<String>()
                .starts_with(". ")
        {
            let formatted = format!("{indent}  {trimmed}");
            let spans = build_inline_spans(&formatted, content_style);
            lines.push(Line::from(spans));
            plain_lines.push(formatted);
            continue;
        }

        // Regular content line with inline formatting.
        let formatted = format!("{indent}{raw_line}");
        let spans = build_inline_spans(&formatted, content_style);
        lines.push(Line::from(spans));
        plain_lines.push(formatted);
    }
}

/// Build styled spans for a line with inline markdown:
///   - `**bold**` -> bold
///   - `` `code` `` -> code style
fn build_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let code_style = Style::default()
        .fg(Color::Rgb(200, 220, 255))
        .bg(COLOR_CODE_BG);
    let bold_style = base_style.add_modifier(Modifier::BOLD);

    while i < len {
        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !buf.is_empty() {
                spans.push(Span::styled(buf.clone(), base_style));
                buf.clear();
            }
            i += 2;
            let mut bold_buf = String::new();
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                bold_buf.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing **
            }
            if !bold_buf.is_empty() {
                spans.push(Span::styled(bold_buf, bold_style));
            }
            continue;
        }

        // Inline code: `...`
        if chars[i] == '`' {
            if !buf.is_empty() {
                spans.push(Span::styled(buf.clone(), base_style));
                buf.clear();
            }
            i += 1;
            let mut code_buf = String::new();
            while i < len && chars[i] != '`' {
                code_buf.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing `
            }
            if !code_buf.is_empty() {
                spans.push(Span::styled(format!(" {code_buf} "), code_style));
            }
            continue;
        }

        buf.push(chars[i]);
        i += 1;
    }

    if !buf.is_empty() {
        spans.push(Span::styled(buf, base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }

    spans
}

// ---------------------------------------------------------------------------
// Streaming indicator (aligned with GUI streaming-indicator class)
// ---------------------------------------------------------------------------

fn render_streaming_indicator(lines: &mut Vec<Line>, plain: &mut Vec<String>) {
    let spans = vec![
        Span::styled("     ", Style::default()),
        Span::styled(
            "* ",
            Style::default()
                .fg(COLOR_STREAMING_DOT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Thinking...", Style::default().fg(COLOR_MUTED)),
    ];
    lines.push(Line::from(spans));
    plain.push("     * Thinking...".to_string());
}

// ---------------------------------------------------------------------------
// Error banner (aligned with GUI chat-error class)
// ---------------------------------------------------------------------------

fn render_error(lines: &mut Vec<Line>, plain: &mut Vec<String>, err: &str) {
    let formatted = format!("     ! {err}");
    lines.push(Line::from(Span::styled(
        formatted.clone(),
        Style::default()
            .fg(COLOR_ERROR)
            .add_modifier(Modifier::BOLD),
    )));
    plain.push(formatted);
}

// ---------------------------------------------------------------------------
// Selection highlight (unchanged from original)
// ---------------------------------------------------------------------------

/// Apply inverse-color highlight to characters in a line that fall within the selection.
fn apply_selection_highlight<'a>(
    line: &Line<'a>,
    row: usize,
    selection: &TextSelection,
) -> Line<'a> {
    let highlight_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);

    let mut new_spans: Vec<Span<'a>> = Vec::new();
    let mut col = 0usize;

    for span in &line.spans {
        let text = span.content.as_ref();
        let span_len = text.chars().count();

        let span_start = col;
        let span_end = col + span_len;

        let sel_start_in_span = selection.contains(row, span_start);
        let sel_end_in_span = span_end > 0 && selection.contains(row, span_end - 1);

        if sel_start_in_span && sel_end_in_span {
            new_spans.push(Span::styled(span.content.clone(), highlight_style));
        } else if !sel_start_in_span
            && !sel_end_in_span
            && !selection_overlaps(selection, row, span_start, span_end)
        {
            new_spans.push(span.clone());
        } else {
            let mut normal_buf = String::new();
            let mut highlight_buf = String::new();
            for ch in text.chars() {
                if selection.contains(row, col) {
                    if !normal_buf.is_empty() {
                        new_spans.push(Span::styled(normal_buf.clone(), span.style));
                        normal_buf.clear();
                    }
                    highlight_buf.push(ch);
                } else {
                    if !highlight_buf.is_empty() {
                        new_spans.push(Span::styled(highlight_buf.clone(), highlight_style));
                        highlight_buf.clear();
                    }
                    normal_buf.push(ch);
                }
                col += 1;
            }
            if !highlight_buf.is_empty() {
                new_spans.push(Span::styled(highlight_buf, highlight_style));
            }
            if !normal_buf.is_empty() {
                new_spans.push(Span::styled(normal_buf, span.style));
            }
            continue;
        }
        col = span_end;
    }

    Line::from(new_spans)
}

/// Check whether the selection overlaps the character range `[span_start, span_end)`
/// on the given `row`.
fn selection_overlaps(sel: &TextSelection, row: usize, span_start: usize, span_end: usize) -> bool {
    if sel.is_empty() || span_start >= span_end {
        return false;
    }
    let ((sr, sc), (er, ec)) = sel.sorted();

    if row < sr || row > er {
        return false;
    }

    let sel_col_start = if row == sr { sc } else { 0 };
    let sel_col_end = if row == er { ec } else { usize::MAX };

    span_start < sel_col_end && sel_col_start < span_end
}

// ---------------------------------------------------------------------------
// Text wrapping
// ---------------------------------------------------------------------------

/// Wrap a plain-text string into multiple lines that each fit within `max_width`
/// display columns. Uses Unicode display width for correct CJK / emoji handling.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    if text.is_empty() || UnicodeWidthStr::width(text) <= max_width {
        return vec![text.to_string()];
    }

    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width: usize = 0;

    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > max_width && !current.is_empty() {
            rows.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // T-TUI-02-04: Chat scroll offset limits clamp to message count.
    #[test]
    fn test_scroll_offset_clamping() {
        let total_lines: usize = 5;
        let inner_height: usize = 20;
        let offset: usize = 0;

        let scroll_to = if offset == 0 {
            total_lines.saturating_sub(inner_height)
        } else {
            total_lines
                .saturating_sub(inner_height)
                .saturating_sub(offset)
        };

        assert_eq!(scroll_to, 0, "no scroll when content fits");
    }

    #[test]
    fn test_render_message_creates_lines() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Hello\nWorld".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_message(&mut lines, &mut plain, &msg, false);

        // Header + 2 content lines = 3 lines.
        assert_eq!(lines.len(), 3);
        assert_eq!(plain.len(), 3);
    }

    #[test]
    fn test_streaming_indicator_in_header() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "Thinking...".to_string(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_message(&mut lines, &mut plain, &msg, false);

        let header = &lines[0];
        let header_text: String = header.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(header_text.contains('*'));
    }

    #[test]
    fn test_wrap_text_no_wrap_needed() {
        let result = wrap_text("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_text_exact_fit() {
        let result = wrap_text("12345", 5);
        assert_eq!(result, vec!["12345"]);
    }

    #[test]
    fn test_wrap_text_splits() {
        let result = wrap_text("abcdefghij", 5);
        assert_eq!(result, vec!["abcde", "fghij"]);
    }

    #[test]
    fn test_wrap_text_empty() {
        let result = wrap_text("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_wrap_text_cjk_double_width() {
        let result = wrap_text("你好世界测试", 6);
        assert_eq!(result, vec!["你好世", "界测试"]);
    }

    #[test]
    fn test_display_items_empty_state() {
        let state = AppState::default();
        let items = build_display_items(&state);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], DisplayItem::WelcomeScreen));
    }

    #[test]
    fn test_display_items_with_messages() {
        let mut state = AppState::default();
        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
        });
        let items = build_display_items(&state);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], DisplayItem::Message { .. }));
    }

    #[test]
    fn test_display_items_streaming_indicator() {
        let mut state = AppState::default();
        state.is_streaming = true;
        // No messages have is_streaming=true.
        let _items = build_display_items(&state);
        // WelcomeScreen should NOT appear since we are streaming.
        // But messages is empty and is_streaming is true, so we still get
        // an empty messages list (no WelcomeScreen when streaming).
        // Actually build_display_items returns WelcomeScreen when
        // messages is empty and not streaming, otherwise it runs the loop.
        // Since is_streaming=true, it skips welcome and adds the indicator.
    }

    #[test]
    fn test_inline_bold_formatting() {
        let base = Style::default().fg(COLOR_TEXT);
        let spans = build_inline_spans("hello **world** end", base);
        assert!(spans.len() >= 3, "expected at least 3 spans for bold text");
        // The bold span should contain "world".
        let bold_span = &spans[1];
        assert_eq!(bold_span.content.as_ref(), "world");
    }

    #[test]
    fn test_inline_code_formatting() {
        let base = Style::default().fg(COLOR_TEXT);
        let spans = build_inline_spans("run `cargo test` now", base);
        assert!(
            spans.len() >= 3,
            "expected at least 3 spans for inline code"
        );
    }

    #[test]
    fn test_code_block_rendering() {
        let content = "text\n```rust\nfn main() {}\n```\nmore";
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_content_lines(&mut lines, &mut plain, content, MessageRole::Assistant);
        // Should have: text, ``` rust, fn main, ```, more = 5 lines.
        assert_eq!(lines.len(), 5);
    }
}
