//! Chat panel renderer.
//!
//! Renders the conversation transcript as styled message blocks, aligned with
//! the GUI's `ChatPanel.tsx` display-item model.
//!
//! Display items:
//!   - `Message`         -- user / assistant / system / tool message
//!   - `WelcomeScreen`   -- empty state
//!
//! Lines are pre-wrapped to the available width so that `total_lines`
//! accurately reflects visual rows. This ensures correct auto-scroll
//! and correct mouse-to-content coordinate mapping for text selection.

use std::ops::Range;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::tui::selection::TextSelection;
use crate::tui::state::{
    AppState, CachedMessageRender, ChatMessage, ChatRenderCache, MessageRole, StreamSegment,
    ToolCallDisplayMode, ToolCallInfo, ToolCallStatus, ToolSelection,
};
use crate::tui::theme::Theme;
use crate::tui::tool_renderers::{
    group_tool_indexes, present_tool, quick_summary, ToolKind, ToolLine, ToolRenderGroup, ToolTone,
};

// ---------------------------------------------------------------------------
// Display items (mirrors GUI `DisplayItem` enum)
// ---------------------------------------------------------------------------

/// A flat display item consumed by the renderer.
enum DisplayItem<'a> {
    /// A chat message (user / assistant / system / tool).
    Message {
        message_index: usize,
        msg: &'a ChatMessage,
        is_last: bool,
    },
    /// Welcome screen (no messages, no session).
    WelcomeScreen,
}

/// Build a flat display-item list from `AppState`, mirroring the GUI's
/// `buildDisplayItems` logic.
///
/// While streaming there is always a streaming placeholder message in the
/// transcript, so no separate indicator item is needed: the streaming
/// message's animated header marker covers that role.
fn build_display_items<'a>(state: &'a AppState) -> Vec<DisplayItem<'a>> {
    if state.messages.is_empty() && !state.is_streaming {
        return vec![DisplayItem::WelcomeScreen];
    }

    let mut items: Vec<DisplayItem<'a>> = Vec::new();
    let msg_count = state.messages.len();

    for (i, msg) in state.messages.iter().enumerate() {
        items.push(DisplayItem::Message {
            message_index: i,
            msg,
            is_last: i + 1 == msg_count,
        });
    }

    items
}

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the chat panel into the given area.
///
/// `cache` holds per-message rendered output so historical messages are not
/// re-rendered (markdown, highlighting, wrapping) on every frame. `plain_out`
/// is cleared and refilled with plain-text content lines (one per rendered
/// row, covering the full history) so the selection system can extract text
/// by absolute row/col index.
///
/// `tool_rows_out` follows the same lifecycle as `plain_out`: it is cleared
/// and refilled on every render with the absolute-row span of every tool
/// card in the transcript, paired with its [`ToolSelection`], so mouse clicks
/// can be mapped to a tool card (hit-testing).
///
/// Only the visible window `[scroll_to, scroll_to + inner_height)` is handed
/// to the `Paragraph`, which removes the `u16` scroll-offset limit of
/// `Paragraph::scroll` for very long histories.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    cache: &mut ChatRenderCache,
    plain_out: &mut Vec<String>,
    tool_rows_out: &mut Vec<(Range<usize>, ToolSelection)>,
) {
    let t = &state.theme;

    // Borderless transcript: the conversation fills the panel edge to edge,
    // so the full area is usable content width.
    let inner_width = area.width as usize;

    plain_out.clear();
    tool_rows_out.clear();

    // Degenerate width: preserve the previous single-blank-row behavior.
    if inner_width == 0 {
        let para = Paragraph::new(vec![Line::from("")]);
        frame.render_widget(para, area);
        plain_out.push(String::new());
        return;
    }

    // Drop cache entries for messages no longer present in the transcript.
    cache.retain_messages(state.messages.len());

    let display_items = build_display_items(state);

    // Assemble the row space: fill the full plain-text history and record
    // where each item's styled lines live (render cache or frame-owned).
    let mut owned_items: Vec<Vec<Line<'static>>> = Vec::new();
    let mut row_spans: Vec<(usize, usize, RowSource)> = Vec::new();
    let mut row_cursor = 0usize;

    for item in &display_items {
        match item {
            DisplayItem::WelcomeScreen => {
                let mut raw_lines = Vec::new();
                let mut raw_plain = Vec::new();
                render_welcome(&mut raw_lines, &mut raw_plain, inner_width, t);
                let (lines, plain, _) = wrap_rendered_lines(raw_lines, raw_plain, inner_width);
                push_owned_item(
                    &mut owned_items,
                    &mut row_spans,
                    &mut row_cursor,
                    plain_out,
                    lines,
                    plain,
                );
            }
            DisplayItem::Message {
                message_index,
                msg,
                is_last,
            } => {
                if row_cursor > 0 {
                    push_blank_separator(
                        &mut owned_items,
                        &mut row_spans,
                        &mut row_cursor,
                        plain_out,
                    );
                }
                let entry = cached_message_render(
                    cache,
                    *message_index,
                    msg,
                    state.selected_tool,
                    *is_last,
                    state.tick_counter,
                    inner_width,
                    t,
                );
                let message_start_row = row_cursor;
                row_spans.push((
                    row_cursor,
                    entry.lines.len(),
                    RowSource::Cache(*message_index),
                ));
                row_cursor += entry.lines.len();
                plain_out.extend(entry.plain.iter().cloned());
                // Offset the message-relative tool card spans to absolute rows.
                tool_rows_out.extend(entry.tool_ranges.iter().map(|(tool_index, range)| {
                    (
                        (message_start_row + range.start)..(message_start_row + range.end),
                        ToolSelection {
                            message_index: *message_index,
                            tool_index: *tool_index,
                        },
                    )
                }));
            }
        }
    }

    // Compute scroll.
    let inner_height = area.height as usize;
    let total_lines = row_cursor;
    let scroll_to = compute_scroll_to(total_lines, inner_height, state.scroll_offset);
    let visible_start = scroll_to.min(total_lines);
    let visible_end = visible_start.saturating_add(inner_height).min(total_lines);

    // Slice the visible window out of the assembled row space, applying the
    // selection highlight with absolute row indices.
    let selection = &state.selection;
    let mut visible_lines: Vec<Line> = Vec::with_capacity(visible_end - visible_start);
    for (start, len, source) in &row_spans {
        let item_end = start + len;
        if item_end <= visible_start || *start >= visible_end {
            continue;
        }
        let slice_start = visible_start.saturating_sub(*start);
        let slice_end = visible_end.min(item_end) - start;
        let source_lines: &[Line] = match source {
            RowSource::Cache(index) => cache.get(*index).map_or(&[][..], |entry| &entry.lines[..]),
            RowSource::Owned(index) => &owned_items[*index],
        };
        let slice_abs_start = start + slice_start;
        for (offset, line) in source_lines[slice_start..slice_end].iter().enumerate() {
            if selection.is_empty() {
                visible_lines.push(line.clone());
            } else {
                visible_lines.push(apply_selection_highlight(
                    line,
                    slice_abs_start + offset,
                    selection,
                ));
            }
        }
    }

    let para = Paragraph::new(visible_lines);

    frame.render_widget(para, area);

    // "New content below" indicator when scrolled up during streaming.
    if state.scroll_offset > 0 && state.is_streaming {
        let indicator = Span::styled(
            " v New content below ",
            Style::default()
                .fg(Color::Rgb(20, 20, 30))
                .bg(t.streaming_dot())
                .add_modifier(Modifier::BOLD),
        );
        let indicator_line = Line::from(indicator);
        let indicator_area = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(1),
            width: area.width.saturating_sub(2).min(22),
            height: 1,
        };
        frame.render_widget(Paragraph::new(indicator_line), indicator_area);
    }
}

// ---------------------------------------------------------------------------
// Row-space assembly helpers
// ---------------------------------------------------------------------------

/// Where the styled lines for a row span are stored.
enum RowSource {
    /// Lines live in the per-message render cache (message index).
    Cache(usize),
    /// Lines are owned by this frame (index into the owned item list).
    Owned(usize),
}

