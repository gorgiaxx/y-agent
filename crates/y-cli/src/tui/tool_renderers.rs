use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::state::{ToolCallInfo, ToolCallStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Shell,
    Read,
    Write,
    Search,
    List,
    Edit,
    Web,
    Task,
    Generic,
}

/// Semantic color role for one rendered tool line. The chat panel maps tones
/// onto theme colors so this module stays palette-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTone {
    /// Default section color (args color or result color, chosen by caller).
    Plain,
    /// Secondary information: placeholders, stats, diff hunk headers.
    Dim,
    /// Added line in a diff.
    Added,
    /// Removed line in a diff.
    Removed,
    /// Stderr output from a shell call.
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLine {
    pub text: String,
    pub tone: ToolTone,
}

impl ToolLine {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: ToolTone::Plain,
        }
    }

    fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: ToolTone::Dim,
        }
    }

    fn toned(text: impl Into<String>, tone: ToolTone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPresentation {
    pub kind: ToolKind,
    pub verb: &'static str,
    pub summary: String,
    /// Header chips rendered after the summary, ` · `-joined (exit codes,
    /// diff stats, match counts, ...).
    pub meta: Vec<String>,
    pub argument_lines: Vec<ToolLine>,
    pub result_lines: Vec<ToolLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRenderGroup {
    Single(usize),
    Exploration(Vec<usize>),
}

pub fn group_tool_indexes(tools: &[ToolCallInfo], indexes: &[usize]) -> Vec<ToolRenderGroup> {
    let mut groups = Vec::new();
    let mut exploration = Vec::new();

    for &tool_index in indexes {
        let is_exploration = tools.get(tool_index).is_some_and(|tool| {
            tool.status == ToolCallStatus::Succeeded
                && matches!(
                    classify_tool(&tool.name),
                    ToolKind::Read | ToolKind::Search | ToolKind::List
                )
        });
        if is_exploration {
            exploration.push(tool_index);
        } else {
            flush_exploration_group(&mut groups, &mut exploration);
            groups.push(ToolRenderGroup::Single(tool_index));
        }
    }
    flush_exploration_group(&mut groups, &mut exploration);
    groups
}

fn flush_exploration_group(groups: &mut Vec<ToolRenderGroup>, exploration: &mut Vec<usize>) {
    match exploration.len() {
        0 => {}
        1 => groups.push(ToolRenderGroup::Single(exploration[0])),
        _ => groups.push(ToolRenderGroup::Exploration(std::mem::take(exploration))),
    }
    exploration.clear();
}

pub fn present_tool(tool: &ToolCallInfo, width: usize) -> ToolPresentation {
    let kind = classify_tool(&tool.name);
    // Parse each payload once and share the value between summary extraction
    // and line preview instead of re-parsing per use.
    let input = parse_json(&tool.input_preview);
    let result = parse_json(&tool.result_preview);
    let summary = tool_summary(kind, input.as_ref());
    let meta = tool_meta(kind, input.as_ref(), result.as_ref());
    let argument_lines = argument_preview_lines(kind, &tool.input_preview, input.as_ref(), width);
    let mut result_lines = result_preview_lines(kind, &tool.result_preview, result.as_ref(), width);
    if result_lines.is_empty() && tool.status != ToolCallStatus::Running {
        result_lines.push(ToolLine::dim("(no output)"));
    }

    ToolPresentation {
        kind,
        verb: tool_verb(kind, &tool.name),
        summary,
        meta,
        argument_lines,
        result_lines,
    }
}

/// Cheap one-line input summary (e.g. `src/lib.rs`) for collapsed or
/// unselected tool rows. Parses only the input payload — skips result
/// parsing, preview lines, and wrapping, unlike [`present_tool`].
pub fn quick_summary(tool: &ToolCallInfo) -> String {
    tool_summary(
        classify_tool(&tool.name),
        parse_json(&tool.input_preview).as_ref(),
    )
}

/// Canonical tool names get an exact, case-insensitive classification first
/// (mirroring the GUI's `KNOWN_TOOL_NAMES` contract); unknown or MCP tool
/// names fall back to substring heuristics.
fn classify_tool(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "shellexec" => return ToolKind::Shell,
        "fileread" => return ToolKind::Read,
        "filewrite" => return ToolKind::Write,
        "fileedit" => return ToolKind::Edit,
        "grep" => return ToolKind::Search,
        "glob" => return ToolKind::List,
        "webfetch" | "browser" => return ToolKind::Web,
        "task" => return ToolKind::Task,
        _ => {}
    }
    let name = name.to_ascii_lowercase();
    if contains_any(&name, &["edit", "write", "patch", "replace"]) {
        ToolKind::Edit
    } else if contains_any(&name, &["shell", "exec", "bash", "command", "terminal"]) {
        ToolKind::Shell
    } else if contains_any(&name, &["read", "open_file", "file_get"]) {
        ToolKind::Read
    } else if contains_any(&name, &["web", "browser", "fetch", "http", "url"]) {
        ToolKind::Web
    } else if contains_any(&name, &["search", "grep", "find", "query"]) {
        ToolKind::Search
    } else if contains_any(&name, &["list", "glob", "directory", "tree"]) {
        ToolKind::List
    } else if contains_any(&name, &["agent", "task", "delegate", "workflow"]) {
        ToolKind::Task
    } else {
        ToolKind::Generic
    }
}

fn tool_verb(kind: ToolKind, name: &str) -> &'static str {
    match kind {
        ToolKind::Shell => "Ran",
        ToolKind::Read => "Read",
        ToolKind::Write => "Wrote",
        ToolKind::Search => "Searched",
        ToolKind::List => "Listed",
        ToolKind::Edit => "Edited",
        ToolKind::Web if name.to_ascii_lowercase().contains("search") => "Searched web",
        ToolKind::Web => "Fetched",
        ToolKind::Task => "Delegated",
        ToolKind::Generic => "Called",
    }
}