/// Fully rendered message: wrapped styled lines, their plain-text mirror,
/// and the tool card line ranges in wrapped coordinates.
type RenderedMessage = (Vec<Line<'static>>, Vec<String>, Vec<(usize, Range<usize>)>);

/// Append a frame-owned item to the assembled row space.
fn push_owned_item(
    owned_items: &mut Vec<Vec<Line<'static>>>,
    row_spans: &mut Vec<(usize, usize, RowSource)>,
    row_cursor: &mut usize,
    plain_out: &mut Vec<String>,
    lines: Vec<Line<'static>>,
    plain: Vec<String>,
) {
    row_spans.push((
        *row_cursor,
        lines.len(),
        RowSource::Owned(owned_items.len()),
    ));
    *row_cursor += lines.len();
    owned_items.push(lines);
    plain_out.extend(plain);
}

/// Append a single blank separator row to the assembled row space.
fn push_blank_separator(
    owned_items: &mut Vec<Vec<Line<'static>>>,
    row_spans: &mut Vec<(usize, usize, RowSource)>,
    row_cursor: &mut usize,
    plain_out: &mut Vec<String>,
) {
    push_owned_item(
        owned_items,
        row_spans,
        row_cursor,
        plain_out,
        vec![Line::from("")],
        vec![String::new()],
    );
}

/// Compute the first visible row for the chat viewport.
///
/// `scroll_offset == 0` pins the view to the bottom; larger offsets scroll
/// upward, clamping at the top of the history.
///
/// Shared with `tui::mod` (`terminal_to_content`), which maps mouse
/// coordinates to content rows using the same scroll formula.
pub(crate) fn compute_scroll_to(
    total_lines: usize,
    inner_height: usize,
    scroll_offset: usize,
) -> usize {
    if scroll_offset == 0 {
        total_lines.saturating_sub(inner_height)
    } else {
        total_lines
            .saturating_sub(inner_height)
            .saturating_sub(scroll_offset)
    }
}

/// Return the cached render for a message, re-rendering only when one of the
/// display-relevant inputs changed (content hash, width, tool selection, tail
/// position, or the animation tick for spinner frames).
fn cached_message_render<'c>(
    cache: &'c mut ChatRenderCache,
    message_index: usize,
    msg: &ChatMessage,
    selected_tool: Option<ToolSelection>,
    is_last: bool,
    tick: u64,
    inner_width: usize,
    theme: &Theme,
) -> &'c CachedMessageRender {
    let content_hash = msg.render_hash();
    if cache
        .lookup(
            message_index,
            content_hash,
            inner_width,
            selected_tool,
            is_last,
            tick,
        )
        .is_none()
    {
        let (lines, plain, tool_ranges) = render_message_wrapped(
            msg,
            message_index,
            selected_tool,
            is_last,
            tick,
            inner_width,
            theme,
        );
        cache.store(
            message_index,
            CachedMessageRender {
                content_hash,
                inner_width,
                selected_tool,
                is_last,
                animated: message_has_active_spinner(msg),
                tick,
                lines,
                plain,
                tool_ranges,
                // Stamped by `ChatRenderCache::store`.
                generation: 0,
            },
        );
    }
    let Some(entry) = cache.get(message_index) else {
        unreachable!("entry stored above when missing");
    };
    entry
}

/// Render a single message and wrap its lines to `inner_width`.
///
/// Returns the wrapped styled lines, their plain-text mirror, and the tool
/// card line spans in wrapped coordinates (see [`CachedMessageRender::tool_ranges`]).
fn render_message_wrapped(
    msg: &ChatMessage,
    message_index: usize,
    selected_tool: Option<ToolSelection>,
    is_last: bool,
    tick: u64,
    inner_width: usize,
    theme: &Theme,
) -> RenderedMessage {
    let mut raw_lines: Vec<Line> = Vec::new();
    let mut raw_plain: Vec<String> = Vec::new();
    let mut raw_tool_ranges: Vec<(usize, Range<usize>)> = Vec::new();
    render_message(
        &mut raw_lines,
        &mut raw_plain,
        &mut raw_tool_ranges,
        msg,
        message_index,
        selected_tool,
        is_last,
        tick,
        inner_width,
        theme,
    );
    let (lines, plain, wrap_counts) = wrap_rendered_lines(raw_lines, raw_plain, inner_width);
    let tool_ranges = offset_tool_ranges(raw_tool_ranges, &wrap_counts);
    (lines, plain, tool_ranges)
}

/// Convert raw (pre-wrap) tool card line ranges to wrapped-line ranges.
///
/// `wrap_counts[i]` is the number of wrapped rows that raw line `i` produced.
/// Wrapping splits lines in order and never merges lines, so prefix sums of
/// the counts map every raw line boundary to its wrapped line offset.
fn offset_tool_ranges(
    raw_ranges: Vec<(usize, Range<usize>)>,
    wrap_counts: &[usize],
) -> Vec<(usize, Range<usize>)> {
    let mut offsets: Vec<usize> = Vec::with_capacity(wrap_counts.len() + 1);
    offsets.push(0);
    for count in wrap_counts {
        let previous = offsets.last().copied().unwrap_or(0);
        offsets.push(previous + count);
    }
    raw_ranges
        .into_iter()
        .map(|(tool_index, range)| (tool_index, offsets[range.start]..offsets[range.end]))
        .collect()
}

/// Wrap pre-rendered lines to `inner_width`, keeping styled and plain output
/// aligned row by row.
///
/// Lines that fit keep their original spans. Lines that overflow are split
/// span-aware: rows break by display width across span boundaries while each
/// span keeps its own style, and wide characters are never split. The plain
/// mirror of each wrapped row is the exact concatenation of that row's span
/// contents, so selection row/col mapping stays 1:1 with the rendered output.
///
/// The third return value holds, per raw input line, how many wrapped rows it
/// produced, so raw line ranges can be mapped onto wrapped coordinates (see
/// [`offset_tool_ranges`]).
fn wrap_rendered_lines(
    raw_lines: Vec<Line<'static>>,
    raw_plain: Vec<String>,
    inner_width: usize,
) -> (Vec<Line<'static>>, Vec<String>, Vec<usize>) {
    let mut lines: Vec<Line> = Vec::new();
    let mut plain_lines: Vec<String> = Vec::new();
    let mut wrap_counts: Vec<usize> = Vec::with_capacity(raw_lines.len());
    for (raw_line, raw_text) in raw_lines.into_iter().zip(raw_plain) {
        if inner_width == 0 || UnicodeWidthStr::width(raw_text.as_str()) <= inner_width {
            lines.push(raw_line);
            plain_lines.push(raw_text);
            wrap_counts.push(1);
            continue;
        }
        let wrapped_rows = wrap_spans(&raw_line.spans, inner_width);
        wrap_counts.push(wrapped_rows.len());
        for wrapped in wrapped_rows {
            let plain_row: String = wrapped.spans.iter().map(|s| s.content.as_ref()).collect();
            plain_lines.push(plain_row);
            lines.push(wrapped);
        }
    }
    (lines, plain_lines, wrap_counts)
}

/// Split spans into rows of at most `max_width` display columns, preserving
/// each span's style across row boundaries.
///
/// A character is never split: a wide character that would overflow starts a
/// new row (or occupies a fresh row alone when it exceeds `max_width`).
fn wrap_spans(spans: &[Span<'static>], max_width: usize) -> Vec<Line<'static>> {
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cur_spans: Vec<Span<'static>> = Vec::new();
    let mut row_width = 0usize;

    for span in spans {
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if row_width + ch_width > max_width && row_width > 0 {
                if !buf.is_empty() {
                    cur_spans.push(Span::styled(std::mem::take(&mut buf), style));
                }
                rows.push(Line::from(std::mem::take(&mut cur_spans)));
                row_width = 0;
            }
            buf.push(ch);
            row_width += ch_width;
        }
        if !buf.is_empty() {
            cur_spans.push(Span::styled(buf, style));
        }
    }
    if !cur_spans.is_empty() {
        rows.push(Line::from(cur_spans));
    }
    if rows.is_empty() {
        rows.push(Line::from(""));
    }
    rows
}

/// Whether rendering `msg` produces an animation-tick-dependent element: the
/// header spinner of a streaming message, or the braille spinner of an
/// incomplete "Thinking..." card. Such messages are re-rendered once per tick
/// instead of being served fully from cache.
fn message_has_active_spinner(msg: &ChatMessage) -> bool {
    // Streaming messages animate the header spinner every tick.
    if msg.is_streaming {
        return true;
    }
    // Legacy aggregate reasoning path (historical messages).
    if msg.segments.is_empty() && !msg.reasoning_content.is_empty() && !msg.reasoning_complete {
        return true;
    }
    if msg.segments.is_empty() {
        return has_open_think_block(&msg.content);
    }
    msg.segments.iter().any(|segment| match segment {
        StreamSegment::Reasoning { is_complete, .. } => !is_complete,
        StreamSegment::Text(text) => has_open_think_block(text),
        StreamSegment::ToolCall(_) => false,
    })
}

/// Whether `text` contains an unclosed `<think>` tag (renders a spinner).
fn has_open_think_block(text: &str) -> bool {
    match text.rfind("<think>") {
        Some(open) => !text[open..].contains("</think>"),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Welcome screen (aligned with GUI WelcomePage empty state)
// ---------------------------------------------------------------------------

fn render_welcome(lines: &mut Vec<Line>, plain: &mut Vec<String>, width: usize, t: &Theme) {
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
            .fg(t.welcome())
            .add_modifier(Modifier::BOLD),
    )));
    plain.push(padded);

    lines.push(Line::from(""));
    plain.push(String::new());

    let subtitle = "Type a message or press / for commands.";
    let pad2 = width.saturating_sub(subtitle.len()) / 2;
    let padded2 = format!("{}{}", " ".repeat(pad2), subtitle);
    lines.push(Line::from(Span::styled(
        padded2.clone(),
        Style::default().fg(t.muted()),
    )));
    plain.push(padded2);

    lines.push(Line::from(""));
    plain.push(String::new());

    let commands = "/mode   /goal <objective>   /resume   /copy";
    let command_pad = width.saturating_sub(commands.len()) / 2;
    let padded_commands = format!("{}{}", " ".repeat(command_pad), commands);
    lines.push(Line::from(Span::styled(
        padded_commands.clone(),
        Style::default().fg(t.input_border_focused()),
    )));
    plain.push(padded_commands);
}

// ---------------------------------------------------------------------------
// Message rendering (role-based, aligned with GUI chat-box components)
// ---------------------------------------------------------------------------

/// Render a single message with role-based styling.
///
/// Layout (mirrors GUI `AssistantMessageShell` / `UserBubble`):
///
/// ```text
///   Role [streaming spinner] [cancelled]
///   content line 1
///   content line 2
///   ...
///   [timestamp] [tokens]    (for non-streaming assistant only)
/// ```
///
/// Every rendered tool card records its raw (pre-wrap) line span into
/// `tool_ranges` as `(tool_index, line_range)` pairs; the caller maps them
/// onto wrapped coordinates after [`wrap_rendered_lines`].
fn render_message(
    lines: &mut Vec<Line>,
    plain_lines: &mut Vec<String>,
    tool_ranges: &mut Vec<(usize, Range<usize>)>,
    msg: &ChatMessage,
    message_index: usize,
    selected_tool: Option<ToolSelection>,
    is_last: bool,
    tick: u64,
    content_width: usize,
    t: &Theme,
) {
    // No role header line: "You"/"Assistant" labels cost a full row per
    // message and force every other line one indent level deeper. Instead
    // the user's own text is highlighted (see render_content_lines) and
    // tool/thought cards act as the visual separators between turns.
    let content_start = lines.len();

    // Historical messages may only have the legacy aggregate reasoning field.
    // Streaming messages render reasoning from event-ordered segments below.
    if msg.segments.is_empty() && !msg.reasoning_content.is_empty() {
        render_think_card(
            lines,
            plain_lines,
            &msg.reasoning_content,
            msg.reasoning_complete,
            tick,
            t,
        );
    }

    // Render content with tool calls interleaved in event order.
    //
    // When event-ordered segments are available (populated during streaming),
    // use them so tool call cards appear at the position they were executed.
    // Otherwise fall back to parsing the accumulated content string (for
    // historical messages loaded from the database).
    if msg.segments.is_empty() {
        let content_segs = preprocess_content(&msg.content);
        let mut tc_idx: usize = 0;
        for seg in &content_segs {
            match seg {
                ContentSegment::Text(text) => {
                    render_content_lines(lines, plain_lines, text, msg.role, content_width, t);
                }
                ContentSegment::ThinkBlock {
                    content,
                    is_complete,
                } => {
                    render_think_card(lines, plain_lines, content, *is_complete, tick, t);
                }
                ContentSegment::ToolCall {
                    name,
                    arguments,
                    is_streaming,
                } => {
                    if let Some(tc) = msg.tool_calls.get(tc_idx) {
                        render_tool_call_executed_card(
                            lines,
                            plain_lines,
                            tool_ranges,
                            tc,
                            tc_idx,
                            selected_tool
                                == Some(ToolSelection {
                                    message_index,
                                    tool_index: tc_idx,
                                }),
                            content_width,
                            t,
                        );
                    } else {
                        render_tool_call_card(
                            lines,
                            plain_lines,
                            name,
                            arguments.as_deref(),
                            *is_streaming,
                            t,
                        );
                    }
                    tc_idx += 1;
                }
            }
        }
        if tc_idx < msg.tool_calls.len() {
            let tool_indexes = (tc_idx..msg.tool_calls.len()).collect::<Vec<_>>();
            render_tool_index_run(
                lines,
                plain_lines,
                tool_ranges,
                &msg.tool_calls,
                &tool_indexes,
                message_index,
                selected_tool,
                content_width,
                t,
            );
        }
    } else {
        let mut segment_index = 0;
        while segment_index < msg.segments.len() {
            match &msg.segments[segment_index] {
                StreamSegment::Text(text) => {
                    let sub_segs = preprocess_content(text);
                    for sub in &sub_segs {
                        match sub {
                            ContentSegment::Text(segment_text) => {
                                render_content_lines(
                                    lines,
                                    plain_lines,
                                    segment_text,
                                    msg.role,
                                    content_width,
                                    t,
                                );
                            }
                            ContentSegment::ThinkBlock {
                                content,
                                is_complete,
                            } => {
                                render_think_card(
                                    lines,
                                    plain_lines,
                                    content,
                                    *is_complete,
                                    tick,
                                    t,
                                );
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
                                    t,
                                );
                            }
                        }
                    }
                    segment_index += 1;
                }
                StreamSegment::Reasoning {
                    content,
                    is_complete,
                } => {
                    render_think_card(lines, plain_lines, content, *is_complete, tick, t);
                    segment_index += 1;
                }
                StreamSegment::ToolCall(_) => {
                    let start = segment_index;
                    while segment_index < msg.segments.len()
                        && matches!(msg.segments[segment_index], StreamSegment::ToolCall(_))
                    {
                        segment_index += 1;
                    }
                    let tool_indexes = msg.segments[start..segment_index]
                        .iter()
                        .filter_map(|segment| match segment {
                            StreamSegment::ToolCall(tool_index) => Some(*tool_index),
                            StreamSegment::Text(_) | StreamSegment::Reasoning { .. } => None,
                        })
                        .collect::<Vec<_>>();
                    render_tool_index_run(
                        lines,
                        plain_lines,
                        tool_ranges,
                        &msg.tool_calls,
                        &tool_indexes,
                        message_index,
                        selected_tool,
                        content_width,
                        t,
                    );
                }
            }
        }
    }

    // A streaming message that has not produced any renderable content yet
    // still needs a visible placeholder (the status bar carries the global
    // running indicator).
    if msg.is_streaming && lines.len() == content_start {
        let spinner = SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()];
        lines.push(Line::from(Span::styled(
            spinner.to_string(),
            Style::default().fg(t.streaming_dot()),
        )));
        plain_lines.push(spinner.to_string());
    }

    if msg.is_cancelled {
        lines.push(Line::from(Span::styled(
            "(cancelled)".to_string(),
            Style::default().fg(t.error()),
        )));
        plain_lines.push("(cancelled)".to_string());
    }

    // Footer: timestamp + tokens (for completed assistant messages only).
    if msg.role == MessageRole::Assistant && !msg.is_streaming && is_last {
        let time_str = msg.timestamp.format("%H:%M").to_string();
        lines.push(Line::from(Span::styled(
            time_str.clone(),
            Style::default().fg(t.muted()),
        )));
        plain_lines.push(time_str);
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

/// Braille spinner frames for animated thinking/streaming indicators.
///
/// Shared with the status bar's "running" segment so every activity
/// indicator animates identically.
pub(crate) const SPINNER_FRAMES: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

/// Render a thinking block as a collapsible card.
///
/// Layout:
/// ```text
///      [Thinking] Thought  (or spinner + "Thinking..." if streaming)
///      | reasoning line 1
///      | reasoning line 2
///      | ...
/// ```
fn render_think_card(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    content: &str,
    is_complete: bool,
    tick: u64,
    t: &Theme,
) {
    // Thought cards are top-level separators now: no indent.
    let indent = "";

    let header_spans = if is_complete {
        let label_style = Style::default()
            .fg(t.think_accent())
            .add_modifier(Modifier::BOLD);
        vec![
            Span::styled(
                format!("{indent}\u{25b8} "),
                Style::default().fg(t.think_accent()),
            ),
            Span::styled("Thought".to_string(), label_style),
        ]
    } else {
        let frame_idx = (tick as usize) % SPINNER_FRAMES.len();
        let spinner = SPINNER_FRAMES[frame_idx];
        let label_style = Style::default()
            .fg(t.think_accent())
            .add_modifier(Modifier::BOLD);
        vec![
            Span::styled(
                format!("{indent}{spinner} "),
                Style::default().fg(t.think_accent()),
            ),
            Span::styled("Thinking...".to_string(), label_style),
        ]
    };

    let label = if is_complete {
        "Thought"
    } else {
        "Thinking..."
    };
    let header_plain = format!("{indent}> {label}");
    lines.push(Line::from(header_spans));
    plain.push(header_plain);

    if is_complete {
        let content_lines: Vec<&str> = content.lines().collect();
        let preview_count = 3.min(content_lines.len());
        for line_text in content_lines.iter().take(preview_count) {
            let formatted = format!("{indent}\u{2502} {line_text}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default().fg(t.think_text()),
            )));
            plain.push(formatted);
        }
        if content_lines.len() > preview_count {
            let more = content_lines.len() - preview_count;
            let more_text = format!("{indent}\u{2502} ... ({more} more lines)");
            lines.push(Line::from(Span::styled(
                more_text.clone(),
                Style::default().fg(t.muted()),
            )));
            plain.push(more_text);
        }
    } else {
        for line_text in content.lines() {
            let formatted = format!("{indent}\u{2502} {line_text}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default().fg(t.think_text()),
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
///        <arguments in accent color>
/// ```
fn render_tool_call_card(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    name: &str,
    arguments: Option<&str>,
    is_streaming: bool,
    t: &Theme,
) {
    let indent = "";

    let (status_label, status_color) = if is_streaming {
        ("Running...", t.warning())
    } else {
        ("Done", t.success())
    };

    let header_spans = vec![
        Span::styled(
            format!("{indent}\u{2022} "),
            Style::default().fg(t.tool_card_accent()),
        ),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(t.tool_card_accent())
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
                Style::default().fg(t.tool_card_text()),
            )));
            plain.push(args_line);
        }
    }
}