fn tool_summary(kind: ToolKind, input: Option<&Value>) -> String {
    match kind {
        ToolKind::Read => return read_summary(input),
        ToolKind::Generic => return input.map_or_else(String::new, inline_args_summary),
        _ => {}
    }
    let keys: &[&str] = match kind {
        ToolKind::Shell => &["command", "cmd", "script"],
        ToolKind::Write | ToolKind::Edit | ToolKind::List => {
            &["path", "file_path", "file", "directory"]
        }
        ToolKind::Search => &["query", "pattern", "search", "needle"],
        ToolKind::Web => &["url", "query", "href"],
        ToolKind::Task => &["description", "prompt", "task", "agent_name"],
        ToolKind::Read | ToolKind::Generic => unreachable!(),
    };
    input
        .and_then(|value| first_string(value, keys))
        .unwrap_or_default()
}

/// `path` plus a `:start-end` suffix when the read requested a line range.
fn read_summary(input: Option<&Value>) -> String {
    let Some(value) = input else {
        return String::new();
    };
    let path = first_string(value, &["path", "file_path", "file"]).unwrap_or_default();
    let object = value.as_object();
    let offset = object
        .and_then(|obj| obj.get("line_offset").or_else(|| obj.get("offset")))
        .and_then(Value::as_u64);
    let limit = object
        .and_then(|obj| obj.get("limit"))
        .and_then(Value::as_u64);
    match (offset, limit) {
        (Some(start), Some(count)) if start > 0 => {
            format!("{path}:{}-{}", start + 1, start + count)
        }
        (Some(start), None) if start > 0 => format!("{path}:{}-", start + 1),
        _ => path,
    }
}