/// Render a tool call from structured `ToolCallInfo` (from `ToolCallExecuted` events).
///
/// Records each rendered card's raw line span into `tool_ranges`.
fn render_tool_index_run(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    tool_ranges: &mut Vec<(usize, Range<usize>)>,
    tools: &[ToolCallInfo],
    tool_indexes: &[usize],
    message_index: usize,
    selected_tool: Option<ToolSelection>,
    content_width: usize,
    t: &Theme,
) {
    for group in group_tool_indexes(tools, tool_indexes) {
        match group {
            ToolRenderGroup::Single(tool_index) => {
                if let Some(tool) = tools.get(tool_index) {
                    render_tool_call_executed_card(
                        lines,
                        plain,
                        tool_ranges,
                        tool,
                        tool_index,
                        selected_tool
                            == Some(ToolSelection {
                                message_index,
                                tool_index,
                            }),
                        content_width,
                        t,
                    );
                }
            }
            ToolRenderGroup::Exploration(group_indexes) => render_exploration_group(
                lines,
                plain,
                tool_ranges,
                tools,
                &group_indexes,
                message_index,
                selected_tool,
                content_width,
                t,
            ),
        }
    }
}

/// Render an exploration group (collapsed run of read/search/list calls).
///
/// Each child card gets its own entry in `tool_ranges`; the group header line
/// belongs to no child and is left out of the ranges.
fn render_exploration_group(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    tool_ranges: &mut Vec<(usize, Range<usize>)>,
    tools: &[ToolCallInfo],
    tool_indexes: &[usize],
    message_index: usize,
    selected_tool: Option<ToolSelection>,
    content_width: usize,
    t: &Theme,
) {
    let indent = "";
    let total_duration: u64 = tool_indexes
        .iter()
        .filter_map(|index| tools.get(*index)?.duration_ms)
        .sum();
    let timing = if total_duration == 0 {
        String::new()
    } else {
        format!(" ({})", humanize_ms(total_duration))
    };
    let group_selected = selected_tool.is_some_and(|selected| {
        selected.message_index == message_index && tool_indexes.contains(&selected.tool_index)
    });
    let header_marker = if group_selected { ">" } else { "\u{2022}" };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{indent}{header_marker} "),
            Style::default().fg(if group_selected {
                t.input_border_focused()
            } else {
                t.tool_card_accent()
            }),
        ),
        Span::styled(
            "Exploring",
            Style::default()
                .fg(t.tool_card_accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} calls", tool_indexes.len()),
            Style::default().fg(t.muted()),
        ),
        Span::styled(format!("  Done{timing}"), Style::default().fg(t.success())),
    ]));
    plain.push(format!(
        "{indent}{header_marker} Exploring  {} calls  Done{timing}",
        tool_indexes.len()
    ));

    for &tool_index in tool_indexes {
        let Some(tool) = tools.get(tool_index) else {
            continue;
        };
        let child_start = lines.len();
        let selected = selected_tool
            == Some(ToolSelection {
                message_index,
                tool_index,
            });
        let timing = tool.duration_ms.map_or_else(String::new, |duration| {
            format!(" ({})", humanize_ms(duration))
        });
        let selected_label = if selected { "  [selected]" } else { "" };
        let marker = if selected { ">" } else { "-" };

        // Cheap path: collapsed or unselected children render only the
        // one-line input summary, skipping result parsing, preview lines,
        // and presentation wrapping.
        if !selected || tool.display_mode == ToolCallDisplayMode::Collapsed {
            let quick = quick_summary(tool);
            let summary = if quick.is_empty() {
                tool.name.clone()
            } else {
                truncate_str(&quick, 56)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{indent}  {marker} "),
                    Style::default().fg(if selected {
                        t.input_border_focused()
                    } else {
                        t.muted()
                    }),
                ),
                Span::styled(summary.clone(), Style::default().fg(t.tool_card_text())),
                Span::styled(timing.clone(), Style::default().fg(t.muted())),
                Span::styled(
                    selected_label,
                    Style::default().fg(t.input_border_focused()),
                ),
            ]));
            plain.push(format!(
                "{indent}  {marker} {summary}{timing}{selected_label}"
            ));
        } else {
            let presentation = present_tool(tool, content_width.saturating_sub(indent.len() + 6));
            let summary = if presentation.summary.is_empty() {
                tool.name.as_str()
            } else {
                presentation.summary.as_str()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{indent}  {marker} "),
                    Style::default().fg(if selected {
                        t.input_border_focused()
                    } else {
                        t.muted()
                    }),
                ),
                Span::styled(
                    format!("{} {}", presentation.verb, truncate_str(summary, 56)),
                    Style::default().fg(t.tool_card_text()),
                ),
                Span::styled(timing.clone(), Style::default().fg(t.muted())),
                Span::styled(
                    selected_label,
                    Style::default().fg(t.input_border_focused()),
                ),
            ]));
            plain.push(format!(
                "{indent}  {marker} {} {}{timing}{selected_label}",
                presentation.verb,
                truncate_str(summary, 56)
            ));

            if tool.display_mode == ToolCallDisplayMode::Expanded {
                push_tool_section(
                    lines,
                    plain,
                    &presentation.argument_lines,
                    80,
                    t.tool_card_text(),
                    t,
                    false,
                );
            }
            let result_limit = match tool.display_mode {
                ToolCallDisplayMode::Collapsed => 0,
                ToolCallDisplayMode::Preview => 4,
                ToolCallDisplayMode::Expanded => 200,
            };
            push_tool_section(
                lines,
                plain,
                &presentation.result_lines,
                result_limit,
                t.muted(),
                t,
                presentation.kind == ToolKind::Shell,
            );
        }
        tool_ranges.push((tool_index, child_start..lines.len()));
    }
}

/// Render a tool call from structured `ToolCallInfo` (from `ToolCallExecuted` events).
///
/// Pushes the card's raw line span as `(tool_index, range)` onto `tool_ranges`.
fn render_tool_call_executed_card(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    tool_ranges: &mut Vec<(usize, Range<usize>)>,
    tc: &ToolCallInfo,
    tool_index: usize,
    selected: bool,
    content_width: usize,
    t: &Theme,
) {
    let card_start = lines.len();
    let indent = "";

    let (status_label, status_color) = match tc.status {
        ToolCallStatus::Running => ("Running", t.warning()),
        ToolCallStatus::Succeeded => ("Done", t.success()),
        ToolCallStatus::Failed => ("Failed", t.error()),
    };
    let presentation = present_tool(tc, content_width.saturating_sub(indent.len() + 4));
    let timing = tc.duration_ms.map_or_else(String::new, |duration| {
        format!(" ({})", humanize_ms(duration))
    });
    let collapsed_summary =
        if tc.display_mode == ToolCallDisplayMode::Collapsed && !presentation.summary.is_empty() {
            format!("  {}", truncate_str(&presentation.summary, 48))
        } else {
            String::new()
        };
    let meta_chips = if presentation.meta.is_empty() {
        String::new()
    } else {
        format!("  {}", presentation.meta.join(" · "))
    };

    let selected_label = if selected { "  [selected]" } else { "" };
    let heading =
        if tc.display_mode != ToolCallDisplayMode::Collapsed && !presentation.summary.is_empty() {
            format!(
                "{} {}",
                presentation.verb,
                truncate_str(&presentation.summary, 56)
            )
        } else {
            format!("{} {}", presentation.verb, tc.name)
        };
    let header_spans = vec![
        Span::styled(
            format!("{indent}{} ", if selected { ">" } else { "\u{2022}" }),
            Style::default().fg(if selected {
                t.input_border_focused()
            } else {
                t.tool_card_accent()
            }),
        ),
        Span::styled(
            heading.clone(),
            Style::default()
                .fg(t.tool_card_accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(collapsed_summary.clone(), Style::default().fg(t.muted())),
        Span::styled(meta_chips.clone(), Style::default().fg(t.muted())),
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{status_label}{timing}"),
            Style::default().fg(status_color),
        ),
        Span::styled(
            selected_label,
            Style::default().fg(t.input_border_focused()),
        ),
    ];
    let header_plain = format!(
        "{indent}{} {}{}{}  {status_label}{timing}{selected_label}",
        if selected { ">" } else { "*" },
        heading,
        collapsed_summary,
        meta_chips
    );
    lines.push(Line::from(header_spans));
    plain.push(header_plain);

    if tc.display_mode != ToolCallDisplayMode::Collapsed {
        if tc.display_mode == ToolCallDisplayMode::Expanded
            && !presentation.argument_lines.is_empty()
        {
            push_tool_section(
                lines,
                plain,
                &presentation.argument_lines,
                80,
                t.tool_card_text(),
                t,
                false,
            );
        }

        let result_limit = match tc.display_mode {
            ToolCallDisplayMode::Collapsed => 0,
            ToolCallDisplayMode::Preview => 4,
            ToolCallDisplayMode::Expanded => 200,
        };
        // Shell output keeps the *tail* in preview: build/log failures carry
        // their signal in the last lines, not the first.
        let tail = presentation.kind == ToolKind::Shell;
        push_tool_section(
            lines,
            plain,
            &presentation.result_lines,
            result_limit,
            t.muted(),
            t,
            tail,
        );
    }

    tool_ranges.push((tool_index, card_start..lines.len()));
}

/// Push tool output lines with plain indentation. `Plain`-toned lines take
/// `color`; other tones map onto theme roles (diff colors, stderr, dim).
/// When `tail` is set and the content overflows `limit`, the *last* `limit`
/// lines are kept (logs carry their signal at the end) behind an `N earlier
/// lines` marker; otherwise the first `limit` lines are kept.
fn push_tool_section(
    lines: &mut Vec<Line>,
    plain: &mut Vec<String>,
    content: &[ToolLine],
    limit: usize,
    color: Color,
    t: &Theme,
    tail: bool,
) {
    if content.is_empty() || limit == 0 {
        return;
    }
    let indent = "";
    let (window, earlier) = if tail && content.len() > limit {
        (&content[content.len() - limit..], content.len() - limit)
    } else {
        (&content[..content.len().min(limit)], 0)
    };
    if earlier > 0 {
        let marker = format!("{indent}    ... {earlier} earlier lines (ctrl+o to expand)");
        lines.push(Line::from(Span::styled(
            marker.clone(),
            Style::default().fg(t.muted()),
        )));
        plain.push(marker);
    }
    for tool_line in window {
        let output_line = format!("{indent}  {}", tool_line.text);
        lines.push(Line::from(Span::styled(
            output_line.clone(),
            Style::default().fg(tool_tone_color(tool_line.tone, color, t)),
        )));
        plain.push(output_line);
    }
    if earlier == 0 && content.len() > window.len() {
        let omitted = content.len() - window.len();
        let more_line = format!("{indent}    ... {omitted} more lines (ctrl+o to expand)");
        lines.push(Line::from(Span::styled(
            more_line.clone(),
            Style::default().fg(t.muted()),
        )));
        plain.push(more_line);
    }
}

/// Map a line's semantic tone onto a concrete theme color; `default` is the
/// section color chosen by the caller (args vs results).
fn tool_tone_color(tone: ToolTone, default: Color, t: &Theme) -> Color {
    match tone {
        ToolTone::Plain => default,
        ToolTone::Dim => t.muted(),
        ToolTone::Added => t.success(),
        ToolTone::Removed => t.error(),
        ToolTone::Stderr => t.warning(),
    }
}

/// `123ms` under a second, `1.2s` above — compact timing for card headers.
fn humanize_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
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
    content_width: usize,
    t: &Theme,
) {
    let indent = "";
    let content_style = match role {
        // The user's own text is the one highlighted element in the
        // transcript: bold + accent makes sent messages pop without a
        // "You" header line.
        MessageRole::User => Style::default()
            .fg(t.user_accent())
            .add_modifier(Modifier::BOLD),
        MessageRole::Assistant => Style::default().fg(t.text()),
        MessageRole::System => Style::default().fg(t.system_accent()),
        MessageRole::Tool => Style::default().fg(t.normal()),
    };

    // Use pulldown-cmark-based markdown renderer for assistant messages.
    if role == MessageRole::Assistant {
        let md_lines = crate::tui::markdown::render_markdown(content, content_width);
        for md_line in md_lines {
            let plain_text: String = md_line.spans.iter().map(|s| s.content.as_ref()).collect();
            let plain = format!("{indent}{plain_text}");
            let mut spans = vec![Span::raw(indent.to_string())];
            spans.extend(md_line.spans);
            lines.push(Line::from(spans));
            plain_lines.push(plain);
        }
        return;
    }

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
                    Style::default().fg(t.muted()),
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
                    Style::default().fg(t.muted()),
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
                Style::default().fg(t.code_block_fg()).bg(t.code_bg()),
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

        // Horizontal rules (---, ***, ___).
        if is_horizontal_rule(trimmed) {
            let hr_width = 40;
            let hr_line = format!("{indent}{}", "\u{2500}".repeat(hr_width));
            lines.push(Line::from(Span::styled(
                hr_line.clone(),
                Style::default().fg(t.hr()),
            )));
            plain_lines.push(hr_line);
            continue;
        }

        // Blockquotes (> text).
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let formatted = format!("{indent}\u{2502} {rest}");
            let spans = build_inline_spans(&formatted, Style::default().fg(t.blockquote()), t);
            lines.push(Line::from(spans));
            plain_lines.push(formatted);
            continue;
        }
        // Bare blockquote marker.
        if trimmed == ">" {
            let formatted = format!("{indent}\u{2502}");
            lines.push(Line::from(Span::styled(
                formatted.clone(),
                Style::default().fg(t.blockquote()),
            )));
            plain_lines.push(formatted);
            continue;
        }

        // Bullet list items.
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let bullet_content = &trimmed[2..];
            let formatted = format!("{indent}  {bullet_content}");
            let spans = build_inline_spans(&formatted, content_style, t);
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
            let spans = build_inline_spans(&formatted, content_style, t);
            lines.push(Line::from(spans));
            plain_lines.push(formatted);
            continue;
        }

        // Regular content line with inline formatting.
        let formatted = format!("{indent}{raw_line}");
        let spans = build_inline_spans(&formatted, content_style, t);
        lines.push(Line::from(spans));
        plain_lines.push(formatted);
    }
}

/// Check if a trimmed line is a horizontal rule (---, ***, ___).
fn is_horizontal_rule(trimmed: &str) -> bool {
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed.chars().next().unwrap_or(' ');
    matches!(first, '-' | '*' | '_') && trimmed.chars().all(|c| c == first || c == ' ')
}