/// Compact `key=value` one-liner for tools without a dedicated summary field,
/// so collapsed rows never fall back to raw JSON.
fn inline_args_summary(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return String::new();
    };
    object
        .iter()
        .map(|(key, value)| format!("{key}={}", inline_value(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn inline_value(value: &Value) -> String {
    let rendered = match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => format!("[{} items]", items.len()),
        Value::Object(map) => format!("{{{} keys}}", map.len()),
    };
    truncate_chars(&rendered, 24)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Header chips: tool-specific stats shown next to the summary.
fn tool_meta(kind: ToolKind, input: Option<&Value>, result: Option<&Value>) -> Vec<String> {
    let mut meta = Vec::new();
    match kind {
        ToolKind::Shell => {
            if let Some(code) = result
                .and_then(|value| value.get("exit_code"))
                .and_then(Value::as_i64)
            {
                if code != 0 {
                    meta.push(format!("exit {code}"));
                }
            }
        }
        ToolKind::Write => {
            if let Some(content) = input.and_then(|value| field_text(value, &["content"])) {
                meta.push(format!("{} lines", content.lines().count()));
            }
        }
        ToolKind::Edit => {
            let (removed, added) = edit_line_counts(input);
            if removed + added > 0 {
                meta.push(format!("+{added} -{removed}"));
            }
        }
        ToolKind::Search => meta.extend(grep_meta(result)),
        ToolKind::List => meta.extend(glob_meta(result)),
        _ => {}
    }
    meta
}

fn edit_line_counts(input: Option<&Value>) -> (usize, usize) {
    let Some(value) = input else {
        return (0, 0);
    };
    let removed =
        field_text(value, &["old_string", "old", "search"]).map_or(0, |text| text.lines().count());
    let added =
        field_text(value, &["new_string", "new", "replace"]).map_or(0, |text| text.lines().count());
    (removed, added)
}

/// Stats from the structured `Grep` result contract.
fn grep_meta(result: Option<&Value>) -> Vec<String> {
    let Some(object) = result.and_then(Value::as_object) else {
        return Vec::new();
    };
    if !object.contains_key("mode") {
        return Vec::new();
    }
    let files = object.get("numFiles").and_then(Value::as_u64);
    let mut meta = Vec::new();
    match object.get("mode").and_then(Value::as_str) {
        Some("count") => {
            if let Some(matches) = object.get("numMatches").and_then(Value::as_u64) {
                meta.push(format!("{matches} matches"));
            }
        }
        Some("content") => {
            if let Some(lines) = object.get("numLines").and_then(Value::as_u64) {
                meta.push(format!("{lines} lines"));
            }
        }
        _ => {}
    }
    if let Some(files) = files {
        meta.push(format!("{files} files"));
    }
    if object.get("truncated").and_then(Value::as_bool) == Some(true) {
        meta.push("truncated".to_string());
    }
    meta
}

/// Stats from the structured `Glob` result contract.
fn glob_meta(result: Option<&Value>) -> Vec<String> {
    let Some(object) = result.and_then(Value::as_object) else {
        return Vec::new();
    };
    if !object.contains_key("matches") {
        return Vec::new();
    }
    let mut meta = Vec::new();
    if let Some(count) = object.get("count").and_then(Value::as_u64) {
        meta.push(format!("{count} files"));
    }
    if object.get("truncated").and_then(Value::as_bool) == Some(true) {
        meta.push("truncated".to_string());
    }
    meta
}

/// Argument preview lines: extract the meaningful field(s) per tool kind,
/// falling back to the generic JSON preview when nothing obvious matches.
fn argument_preview_lines(
    kind: ToolKind,
    raw: &str,
    parsed: Option<&Value>,
    width: usize,
) -> Vec<ToolLine> {
    if let Some(value) = parsed {
        let extracted = match kind {
            ToolKind::Shell => shell_argument_lines(value, width),
            ToolKind::Read => text_field_lines(value, &["path", "file_path", "file"], width),
            ToolKind::Write => write_argument_lines(value, width),
            ToolKind::Edit => edit_argument_lines(value, width),
            ToolKind::Web => text_field_lines(value, &["url", "query"], width),
            ToolKind::Search => text_field_lines(value, &["query", "pattern"], width),
            ToolKind::List => text_field_lines(value, &["path", "directory"], width),
            ToolKind::Task => text_field_lines(value, &["description", "prompt"], width),
            ToolKind::Generic => None,
        };
        if let Some(lines) = extracted {
            return lines;
        }
    }
    preview_lines(raw, parsed, width)
}

/// Shell arguments render as the operator typed them: `$ command`.
fn shell_argument_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let command = field_text(value, &["command", "cmd", "script"])?;
    let inner_width = width.saturating_sub(2);
    let lines = text_lines(&command, inner_width)
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                ToolLine::plain(format!("$ {line}"))
            } else {
                ToolLine::plain(format!("  {line}"))
            }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Write args show the target path, then the written content itself.
fn write_argument_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let path = first_string(value, &["path", "file_path", "file"]);
    let content = field_text(value, &["content"]);
    let mut lines = Vec::new();
    if let Some(path) = path {
        lines.extend(text_lines(&path, width).into_iter().map(ToolLine::plain));
    }
    if let Some(content) = content {
        lines.extend(text_lines(&content, width).into_iter().map(ToolLine::plain));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Edit args show the target path, then a synthesized diff: removed lines in
/// red, added lines in green. Tools that pass a ready-made `diff` field get
/// the same coloring via [`toned_diff_lines`].
fn edit_argument_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let path = first_string(value, &["path", "file_path", "file"]);
    let old = field_text(value, &["old_string", "old", "search"]);
    let new = field_text(value, &["new_string", "new", "replace"]);
    let diff = field_text(value, &["diff"]);
    let mut lines = Vec::new();
    if let Some(path) = path {
        lines.extend(text_lines(&path, width).into_iter().map(ToolLine::plain));
    }
    if old.is_none() && new.is_none() {
        if let Some(diff) = diff {
            lines.extend(toned_diff_lines(&diff, width));
        }
    } else {
        if let Some(old) = old {
            for line in text_lines(&old, width.saturating_sub(2)) {
                lines.push(ToolLine::toned(format!("- {line}"), ToolTone::Removed));
            }
        }
        if let Some(new) = new {
            for line in text_lines(&new, width.saturating_sub(2)) {
                lines.push(ToolLine::toned(format!("+ {line}"), ToolTone::Added));
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Result preview lines: per-kind extraction first, then the generic
/// envelope-aware JSON preview.
fn result_preview_lines(
    kind: ToolKind,
    raw: &str,
    parsed: Option<&Value>,
    width: usize,
) -> Vec<ToolLine> {
    if let Some(value) = parsed {
        let extracted = match kind {
            ToolKind::Shell => shell_result_lines(value, width),
            ToolKind::Read => text_field_lines(value, &["content"], width),
            ToolKind::Write => write_result_lines(value),
            ToolKind::Edit => edit_result_lines(value, width),
            ToolKind::Web => text_field_lines(value, &["content", "text"], width),
            ToolKind::Search => grep_result_lines(value, width),
            ToolKind::List => glob_result_lines(value, width),
            _ => None,
        };
        if let Some(lines) = extracted {
            return lines;
        }
    }
    // A raw (non-JSON) diff payload still deserves colored +/- lines.
    if kind == ToolKind::Edit && looks_like_diff(raw) {
        return toned_diff_lines(raw, width);
    }
    preview_lines(raw, parsed, width)
}

/// Shell results surface stdout first, then stderr on warning-toned lines.
/// When both streams are empty the shell shape is still honored with a
/// concise placeholder instead of falling back to a raw JSON blob. Returns
/// `None` only when the payload has neither `stdout` nor `stderr`, so
/// callers fall back to the generic preview.
fn shell_result_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let object = value.as_object()?;
    if !object.contains_key("stdout") && !object.contains_key("stderr") {
        return None;
    }
    let mut lines = Vec::new();
    if let Some(stdout) = object.get("stdout") {
        lines.extend(
            text_lines(&display_text(stdout), width)
                .into_iter()
                .map(ToolLine::plain),
        );
    }
    if let Some(stderr) = object.get("stderr") {
        let text = display_text(stderr);
        if !text.trim().is_empty() {
            lines.extend(
                text_lines(&text, width)
                    .into_iter()
                    .map(|line| ToolLine::toned(line, ToolTone::Stderr)),
            );
        }
    }
    if lines.is_empty() {
        let exit = object.get("exit_code").and_then(Value::as_i64);
        let placeholder = match exit {
            Some(code) if code != 0 => format!("(exit {code})"),
            _ => "(no output)".to_string(),
        };
        Some(vec![ToolLine::dim(placeholder)])
    } else {
        Some(lines)
    }
}

/// Write results compress to a single dim stat line (`bytes_written`).
fn write_result_lines(value: &Value) -> Option<Vec<ToolLine>> {
    let object = value.as_object()?;
    let bytes = object.get("bytes_written").and_then(Value::as_u64)?;
    Some(vec![ToolLine::dim(format!("{bytes} bytes written"))])
}

/// Edit results: a returned unified diff renders with colored +/- lines; the
/// structured replacement report compresses to a dim stat line.
fn edit_result_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let object = value.as_object()?;
    if let Some(diff) = field_text(value, &["diff", "content"]) {
        if looks_like_diff(&diff) {
            return Some(toned_diff_lines(&diff, width));
        }
    }
    let replacements = object.get("replacements").and_then(Value::as_u64)?;
    let label = if replacements == 1 {
        "1 replacement".to_string()
    } else {
        format!("{replacements} replacements")
    };
    Some(vec![ToolLine::dim(label)])
}

/// Grep results: match lines (content/count modes) or the filename list
/// (`files_with_matches` mode) as plain text; other search tools fall back.
fn grep_result_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let object = value.as_object()?;
    match object.get("mode").and_then(Value::as_str) {
        Some("files_with_matches") => {
            let filenames = object.get("filenames")?.as_array()?;
            let lines = filenames
                .iter()
                .filter_map(Value::as_str)
                .flat_map(|name| text_lines(name, width))
                .map(ToolLine::plain)
                .collect::<Vec<_>>();
            Some(lines)
        }
        Some("content" | "count") => {
            let content = object.get("content").and_then(Value::as_str)?;
            Some(
                text_lines(content, width)
                    .into_iter()
                    .map(ToolLine::plain)
                    .collect(),
            )
        }
        _ => None,
    }
}

/// Glob results render the match list; other list tools fall back.
fn glob_result_lines(value: &Value, width: usize) -> Option<Vec<ToolLine>> {
    let matches = value.get("matches")?.as_array()?;
    let lines = matches
        .iter()
        .filter_map(Value::as_str)
        .flat_map(|name| text_lines(name, width))
        .map(ToolLine::plain)
        .collect::<Vec<_>>();
    Some(lines)
}

/// Display lines for the first matching key of an object, or `None` when no
/// key matches or the extracted text renders to nothing.
fn text_field_lines(value: &Value, keys: &[&str], width: usize) -> Option<Vec<ToolLine>> {
    let lines = text_lines(&field_text(value, keys)?, width);
    if lines.is_empty() {
        None
    } else {
        Some(lines.into_iter().map(ToolLine::plain).collect())
    }
}

/// Extract the first matching key's display text from an object.
fn field_text(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).map(display_text))
}

/// The first matching key as a display string (strings verbatim, other
/// scalars stringified).
fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object.get(*key).map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    })
}

/// Render a JSON value as display text: strings verbatim (so embedded
/// newlines render as real lines), anything else as pretty JSON.
fn display_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => pretty_json(other),
    }
}

/// The service normalizes tool output into single-key `{"result": ...}` or
/// `{"error": ...}` envelopes. Unwrap them so the payload renders as real
/// multi-line text instead of an escaped JSON blob.
fn unwrap_envelope(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    ["result", "error"].iter().find_map(|key| object.get(*key))
}

/// A JSON object whose values are all scalars renders as `key: value` lines
/// instead of a pretty-printed JSON blob.
fn flat_object_lines(object: &serde_json::Map<String, Value>, width: usize) -> Option<Vec<String>> {
    if object.is_empty()
        || object
            .values()
            .any(|value| matches!(value, Value::Object(_) | Value::Array(_)))
    {
        return None;
    }
    let mut lines = Vec::new();
    for (key, value) in object {
        let rendered = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        lines.extend(text_lines(&format!("{key}: {rendered}"), width));
    }
    Some(lines)
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Heuristic: text carries unified-diff markers worth coloring.
fn looks_like_diff(text: &str) -> bool {
    let mut added = false;
    let mut removed = false;
    for line in text.lines() {
        added |= line.starts_with("+ ") || line.starts_with('+') && !line.starts_with("++");
        removed |= line.starts_with("- ") || line.starts_with('-') && !line.starts_with("--");
        if line.starts_with("@@") {
            return true;
        }
    }
    added && removed
}

/// Tone diff lines by marker: `+` added, `-` removed, `@@` hunk headers dim.
fn toned_diff_lines(text: &str, width: usize) -> Vec<ToolLine> {
    let mut lines = Vec::new();
    for line in sanitize_terminal_text(text).lines() {
        let tone = if line.starts_with("@@") {
            ToolTone::Dim
        } else if line.starts_with('+') {
            ToolTone::Added
        } else if line.starts_with('-') {
            ToolTone::Removed
        } else {
            ToolTone::Plain
        };
        for wrapped in wrap_line(line, width.max(20)) {
            lines.push(ToolLine::toned(wrapped, tone));
        }
    }
    lines
}

/// Split display text into lines: sanitize terminal escapes and control
/// characters (shell output frequently carries color codes and hyperlink
/// sequences whose bytes would corrupt width math and rendering), then wrap
/// each line to the display width.
fn text_lines(text: &str, width: usize) -> Vec<String> {
    sanitize_terminal_text(text)
        .lines()
        .flat_map(|line| wrap_line(line, width.max(20)))
        .collect()
}

fn preview_lines(raw: &str, parsed: Option<&Value>, width: usize) -> Vec<ToolLine> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let lines = match parsed {
        Some(Value::String(text)) => text_lines(text, width),
        Some(other) => render_value_lines(unwrap_envelope(other).unwrap_or(other), width),
        None => text_lines(raw.trim(), width),
    };
    lines.into_iter().map(ToolLine::plain).collect()
}