/// Build styled spans for a line with inline markdown:
///   - `**bold**` -> bold
///   - `*italic*` -> italic
///   - `~~strikethrough~~` -> crossed out
///   - `` `code` `` -> code style
fn build_inline_spans(text: &str, base_style: Style, t: &Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let code_style = Style::default().fg(t.code_fg()).bg(t.code_bg());
    let bold_style = base_style.add_modifier(Modifier::BOLD);
    let italic_style = base_style.add_modifier(Modifier::ITALIC);
    let strikethrough_style = base_style.add_modifier(Modifier::CROSSED_OUT);

    while i < len {
        // Strikethrough: ~~...~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if !buf.is_empty() {
                spans.push(Span::styled(buf.clone(), base_style));
                buf.clear();
            }
            i += 2;
            let mut strike_buf = String::new();
            while i + 1 < len && !(chars[i] == '~' && chars[i + 1] == '~') {
                strike_buf.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing ~~
            }
            if !strike_buf.is_empty() {
                spans.push(Span::styled(strike_buf, strikethrough_style));
            }
            continue;
        }

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

        // Italic: *...* (single asterisk, not followed by another *)
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            // Look for closing single *
            let mut end = i + 1;
            while end < len && chars[end] != '*' {
                end += 1;
            }
            if end < len && end > i + 1 {
                if !buf.is_empty() {
                    spans.push(Span::styled(buf.clone(), base_style));
                    buf.clear();
                }
                let italic_buf: String = chars[i + 1..end].iter().collect();
                spans.push(Span::styled(italic_buf, italic_style));
                i = end + 1; // skip closing *
                continue;
            }
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // T-TUI-02-04: Chat scroll offset limits clamp to message count.
    #[test]
    fn test_scroll_offset_clamping() {
        assert_eq!(
            compute_scroll_to(5, 20, 0),
            0,
            "no scroll when content fits"
        );
        assert_eq!(
            compute_scroll_to(100, 20, 500),
            0,
            "huge offset clamps to top"
        );
        assert_eq!(compute_scroll_to(100, 20, 30), 50);
    }

    #[test]
    fn test_render_message_creates_lines() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Hello\nWorld".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();
        let theme = Theme::default();
        render_message(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &msg,
            0,
            None,
            false,
            0,
            80,
            &theme,
        );

        // No role header line: just the 2 content lines.
        assert_eq!(lines.len(), 2);
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0], "Hello");
        // The user's own text carries the highlight: bold + user accent.
        let first_span = &lines[0].spans[0];
        assert_eq!(first_span.style.fg, Some(theme.user_accent()));
        assert!(first_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_streaming_placeholder_for_empty_message() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        };
        let placeholder_at = |tick: u64| {
            let mut lines = Vec::new();
            let mut plain = Vec::new();
            let mut tool_ranges = Vec::new();
            render_message(
                &mut lines,
                &mut plain,
                &mut tool_ranges,
                &msg,
                0,
                None,
                false,
                tick,
                80,
                &Theme::default(),
            );

            assert_eq!(lines.len(), 1, "placeholder must be a single line");
            assert!(plain[0].contains(SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]));
            lines[0]
                .spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };

        // The streaming placeholder is the animated braille spinner.
        assert!(placeholder_at(0).contains(SPINNER_FRAMES[0]));
        assert!(placeholder_at(1).contains(SPINNER_FRAMES[1]));
        assert!(!placeholder_at(0).contains(SPINNER_FRAMES[1]));
    }

    #[test]
    fn test_reasoning_cards_render_at_their_timeline_positions() {
        let tool = ToolCallInfo {
            tool_call_id: "call-read".into(),
            name: "FileRead".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(5),
            input_preview: r#"{"path":"src/lib.rs"}"#.into(),
            result_preview: "contents".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        };
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "The result is valid.".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: "Inspect the fileNow verify the result".into(),
            reasoning_complete: false,
            tool_calls: vec![tool],
            segments: vec![
                StreamSegment::Reasoning {
                    content: "Inspect the file".into(),
                    is_complete: true,
                },
                StreamSegment::ToolCall(0),
                StreamSegment::Reasoning {
                    content: "Now verify the result".into(),
                    is_complete: true,
                },
                StreamSegment::Text("The result is valid.".into()),
            ],
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();

        render_message(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &msg,
            0,
            None,
            false,
            0,
            80,
            &Theme::default(),
        );

        let first_reasoning = plain
            .iter()
            .position(|line| line.contains("Inspect the file"))
            .unwrap();
        let tool = plain
            .iter()
            .position(|line| line.contains("Read src/lib.rs"))
            .unwrap();
        let second_reasoning = plain
            .iter()
            .position(|line| line.contains("Now verify the result"))
            .unwrap();
        let answer = plain
            .iter()
            .position(|line| line.contains("The result is valid."))
            .unwrap();

        assert!(first_reasoning < tool);
        assert!(tool < second_reasoning);
        assert!(second_reasoning < answer);
    }

    /// Wrap a plain-text string through the span-aware wrap path (single
    /// default-style span) and return the plain rows.
    fn wrap_plain(text: &str, max_width: usize) -> Vec<String> {
        let line = Line::from(Span::raw(text.to_string()));
        wrap_spans(&line.spans, max_width)
            .iter()
            .map(|row| row.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn test_wrap_plain_no_wrap_needed() {
        let result = wrap_plain("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_plain_exact_fit() {
        let result = wrap_plain("12345", 5);
        assert_eq!(result, vec!["12345"]);
    }

    #[test]
    fn test_wrap_plain_splits() {
        let result = wrap_plain("abcdefghij", 5);
        assert_eq!(result, vec!["abcde", "fghij"]);
    }

    #[test]
    fn test_wrap_plain_empty() {
        let result = wrap_plain("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_wrap_plain_cjk_double_width() {
        let result = wrap_plain("你好世界测试", 6);
        assert_eq!(result, vec!["你好世", "界测试"]);
    }

    // T-CHAT-WRAP-SPAN: a wrapped multi-span line keeps every span's style
    // instead of collapsing to the first span's style.
    #[test]
    fn test_wrap_rendered_lines_preserves_span_styles() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let raw_line = Line::from(vec![Span::styled("aaaa", red), Span::styled("bbbb", blue)]);
        let (lines, plain, counts) =
            wrap_rendered_lines(vec![raw_line], vec!["aaaabbbb".to_string()], 6);

        assert_eq!(lines.len(), 2);
        assert_eq!(counts, vec![2], "one raw line split into two rows");
        assert_eq!(plain, vec!["aaaabb".to_string(), "bb".to_string()]);
        // First row: the red span intact plus the first half of the blue span.
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "aaaa");
        assert_eq!(lines[0].spans[0].style, red);
        assert_eq!(lines[0].spans[1].content.as_ref(), "bb");
        assert_eq!(lines[0].spans[1].style, blue);
        // Second row: the blue remainder keeps its style.
        assert_eq!(lines[1].spans[0].content.as_ref(), "bb");
        assert_eq!(lines[1].spans[0].style, blue);
    }

    // T-CHAT-WRAP-CJK: wide characters wrap by display width and are never
    // split across rows.
    #[test]
    fn test_wrap_rendered_lines_cjk_wide_chars() {
        let cjk_style = Style::default().fg(Color::Green);
        let raw_line = Line::from(vec![
            Span::styled("ab", Style::default()),
            Span::styled("你好世界", cjk_style),
        ]);
        let (lines, plain, counts) =
            wrap_rendered_lines(vec![raw_line], vec!["ab你好世界".to_string()], 6);

        // "ab你好" is exactly 6 display columns; "世界" wraps to the next row.
        assert_eq!(plain, vec!["ab你好".to_string(), "世界".to_string()]);
        assert_eq!(counts, vec![2]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].spans[0].style, cjk_style);
    }

    // T-CHAT-WRAP-INVARIANT: the plain mirror of each wrapped row is the exact
    // concatenation of that row's span contents, and the rows reassemble into
    // the original text (selection row mapping depends on this 1:1 alignment).
    #[test]
    fn test_wrap_rendered_lines_plain_invariant() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let code = Style::default().bg(Color::DarkGray);
        let original = "some longer text with code mixed in".to_string();
        let raw_line = Line::from(vec![
            Span::styled("some longer ", Style::default()),
            Span::styled("text with ", bold),
            Span::styled("code mixed in", code),
        ]);
        let (lines, plain, counts) =
            wrap_rendered_lines(vec![raw_line], vec![original.clone()], 10);

        assert!(lines.len() > 1, "line must actually wrap");
        assert_eq!(counts, vec![lines.len()]);
        assert_eq!(lines.len(), plain.len());
        for (line, plain_row) in lines.iter().zip(&plain) {
            let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(&joined, plain_row);
        }
        assert_eq!(plain.concat(), original);
    }

    #[test]
    fn test_display_items_empty_state() {
        let state = AppState::new();
        let items = build_display_items(&state);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], DisplayItem::WelcomeScreen));
    }

    #[test]
    fn test_welcome_screen_advertises_command_first_workflow() {
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_welcome(&mut lines, &mut plain, 80, &Theme::default());
        let text = plain.join("\n");

        assert!(text.contains("/goal"));
        assert!(text.contains("/mode"));
        assert!(text.contains("/resume"));
        assert!(text.contains("/copy"));
    }

    #[test]
    fn test_display_items_with_messages() {
        let mut state = AppState::new();
        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        });
        let items = build_display_items(&state);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], DisplayItem::Message { .. }));
    }

    #[test]
    fn test_inline_bold_formatting() {
        let t = Theme::default();
        let base = Style::default().fg(t.text());
        let spans = build_inline_spans("hello **world** end", base, &t);
        assert!(spans.len() >= 3, "expected at least 3 spans for bold text");
        let bold_span = &spans[1];
        assert_eq!(bold_span.content.as_ref(), "world");
    }

    #[test]
    fn test_inline_code_formatting() {
        let t = Theme::default();
        let base = Style::default().fg(t.text());
        let spans = build_inline_spans("run `cargo test` now", base, &t);
        assert!(
            spans.len() >= 3,
            "expected at least 3 spans for inline code"
        );
    }

    #[test]
    fn test_code_block_rendering() {
        let t = Theme::default();
        let content = "text\n```rust\nfn main() {}\n```\nmore";
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        render_content_lines(
            &mut lines,
            &mut plain,
            content,
            MessageRole::Assistant,
            80,
            &t,
        );
        // Markdown renderer produces lines for: text, lang label, code line, more.
        assert!(lines.len() >= 3, "code block should produce multiple lines");
    }

    #[test]
    fn test_tool_card_renders_semantic_action_arguments_and_result() {
        let tool = ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(238),
            input_preview: r#"{"command":"cargo test"}"#.into(),
            result_preview: "test result: ok\n42 passed".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Expanded,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();

        render_tool_call_executed_card(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &tool,
            0,
            true,
            80,
            &Theme::default(),
        );

        let text = plain.join("\n");
        // The header carries the semantic action, not the raw tool name.
        assert!(text.contains("Ran cargo test"));
        // No raw `command=` line, no Arguments/Result labels, no box glyphs.
        assert!(!text.contains("command="));
        assert!(!text.contains("Arguments:"));
        assert!(!text.contains("Result:"));
        assert!(!text.contains('\u{2514}'));
        assert!(text.contains("test result: ok"));
        assert!(text.contains("42 passed"));
        assert!(text.contains("selected"));
        // The whole card is one recorded range for tool index 0.
        assert_eq!(tool_ranges, vec![(0, 0..lines.len())]);
    }

    #[test]
    fn test_collapsed_tool_card_keeps_summary_and_hides_result() {
        let tool = ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(10),
            input_preview: r#"{"command":"cargo check"}"#.into(),
            result_preview: "hidden result".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Collapsed,
        };
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();

        render_tool_call_executed_card(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &tool,
            0,
            false,
            80,
            &Theme::default(),
        );

        let text = plain.join("\n");
        assert!(text.contains("cargo check"));
        assert!(!text.contains("hidden result"));
    }

    #[test]
    fn test_exploration_group_renders_selected_child_detail() {
        let tools = vec![
            ToolCallInfo {
                tool_call_id: "call-read-1".into(),
                name: "FileRead".into(),
                status: ToolCallStatus::Succeeded,
                duration_ms: Some(10),
                input_preview: r#"{"path":"src/lib.rs"}"#.into(),
                result_preview: "library contents".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
                display_mode: ToolCallDisplayMode::Preview,
            },
            ToolCallInfo {
                tool_call_id: "call-search-1".into(),
                name: "FileSearch".into(),
                status: ToolCallStatus::Succeeded,
                duration_ms: Some(15),
                input_preview: r#"{"query":"ToolCallInfo"}"#.into(),
                result_preview: "3 matches".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
                display_mode: ToolCallDisplayMode::Expanded,
            },
        ];
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();
        let message = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: chrono::Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: tools,
            segments: vec![StreamSegment::ToolCall(0), StreamSegment::ToolCall(1)],
        };

        render_message(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &message,
            0,
            Some(ToolSelection {
                message_index: 0,
                tool_index: 1,
            }),
            false,
            0,
            80,
            &Theme::default(),
        );

        let text = plain.join("\n");
        assert_eq!(text.matches("Exploring").count(), 1);
        // Unselected children render the cheap one-line input summary (no
        // result parsing, no detail sections).
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains("library contents"));
        // The selected child keeps the full presentation and detail sections.
        assert!(text.contains("Searched ToolCallInfo"));
        assert!(text.contains("selected"));
        assert!(text.contains("3 matches"));
    }

    // Collapsed group children take the cheap summary path even when
    // selected: the input summary is shown, result detail is not.
    #[test]
    fn test_exploration_group_collapsed_child_renders_summary_only() {
        let tools = vec![
            ToolCallInfo {
                tool_call_id: "call-read-1".into(),
                name: "FileRead".into(),
                status: ToolCallStatus::Succeeded,
                duration_ms: Some(10),
                input_preview: r#"{"path":"src/lib.rs"}"#.into(),
                result_preview: "library contents".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
                display_mode: ToolCallDisplayMode::Collapsed,
            },
            ToolCallInfo {
                tool_call_id: "call-search-1".into(),
                name: "FileSearch".into(),
                status: ToolCallStatus::Succeeded,
                duration_ms: Some(15),
                input_preview: r#"{"query":"ToolCallInfo"}"#.into(),
                result_preview: "3 matches".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
                display_mode: ToolCallDisplayMode::Collapsed,
            },
        ];
        let mut lines = Vec::new();
        let mut plain = Vec::new();
        let mut tool_ranges = Vec::new();

        render_exploration_group(
            &mut lines,
            &mut plain,
            &mut tool_ranges,
            &tools,
            &[0, 1],
            0,
            Some(ToolSelection {
                message_index: 0,
                tool_index: 0,
            }),
            80,
            &Theme::default(),
        );

        let text = plain.join("\n");
        // Collapsed children show the cheap input summary but no result detail.
        assert!(text.contains("src/lib.rs"));
        assert!(text.contains("ToolCallInfo"));
        assert!(!text.contains("library contents"));
        assert!(!text.contains("3 matches"));
    }

    // -----------------------------------------------------------------------
    // Scroll computation / visible-window slicing
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_scroll_to_bottom_without_offset() {
        assert_eq!(compute_scroll_to(100, 20, 0), 80);
        assert_eq!(compute_scroll_to(10, 20, 0), 0, "content fits, no scroll");
    }

    #[test]
    fn test_compute_scroll_to_clamps_at_top() {
        assert_eq!(compute_scroll_to(100, 20, 10), 70);
        assert_eq!(
            compute_scroll_to(100, 20, 999),
            0,
            "huge offset clamps to top"
        );
    }

    // T-CHAT-SCROLL-U16: histories taller than u16::MAX must not reset scroll.
    #[test]
    fn test_compute_scroll_to_supports_histories_beyond_u16() {
        let total = 70_000;
        let height = 50;
        assert_eq!(compute_scroll_to(total, height, 0), 69_950);
        assert_eq!(compute_scroll_to(total, height, 69_950), 0);
    }

    /// Extract the full text of a `TestBackend` terminal buffer, one
    /// buffer row per line.
    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }
        text
    }

    fn user_message(content: String) -> ChatMessage {
        ChatMessage {
            role: MessageRole::User,
            content,
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        }
    }

    // T-CHAT-SCROLL-U16-RENDER: rendering a >u16::MAX-row history must show the
    // tail (not jump back to the top) and must not panic.
    #[test]
    fn test_render_shows_tail_for_history_beyond_u16_lines() {
        let mut state = AppState::new();
        // 3000 messages x 27 rows (header + 25 content + separator) > 65535 rows.
        for i in 0..3000 {
            let body = (0..25)
                .map(|j| format!("m{i}-r{j}"))
                .collect::<Vec<_>>()
                .join("\n");
            state.messages.push(user_message(body));
        }

        let backend = ratatui::backend::TestBackend::new(60, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut cache = ChatRenderCache::default();
        let mut plain = Vec::new();
        let mut tool_rows = Vec::new();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &state,
                    &mut cache,
                    &mut plain,
                    &mut tool_rows,
                );
            })
            .unwrap();

        let total_rows = plain.len();
        assert!(
            total_rows > u16::MAX as usize,
            "test requires more than u16::MAX rows, got {total_rows}"
        );

        let text = buffer_text(&terminal);
        assert!(
            text.contains("m2999-r24"),
            "tail of a huge history must stay visible:\n{text}"
        );
        assert!(
            !text.contains("m0-r0"),
            "top of a bottom-scrolled history must not be rendered:\n{text}"
        );
    }

    // T-CHAT-PLAIN-FULL: the plain-lines buffer must cover the full history
    // even when scrolled to the top, so selection extraction by absolute row
    // keeps working.
    #[test]
    fn test_render_plain_lines_cover_full_history_when_scrolled_up() {
        let mut state = AppState::new();
        for i in 0..50 {
            state.messages.push(user_message(format!("message {i}")));
        }
        state.scroll_offset = usize::MAX / 2; // scroll to top

        let backend = ratatui::backend::TestBackend::new(60, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut cache = ChatRenderCache::default();
        let mut plain = Vec::new();
        let mut tool_rows = Vec::new();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &state,
                    &mut cache,
                    &mut plain,
                    &mut tool_rows,
                );
            })
            .unwrap();

        // 50 messages x 1 content line + 49 separators = 99 rows.
        assert_eq!(plain.len(), 99);
        assert!(plain.iter().any(|line| line.contains("message 0")));
        assert!(plain.iter().any(|line| line.contains("message 49")));

        let text = buffer_text(&terminal);
        assert!(
            text.contains("message 0"),
            "scrolled to top must show the first message:\n{text}"
        );
    }

    // T-CHAT-SELECTION-SLICE: selection highlight uses absolute row indices
    // and must still apply to the sliced visible window.
    #[test]
    fn test_render_selection_highlight_after_slicing() {
        let mut state = AppState::new();
        for i in 0..10 {
            state.messages.push(user_message(format!("selectable {i}")));
        }
        state.scroll_offset = usize::MAX / 2; // top of history
        state.selection.start(0, 0);
        state.selection.update(2, 5);

        let backend = ratatui::backend::TestBackend::new(60, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut cache = ChatRenderCache::default();
        let mut plain = Vec::new();
        let mut tool_rows = Vec::new();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &state,
                    &mut cache,
                    &mut plain,
                    &mut tool_rows,
                );
            })
            .unwrap();

        // Row 1 (buffer y = 2) lies fully inside the selection and must be
        // highlighted (white background).
        let buffer = terminal.backend().buffer();
        let cell = buffer.cell((2, 2)).unwrap();
        assert_eq!(
            cell.bg,
            Color::White,
            "selected row should be highlighted after visible-window slicing"
        );
    }

    // -----------------------------------------------------------------------
    // Per-message render cache
    // -----------------------------------------------------------------------

    #[test]
    fn test_cached_message_render_hit_and_invalidation() {
        let theme = Theme::default();
        let mut cache = ChatRenderCache::default();
        let msg = user_message("hello world".to_string());

        let g1 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        let g2 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        assert_eq!(g1, g2, "unchanged inputs must be a cache hit");

        // Width change (resize) invalidates.
        let g3 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 40, &theme).generation;
        assert_ne!(g1, g3, "width change must re-render");

        // Content change invalidates.
        let mut changed = msg.clone();
        changed.content.push_str(" extra");
        let g4 =
            cached_message_render(&mut cache, 0, &changed, None, true, 0, 40, &theme).generation;
        assert_ne!(g3, g4, "content change must re-render");

        // Tool selection change invalidates.
        let selection = Some(ToolSelection {
            message_index: 0,
            tool_index: 0,
        });
        let g5 = cached_message_render(&mut cache, 0, &changed, selection, true, 0, 40, &theme)
            .generation;
        assert_ne!(g4, g5, "selected_tool change must re-render");

        // Tail position change (footer) invalidates.
        let g6 = cached_message_render(&mut cache, 0, &changed, selection, false, 0, 40, &theme)
            .generation;
        assert_ne!(g5, g6, "is_last change must re-render");
    }

    #[test]
    fn test_cached_message_render_animated_revalidates_per_tick() {
        let theme = Theme::default();
        let mut cache = ChatRenderCache::default();
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: vec![StreamSegment::Reasoning {
                content: "working on it".into(),
                is_complete: false,
            }],
        };

        let g1 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        let g2 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        assert_eq!(g1, g2, "same tick must be a cache hit");
        let g3 = cached_message_render(&mut cache, 0, &msg, None, true, 1, 80, &theme).generation;
        assert_ne!(g1, g3, "animated spinner must re-render each tick");
    }

    #[test]
    fn test_cached_message_render_static_ignores_tick() {
        let theme = Theme::default();
        let mut cache = ChatRenderCache::default();
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "done".to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        };

        let g1 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        let g2 = cached_message_render(&mut cache, 0, &msg, None, true, 42, 80, &theme).generation;
        assert_eq!(g1, g2, "static message must ignore the animation tick");
    }

    #[test]
    fn test_message_has_active_spinner_detection() {
        let mut msg = user_message("plain text".to_string());
        assert!(!message_has_active_spinner(&msg));

        // A streaming message animates its header spinner every tick, even
        // without any reasoning content.
        msg.is_streaming = true;
        assert!(message_has_active_spinner(&msg));
        msg.is_streaming = false;

        // Unclosed <think> block animates the "Thinking..." spinner.
        msg.content = "before <think>still thinking".to_string();
        assert!(message_has_active_spinner(&msg));

        // Closed think block is static.
        msg.content = "before <think>done thinking</think> after".to_string();
        assert!(!message_has_active_spinner(&msg));

        // Incomplete event-ordered reasoning animates.
        let mut msg = user_message(String::new());
        msg.segments = vec![StreamSegment::Reasoning {
            content: "working".into(),
            is_complete: false,
        }];
        assert!(message_has_active_spinner(&msg));

        // Complete reasoning is static.
        msg.segments = vec![StreamSegment::Reasoning {
            content: "worked".into(),
            is_complete: true,
        }];
        assert!(!message_has_active_spinner(&msg));

        // Legacy aggregate reasoning path.
        let mut msg = user_message(String::new());
        msg.reasoning_content = "legacy".into();
        msg.reasoning_complete = false;
        assert!(message_has_active_spinner(&msg));
        msg.reasoning_complete = true;
        assert!(!message_has_active_spinner(&msg));
    }

    // -----------------------------------------------------------------------
    // Tool card line ranges (mouse hit-testing)
    // -----------------------------------------------------------------------

    /// A succeeded shell tool call in preview mode (3 card rows at width 80).
    fn shell_tool() -> ToolCallInfo {
        ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(10),
            input_preview: r#"{"command":"cargo test"}"#.into(),
            result_preview: "test result: ok".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        }
    }

    #[test]
    fn test_offset_tool_ranges_maps_raw_to_wrapped() {
        // Raw line 1 splits into 3 wrapped rows; everything after it shifts.
        let ranges = offset_tool_ranges(vec![(0, 1..2), (2, 2..3)], &[1, 3, 1]);
        assert_eq!(ranges, vec![(0, 1..4), (2, 4..5)]);

        // Unsplit lines keep their ranges verbatim.
        let ranges = offset_tool_ranges(vec![(0, 0..2)], &[1, 1]);
        assert_eq!(ranges, vec![(0, 0..2)]);
    }

    #[test]
    fn test_tool_ranges_single_tool_card() {
        let theme = Theme::default();
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: vec![shell_tool()],
            segments: vec![StreamSegment::ToolCall(0)],
        };

        let (lines, plain, ranges) = render_message_wrapped(&msg, 0, None, false, 0, 80, &theme);

        assert_eq!(ranges.len(), 1, "one tool card must record one range");
        let (tool_index, range) = &ranges[0];
        assert_eq!(*tool_index, 0);
        assert_eq!(range.start, 0, "no role header: the card opens the message");
        assert_eq!(range.end, lines.len());
        assert_eq!(range.end, plain.len());
        assert!(
            plain[range.start].contains("Ran cargo test"),
            "first row of the range must be the card header: {plain:?}"
        );
    }

    #[test]
    fn test_tool_ranges_exploration_group_children() {
        let theme = Theme::default();
        let exploration_tool = |id: &str, name: &str, input: &str| ToolCallInfo {
            tool_call_id: id.into(),
            name: name.into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(5),
            input_preview: input.into(),
            result_preview: "hidden".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Collapsed,
        };
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: vec![
                exploration_tool("call-read-1", "FileRead", r#"{"path":"src/lib.rs"}"#),
                exploration_tool("call-search-1", "FileSearch", r#"{"query":"ToolCallInfo"}"#),
            ],
            segments: vec![StreamSegment::ToolCall(0), StreamSegment::ToolCall(1)],
        };

        let (lines, plain, ranges) = render_message_wrapped(&msg, 0, None, false, 0, 80, &theme);

        // Layout: group header, child 0 row, child 1 row (no role header).
        assert_eq!(lines.len(), 3, "unexpected layout: {plain:?}");
        // Each child gets its own range; the group header belongs to no child.
        assert_eq!(ranges, vec![(0, 1..2), (1, 2..3)]);
        assert!(plain[1].contains("src/lib.rs"));
        assert!(plain[2].contains("ToolCallInfo"));
    }

    #[test]
    fn test_tool_ranges_after_wrapping_shifts_with_content() {
        let theme = Theme::default();
        // A 50-char content line wraps into several rows at width 20; the tool
        // card range must be shifted by the wrapped row count.
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "x".repeat(50),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: vec![shell_tool()],
            segments: Vec::new(),
        };

        let (lines, plain, ranges) = render_message_wrapped(&msg, 0, None, false, 0, 20, &theme);

        assert_eq!(ranges.len(), 1);
        let (tool_index, range) = &ranges[0];
        assert_eq!(*tool_index, 0);
        assert!(
            range.start > 2,
            "long content must wrap into multiple rows before the card: {plain:?}"
        );
        // Rows before the card are header + wrapped content rows.
        assert!(
            plain[1..range.start]
                .iter()
                .all(|row| !row.contains("ShellExec")),
            "no content row may leak into the card range: {plain:?}"
        );
        assert!(
            plain[range.start..range.end]
                .concat()
                .contains("Ran cargo test"),
            "the card header must open the range (wrapped rows reassemble): {plain:?}"
        );
        assert_eq!(range.end, lines.len());
    }

    #[test]
    fn test_tool_ranges_cached_hit_matches_rerender() {
        let theme = Theme::default();
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: vec![shell_tool()],
            segments: vec![StreamSegment::ToolCall(0)],
        };

        let mut cache = ChatRenderCache::default();
        let (first_ranges, first_generation) = {
            let entry = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme);
            (entry.tool_ranges.clone(), entry.generation)
        };
        assert!(!first_ranges.is_empty());

        // Cache hit: identical entry, no re-render.
        let hit = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme);
        assert_eq!(hit.generation, first_generation, "must be a cache hit");
        assert_eq!(hit.tool_ranges, first_ranges);

        // Independent re-render in a fresh cache produces identical ranges.
        let mut fresh = ChatRenderCache::default();
        let rerendered = cached_message_render(&mut fresh, 0, &msg, None, true, 0, 80, &theme);
        assert_eq!(rerendered.tool_ranges, first_ranges);
    }

    #[test]
    fn test_render_fills_tool_rows_out_with_absolute_rows() {
        let mut state = AppState::new();
        state.messages.push(user_message("hello".to_string()));
        // User message with a trailing tool card (fallback path): header +
        // one content row + card rows.
        let mut with_tool = user_message("result".to_string());
        with_tool.tool_calls.push(shell_tool());
        state.messages.push(with_tool);

        let backend = ratatui::backend::TestBackend::new(60, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut cache = ChatRenderCache::default();
        let mut plain = Vec::new();
        let mut tool_rows = Vec::new();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &state,
                    &mut cache,
                    &mut plain,
                    &mut tool_rows,
                );
            })
            .unwrap();

        // Absolute rows: msg0 content = row 0, separator = row 1, msg1
        // content = row 2, card = rows 3...
        assert_eq!(tool_rows.len(), 1, "one tool card expected: {tool_rows:?}");
        let (range, selection) = &tool_rows[0];
        assert_eq!(
            *selection,
            ToolSelection {
                message_index: 1,
                tool_index: 0,
            }
        );
        assert_eq!(range.start, 3, "card start in absolute rows: {plain:?}");
        assert_eq!(range.end, plain.len());
        assert!(plain[range.start].contains("Ran cargo test"));

        // A second render (cache hit) clears and refills identical rows.
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &state,
                    &mut cache,
                    &mut plain,
                    &mut tool_rows,
                );
            })
            .unwrap();
        assert_eq!(tool_rows.len(), 1);
        assert_eq!(tool_rows[0].0, 3..plain.len());
    }

    #[test]
    fn test_display_items_streaming_without_messages_has_no_indicator() {
        // The dead StreamingIndicator item was removed: streaming state with
        // an empty transcript yields no display items at all.
        let mut state = AppState::new();
        state.is_streaming = true;
        assert!(build_display_items(&state).is_empty());
    }

    #[test]
    fn test_cached_message_render_streaming_message_revalidates_per_tick() {
        let theme = Theme::default();
        let mut cache = ChatRenderCache::default();
        let mut msg = user_message(String::new());
        msg.is_streaming = true;

        let g1 = cached_message_render(&mut cache, 0, &msg, None, true, 0, 80, &theme).generation;
        let g2 = cached_message_render(&mut cache, 0, &msg, None, true, 1, 80, &theme).generation;
        assert_ne!(g1, g2, "streaming placeholder must re-render each tick");

        let placeholder = &cache.get(0).unwrap().plain[0];
        assert!(
            placeholder.contains(SPINNER_FRAMES[1]),
            "placeholder must show the tick-1 spinner frame: {placeholder}"
        );
    }
}