/// Render a JSON payload as display lines: strings verbatim, flat objects as
/// `key: value` lines, anything else as pretty JSON.
fn render_value_lines(value: &Value, width: usize) -> Vec<String> {
    match value {
        Value::String(text) => text_lines(text, width),
        other => other
            .as_object()
            .and_then(|object| flat_object_lines(object, width))
            .unwrap_or_else(|| text_lines(&pretty_json(other), width)),
    }
}

fn parse_json(raw: &str) -> Option<Value> {
    serde_json::from_str(raw.trim()).ok()
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

/// Sanitize untrusted terminal text (tool output) before it enters the chat
/// buffer: strip every ANSI escape-sequence class — CSI (`ESC [`), OSC
/// (`ESC ]` … BEL/ST), DCS/SOS/PM/APC (`ESC P X ^ _` … ST), and nF/Fe
/// sequences — replace tabs with spaces, and drop remaining control
/// characters (except `\n`, which splits lines). OSC payloads (hyperlinks,
/// window titles) otherwise print as literal `]8;;https://...` text and
/// stray control bytes corrupt ratatui's width accounting, both of which
/// show up as leftover characters on screen.
fn sanitize_terminal_text(value: &str) -> String {
    if !value.contains('\u{1b}') && !value.chars().any(|ch| ch.is_control() && ch != '\n') {
        return value.to_string();
    }
    let mut sanitized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek() {
                // CSI: params/intermediates up to and including the final byte.
                Some('[') => {
                    chars.next();
                    for seq_ch in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&seq_ch) {
                            break;
                        }
                    }
                }
                // OSC / DCS / SOS / PM / APC payloads, terminated by BEL or ST.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    chars.next();
                    skip_escape_payload(&mut chars);
                }
                // nF sequence: intermediates (0x20-0x2F) then one final byte.
                Some('\u{20}'..='\u{2f}') => {
                    while matches!(chars.peek(), Some('\u{20}'..='\u{2f}')) {
                        chars.next();
                    }
                    if matches!(chars.peek(), Some('\u{30}'..='\u{7e}')) {
                        chars.next();
                    }
                }
                // Fe sequence (ESC + 0x40-0x5F, e.g. `ESC c`); a lone ESC
                // before anything else is simply dropped.
                Some('\u{40}'..='\u{5f}') => {
                    chars.next();
                }
                _ => {}
            },
            '\t' => sanitized.push_str("    "),
            '\n' => sanitized.push('\n'),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

/// Skip an escape payload up to and including its terminator: BEL, or ST
/// (`ESC \`). Unterminated payloads consume the rest of the input.
fn skip_escape_payload(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(seq_ch) = chars.next() {
        if seq_ch == '\u{07}' {
            break;
        }
        if seq_ch == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}

// TODO: unify with the chat panel's span-aware `wrap_spans` once the chat
// panel rewrite settles.
/// Wrap a line to at most `width` display columns, keeping wide (CJK/emoji)
/// chars whole instead of counting bytes or chars.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if UnicodeWidthStr::width(line) <= width {
        return vec![line.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for ch in line.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + ch_width > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::{ToolCallDisplayMode, ToolCallInfo, ToolCallStatus};

    fn tool(name: &str, input: &str, result: &str) -> ToolCallInfo {
        ToolCallInfo {
            tool_call_id: format!("call-{name}"),
            name: name.into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(12),
            input_preview: input.into(),
            result_preview: result.into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        }
    }

    fn texts(lines: &[ToolLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn test_shell_renderer_extracts_command_summary() {
        let presentation = present_tool(
            &tool("ShellExec", r#"{"command":"cargo test -p y-cli"}"#, "ok"),
            80,
        );

        assert_eq!(presentation.kind, ToolKind::Shell);
        assert_eq!(presentation.verb, "Ran");
        assert_eq!(presentation.summary, "cargo test -p y-cli");
    }

    #[test]
    fn test_edit_renderer_extracts_path_and_preserves_diff() {
        let presentation = present_tool(
            &tool(
                "FileEdit",
                r#"{"path":"src/main.rs"}"#,
                "@@ -1 +1 @@\n-old\n+new",
            ),
            80,
        );

        assert_eq!(presentation.kind, ToolKind::Edit);
        assert_eq!(presentation.verb, "Edited");
        assert_eq!(presentation.summary, "src/main.rs");
        let added = presentation
            .result_lines
            .iter()
            .find(|line| line.text == "+new")
            .expect("diff line present");
        assert_eq!(added.tone, ToolTone::Added);
    }

    #[test]
    fn test_unknown_tool_uses_generic_renderer() {
        let presentation = present_tool(&tool("CustomTool", r#"{"value":42}"#, "done"), 80);

        assert_eq!(presentation.kind, ToolKind::Generic);
        assert_eq!(presentation.verb, "Called");
        assert!(presentation
            .argument_lines
            .iter()
            .any(|line| line.text.contains("value")));
    }

    #[test]
    fn test_generic_summary_is_inline_key_value_not_json() {
        let presentation = present_tool(
            &tool(
                "CustomTool",
                r#"{"path":"src/main.rs","count":3,"verbose":true}"#,
                "done",
            ),
            80,
        );

        assert_eq!(
            presentation.summary,
            "count=3, path=src/main.rs, verbose=true"
        );
        assert!(!presentation.summary.contains('{'));
    }

    #[test]
    fn test_group_tool_indexes_groups_only_consecutive_successful_exploration_calls() {
        let mut failed_read = tool("FileRead", r#"{"path":"failed.rs"}"#, "denied");
        failed_read.status = ToolCallStatus::Failed;
        let tools = vec![
            tool("FileRead", r#"{"path":"src/lib.rs"}"#, "lib"),
            tool("FileSearch", r#"{"query":"ToolCallInfo"}"#, "matches"),
            tool("FileEdit", r#"{"path":"src/lib.rs"}"#, "updated"),
            tool("DirectoryList", r#"{"path":"src"}"#, "files"),
            tool("FileRead", r#"{"path":"src/main.rs"}"#, "main"),
            failed_read,
        ];

        let groups = group_tool_indexes(&tools, &[0, 1, 2, 3, 4, 5]);

        assert_eq!(
            groups,
            vec![
                ToolRenderGroup::Exploration(vec![0, 1]),
                ToolRenderGroup::Single(2),
                ToolRenderGroup::Exploration(vec![3, 4]),
                ToolRenderGroup::Single(5),
            ]
        );
    }

    #[test]
    fn test_wrap_line_wraps_cjk_by_display_width() {
        // 10 CJK chars = 20 display cells; width 10 fits 5 chars per line.
        let line = "你好世界再见朋友哥们";
        let wrapped = wrap_line(line, 10);
        assert_eq!(wrapped.len(), 2);
        assert_eq!(wrapped[0], "你好世界再");
        assert_eq!(wrapped[1], "见朋友哥们");
        for wrapped_line in &wrapped {
            assert!(UnicodeWidthStr::width(wrapped_line.as_str()) <= 10);
        }
    }

    #[test]
    fn test_wrap_line_keeps_short_multibyte_line_whole() {
        // 6 CJK chars = 12 cells but 18 bytes; must not be split at width 12.
        let line = "你好世界再见";
        assert_eq!(wrap_line(line, 12), vec![line.to_string()]);
    }

    #[test]
    fn test_preview_lines_strips_ansi_from_plain_output() {
        let lines = preview_lines("\u{1b}[31merror: failed\u{1b}[0m", None, 80);
        assert_eq!(texts(&lines), vec!["error: failed".to_string()]);
    }

    #[test]
    fn test_preview_lines_strips_ansi_from_json_string_output() {
        let raw = "\"\\u001b[32mok\\u001b[0m done\"";
        let parsed = parse_json(raw);
        let lines = preview_lines(raw, parsed.as_ref(), 80);
        assert_eq!(texts(&lines), vec!["ok done".to_string()]);
    }

    #[test]
    fn test_result_envelope_unwraps_result_string_into_multiline() {
        let presentation = present_tool(
            &tool("CustomTool", "{}", r#"{"result":"line one\nline two"}"#),
            80,
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["line one".to_string(), "line two".to_string()]
        );
    }

    #[test]
    fn test_result_envelope_unwraps_error_string_into_multiline() {
        let presentation = present_tool(
            &tool("CustomTool", "{}", r#"{"error":"boom\nstack trace"}"#),
            80,
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["boom".to_string(), "stack trace".to_string()]
        );
    }

    #[test]
    fn test_result_envelope_flat_object_renders_key_value_not_json() {
        let presentation = present_tool(&tool("CustomTool", "{}", r#"{"result":{"code":1}}"#), 80);
        let joined = presentation
            .result_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("code: 1"), "got: {joined}");
        assert!(!joined.contains("\"code\""), "raw JSON leaked: {joined}");
    }

    #[test]
    fn test_multi_key_flat_object_renders_key_value_lines() {
        let presentation =
            present_tool(&tool("CustomTool", "{}", r#"{"result":"x","extra":1}"#), 80);
        let joined = presentation
            .result_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("result: x"), "got: {joined}");
        assert!(joined.contains("extra: 1"), "got: {joined}");
        assert!(!joined.contains("\"result\""), "raw JSON leaked: {joined}");
    }

    #[test]
    fn test_unwrapped_long_lines_still_wrap_by_display_width() {
        let long = "ab".repeat(30); // 60 display cells on one line
        let raw = format!(r#"{{"result":"{long}"}}"#);
        let presentation = present_tool(&tool("CustomTool", "{}", &raw), 20);
        assert!(presentation.result_lines.len() >= 3);
        for line in &presentation.result_lines {
            assert!(UnicodeWidthStr::width(line.text.as_str()) <= 20);
        }
    }

    #[test]
    fn test_shell_arguments_show_dollar_prefixed_command() {
        let presentation = present_tool(
            &tool("ShellExec", r#"{"command":"cargo test","timeout":30}"#, ""),
            80,
        );
        assert_eq!(texts(&presentation.argument_lines), vec!["$ cargo test"]);
    }

    #[test]
    fn test_shell_arguments_fall_back_to_cmd_key() {
        let presentation = present_tool(&tool("ShellExec", r#"{"cmd":"ls -la"}"#, ""), 80);
        assert_eq!(texts(&presentation.argument_lines), vec!["$ ls -la"]);
    }

    #[test]
    fn test_shell_result_splits_stdout_and_tones_stderr() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"make"}"#,
                r#"{"stdout":"out one\nout two","stderr":"warn: slow"}"#,
            ),
            80,
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec![
                "out one".to_string(),
                "out two".to_string(),
                "warn: slow".to_string()
            ]
        );
        assert_eq!(presentation.result_lines[0].tone, ToolTone::Plain);
        assert_eq!(presentation.result_lines[2].tone, ToolTone::Stderr);
    }

    #[test]
    fn test_shell_result_skips_empty_stderr() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"ls"}"#,
                r#"{"stdout":"file.rs","stderr":""}"#,
            ),
            80,
        );
        assert_eq!(texts(&presentation.result_lines), vec!["file.rs"]);
    }

    #[test]
    fn test_shell_result_pretty_prints_non_string_stdout() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"ls"}"#,
                r#"{"stdout":{"files":2}}"#,
            ),
            80,
        );
        let joined = presentation
            .result_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("\"files\": 2"));
    }

    #[test]
    fn test_shell_result_empty_streams_show_placeholder_not_json() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"git add ."}"#,
                r#"{"exit_code":0,"stderr":"","stdout":""}"#,
            ),
            80,
        );
        assert_eq!(texts(&presentation.result_lines), vec!["(no output)"]);
    }

    #[test]
    fn test_shell_result_nonzero_exit_shows_code_in_meta_and_placeholder() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"false"}"#,
                r#"{"exit_code":2,"stderr":"","stdout":""}"#,
            ),
            80,
        );
        assert_eq!(texts(&presentation.result_lines), vec!["(exit 2)"]);
        assert_eq!(presentation.meta, vec!["exit 2".to_string()]);
    }

    #[test]
    fn test_shell_result_clean_exit_has_no_exit_meta() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"ls"}"#,
                r#"{"exit_code":0,"stdout":"ok"}"#,
            ),
            80,
        );
        assert!(presentation.meta.is_empty());
    }

    #[test]
    fn test_shell_result_envelope_still_unwraps() {
        let presentation = present_tool(
            &tool("ShellExec", r#"{"command":"ls"}"#, r#"{"result":"a\nb"}"#),
            80,
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn test_read_arguments_show_path() {
        let presentation = present_tool(&tool("FileRead", r#"{"path":"src/lib.rs"}"#, ""), 80);
        assert_eq!(texts(&presentation.argument_lines), vec!["src/lib.rs"]);
    }

    #[test]
    fn test_read_summary_includes_line_range() {
        let ranged = present_tool(
            &tool(
                "FileRead",
                r#"{"path":"src/lib.rs","line_offset":40,"limit":20}"#,
                "",
            ),
            80,
        );
        assert_eq!(ranged.summary, "src/lib.rs:41-60");
        let plain = present_tool(&tool("FileRead", r#"{"path":"src/lib.rs"}"#, ""), 80);
        assert_eq!(plain.summary, "src/lib.rs");
    }

    #[test]
    fn test_write_shows_path_content_and_line_count() {
        let presentation = present_tool(
            &tool(
                "FileWrite",
                r#"{"path":"a.rs","content":"one\ntwo\nthree"}"#,
                r#"{"path":"a.rs","bytes_written":13}"#,
            ),
            80,
        );
        assert_eq!(presentation.kind, ToolKind::Write);
        assert_eq!(presentation.verb, "Wrote");
        assert_eq!(presentation.summary, "a.rs");
        assert_eq!(presentation.meta, vec!["3 lines".to_string()]);
        assert_eq!(
            texts(&presentation.argument_lines),
            vec!["a.rs", "one", "two", "three"]
        );
        assert_eq!(texts(&presentation.result_lines), vec!["13 bytes written"]);
    }

    #[test]
    fn test_edit_arguments_render_synthesized_diff_tones() {
        let presentation = present_tool(
            &tool(
                "FileEdit",
                r#"{"path":"a.rs","old_string":"old","new_string":"new\ntext"}"#,
                "",
            ),
            80,
        );
        assert_eq!(
            texts(&presentation.argument_lines),
            vec!["a.rs", "- old", "+ new", "+ text"]
        );
        assert_eq!(presentation.argument_lines[1].tone, ToolTone::Removed);
        assert_eq!(presentation.argument_lines[2].tone, ToolTone::Added);
        assert_eq!(presentation.meta, vec!["+2 -1".to_string()]);
    }

    #[test]
    fn test_edit_result_reports_replacement_count() {
        let presentation = present_tool(
            &tool(
                "FileEdit",
                r#"{"path":"a.rs","old_string":"o","new_string":"n"}"#,
                r#"{"file_path":"a.rs","action":"edited","replacements":3}"#,
            ),
            80,
        );
        assert_eq!(texts(&presentation.result_lines), vec!["3 replacements"]);
    }

    #[test]
    fn test_read_result_unwraps_content_string() {
        let presentation = present_tool(
            &tool(
                "FileRead",
                r#"{"path":"a.rs"}"#,
                r#"{"content":"line1\nline2"}"#,
            ),
            80,
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["line1".to_string(), "line2".to_string()]
        );
    }

    #[test]
    fn test_grep_content_mode_meta_and_body() {
        let presentation = present_tool(
            &tool(
                "Grep",
                r#"{"pattern":"foo"}"#,
                r#"{"mode":"content","numFiles":2,"numLines":3,"content":"a.rs:1:foo\nb.rs:2:foo"}"#,
            ),
            80,
        );
        assert_eq!(
            presentation.meta,
            vec!["3 lines".to_string(), "2 files".to_string()]
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["a.rs:1:foo", "b.rs:2:foo"]
        );
    }

    #[test]
    fn test_grep_count_mode_meta() {
        let presentation = present_tool(
            &tool(
                "Grep",
                r#"{"pattern":"foo","output_mode":"count"}"#,
                r#"{"mode":"count","numFiles":2,"numMatches":7,"content":"a.rs:5\nb.rs:2"}"#,
            ),
            80,
        );
        assert_eq!(
            presentation.meta,
            vec!["7 matches".to_string(), "2 files".to_string()]
        );
    }

    #[test]
    fn test_grep_files_with_matches_lists_filenames() {
        let presentation = present_tool(
            &tool(
                "Grep",
                r#"{"pattern":"foo","output_mode":"files_with_matches"}"#,
                r#"{"mode":"files_with_matches","numFiles":2,"filenames":["a.rs","b.rs"]}"#,
            ),
            80,
        );
        assert_eq!(presentation.meta, vec!["2 files".to_string()]);
        assert_eq!(texts(&presentation.result_lines), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn test_glob_meta_and_file_list() {
        let presentation = present_tool(
            &tool(
                "Glob",
                r#"{"pattern":"src/**/*.rs"}"#,
                r#"{"matches":["src/a.rs","src/b.rs"],"count":2,"returned_count":2,"truncated":false}"#,
            ),
            80,
        );
        assert_eq!(presentation.meta, vec!["2 files".to_string()]);
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["src/a.rs", "src/b.rs"]
        );
    }

    #[test]
    fn test_web_arguments_and_result_extract_text_fields() {
        let presentation = present_tool(
            &tool(
                "WebFetch",
                r#"{"url":"https://example.com"}"#,
                r#"{"text":"hello\nworld"}"#,
            ),
            80,
        );
        assert_eq!(
            texts(&presentation.argument_lines),
            vec!["https://example.com".to_string()]
        );
        assert_eq!(
            texts(&presentation.result_lines),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn test_search_list_task_arguments_extract_primary_field() {
        let search = present_tool(&tool("Grep", r#"{"pattern":"foo","path":"src"}"#, ""), 80);
        assert_eq!(texts(&search.argument_lines), vec!["foo".to_string()]);
        let list = present_tool(&tool("DirectoryList", r#"{"directory":"src"}"#, ""), 80);
        assert_eq!(texts(&list.argument_lines), vec!["src".to_string()]);
        let task = present_tool(&tool("Task", r#"{"prompt":"do stuff"}"#, ""), 80);
        assert_eq!(texts(&task.argument_lines), vec!["do stuff".to_string()]);
    }

    #[test]
    fn test_exact_classification_wins_over_substring() {
        // Exact canonical names classify deterministically; other names keep
        // the fuzzy substring fallback ("WebSearch" -> Web via "web").
        assert_eq!(classify_tool("WebSearch"), ToolKind::Web);
        assert_eq!(classify_tool("ShellExec"), ToolKind::Shell);
        assert_eq!(classify_tool("FileWrite"), ToolKind::Write);
        assert_eq!(classify_tool("mcp__custom__paint"), ToolKind::Generic);
    }

    #[test]
    fn test_unparseable_payloads_keep_raw_trim_fallback() {
        let presentation = present_tool(&tool("CustomTool", "  not json  ", "  raw output  "), 80);
        assert_eq!(texts(&presentation.argument_lines), vec!["not json"]);
        assert_eq!(texts(&presentation.result_lines), vec!["raw output"]);
    }

    #[test]
    fn test_present_tool_strips_ansi_in_result() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"ls"}"#,
                "\u{1b}[1mfile.rs\u{1b}[0m",
            ),
            80,
        );
        assert_eq!(texts(&presentation.result_lines), vec!["file.rs"]);
    }

    #[test]
    fn test_sanitize_strips_osc_hyperlinks_with_bel() {
        let raw = "before \u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7} after";
        let lines = preview_lines(raw, None, 80);
        assert_eq!(texts(&lines), vec!["before link after".to_string()]);
    }

    #[test]
    fn test_sanitize_strips_osc_with_st_terminator() {
        let raw = "a\u{1b}]0;window title\u{1b}\\b";
        assert_eq!(texts(&preview_lines(raw, None, 80)), vec!["ab".to_string()]);
    }

    #[test]
    fn test_sanitize_strips_dcs_and_charset_escapes() {
        let raw = "x\u{1b}Pq#0\u{1b}\\y\u{1b}(Bz";
        assert_eq!(
            texts(&preview_lines(raw, None, 80)),
            vec!["xyz".to_string()]
        );
    }

    #[test]
    fn test_sanitize_expands_tabs_and_drops_control_chars() {
        let raw = "col\tsep\u{7}\u{1b}[31mred\u{1b}[0m\r\nnext";
        assert_eq!(
            texts(&preview_lines(raw, None, 80)),
            vec!["col    sepred".to_string(), "next".to_string()]
        );
    }
}
