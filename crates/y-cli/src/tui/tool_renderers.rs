use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::state::{ToolCallInfo, ToolCallStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Shell,
    Read,
    Search,
    List,
    Edit,
    Web,
    Task,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPresentation {
    pub kind: ToolKind,
    pub verb: &'static str,
    pub summary: String,
    pub argument_lines: Vec<String>,
    pub result_lines: Vec<String>,
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
    let argument_lines = argument_preview_lines(kind, &tool.input_preview, input.as_ref(), width);
    let mut result_lines = result_preview_lines(kind, &tool.result_preview, result.as_ref(), width);
    if result_lines.is_empty() && tool.status != ToolCallStatus::Running {
        result_lines.push("(no output)".into());
    }

    ToolPresentation {
        kind,
        verb: tool_verb(kind, &tool.name),
        summary,
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

fn classify_tool(name: &str) -> ToolKind {
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
    let keys: &[&str] = match kind {
        ToolKind::Shell => &["command", "cmd", "script"],
        ToolKind::Read | ToolKind::Edit | ToolKind::List => {
            &["path", "file_path", "file", "directory"]
        }
        ToolKind::Search => &["query", "pattern", "search", "needle"],
        ToolKind::Web => &["url", "query", "href"],
        ToolKind::Task => &["description", "prompt", "task", "agent_name"],
        ToolKind::Generic => &[],
    };
    input
        .and_then(|value| first_string(value, keys))
        .unwrap_or_default()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object.get(*key).map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    })
}

/// Argument preview lines: extract the meaningful field(s) per tool kind,
/// falling back to the generic JSON preview when nothing obvious matches.
fn argument_preview_lines(
    kind: ToolKind,
    raw: &str,
    parsed: Option<&Value>,
    width: usize,
) -> Vec<String> {
    if let Some(value) = parsed {
        let extracted = match kind {
            ToolKind::Shell => text_field_lines(value, &["command", "cmd", "script"], width),
            ToolKind::Read | ToolKind::Edit => file_argument_lines(value, width),
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

/// Result preview lines: per-kind extraction first, then the generic
/// envelope-aware JSON preview.
fn result_preview_lines(
    kind: ToolKind,
    raw: &str,
    parsed: Option<&Value>,
    width: usize,
) -> Vec<String> {
    if let Some(value) = parsed {
        let extracted = match kind {
            ToolKind::Shell => shell_result_lines(value, width),
            ToolKind::Read | ToolKind::Edit => text_field_lines(value, &["content"], width),
            ToolKind::Web => text_field_lines(value, &["content", "text"], width),
            _ => None,
        };
        if let Some(lines) = extracted {
            return lines;
        }
    }
    preview_lines(raw, parsed, width)
}

/// Read/Edit inputs show the target path, then the first content payload
/// (`content`, `diff`, or `new_string`) when one is present.
fn file_argument_lines(value: &Value, width: usize) -> Option<Vec<String>> {
    let path = first_string(value, &["path", "file_path", "file"]);
    let body = field_text(value, &["content", "diff", "new_string"]);
    let mut lines = Vec::new();
    if let Some(path) = path {
        lines.extend(text_lines(&path, width));
    }
    if let Some(body) = body {
        lines.extend(text_lines(&body, width));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Shell results surface stdout first, then stderr with a `! ` prefix so
/// error output stands out from normal output. Returns `None` when neither
/// stream is present so callers fall back to the generic preview.
fn shell_result_lines(value: &Value, width: usize) -> Option<Vec<String>> {
    let object = value.as_object()?;
    if !object.contains_key("stdout") && !object.contains_key("stderr") {
        return None;
    }
    let mut lines = Vec::new();
    if let Some(stdout) = object.get("stdout") {
        lines.extend(text_lines(&display_text(stdout), width));
    }
    if let Some(stderr) = object.get("stderr") {
        let text = display_text(stderr);
        if !text.trim().is_empty() {
            // Reserve two columns for the `! ` marker so marked lines stay
            // within the same display width as stdout lines.
            let stderr_lines = text_lines(&text, width.saturating_sub(2));
            lines.extend(stderr_lines.into_iter().map(|line| format!("! {line}")));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Display lines for the first matching key of an object, or `None` when no
/// key matches or the extracted text renders to nothing.
fn text_field_lines(value: &Value, keys: &[&str], width: usize) -> Option<Vec<String>> {
    let lines = text_lines(&field_text(value, keys)?, width);
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Extract the first matching key's display text from an object.
fn field_text(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).map(display_text))
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
fn unwrap_envelope(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    ["result", "error"]
        .iter()
        .find_map(|key| object.get(*key).map(display_text))
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Split display text into lines: strip ANSI escape sequences (shell output
/// frequently carries color codes whose escape bytes would corrupt width
/// math and rendering), then wrap each line to the display width.
fn text_lines(text: &str, width: usize) -> Vec<String> {
    strip_ansi_codes(text)
        .lines()
        .flat_map(|line| wrap_line(line, width.max(20)))
        .collect()
}

fn preview_lines(raw: &str, parsed: Option<&Value>, width: usize) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let normalized = parsed.map_or_else(
        || raw.trim().to_string(),
        |value| match value {
            Value::String(text) => text.clone(),
            other => unwrap_envelope(other).unwrap_or_else(|| pretty_json(other)),
        },
    );
    text_lines(&normalized, width)
}

fn parse_json(raw: &str) -> Option<Value> {
    serde_json::from_str(raw.trim()).ok()
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

/// Strip ANSI CSI escape sequences (`ESC [` params/intermediates + final
/// byte). Hand-rolled to avoid a new dependency; covers the color/style
/// sequences shell tools commonly emit.
fn strip_ansi_codes(value: &str) -> String {
    if !value.contains('\u{1b}') {
        return value.to_string();
    }
    let mut stripped = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
                          // Skip until the final byte (0x40..=0x7E), inclusive.
            for seq_ch in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&seq_ch) {
                    break;
                }
            }
        } else {
            stripped.push(ch);
        }
    }
    stripped
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
        assert!(presentation.result_lines.iter().any(|line| line == "+new"));
    }

    #[test]
    fn test_unknown_tool_uses_generic_renderer() {
        let presentation = present_tool(&tool("CustomTool", r#"{"value":42}"#, "done"), 80);

        assert_eq!(presentation.kind, ToolKind::Generic);
        assert_eq!(presentation.verb, "Called");
        assert!(presentation
            .argument_lines
            .iter()
            .any(|line| line.contains("value")));
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
        assert_eq!(lines, vec!["error: failed".to_string()]);
    }

    #[test]
    fn test_preview_lines_strips_ansi_from_json_string_output() {
        let raw = "\"\\u001b[32mok\\u001b[0m done\"";
        let parsed = parse_json(raw);
        let lines = preview_lines(raw, parsed.as_ref(), 80);
        assert_eq!(lines, vec!["ok done".to_string()]);
    }

    #[test]
    fn test_result_envelope_unwraps_result_string_into_multiline() {
        let presentation = present_tool(
            &tool("CustomTool", "{}", r#"{"result":"line one\nline two"}"#),
            80,
        );
        assert_eq!(
            presentation.result_lines,
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
            presentation.result_lines,
            vec!["boom".to_string(), "stack trace".to_string()]
        );
    }

    #[test]
    fn test_result_envelope_pretty_prints_non_string_value() {
        let presentation = present_tool(&tool("CustomTool", "{}", r#"{"result":{"code":1}}"#), 80);
        let joined = presentation.result_lines.join("\n");
        assert!(joined.contains("\"code\": 1"));
        assert!(!joined.contains("result"));
    }

    #[test]
    fn test_multi_key_object_with_result_key_keeps_pretty_json() {
        let presentation =
            present_tool(&tool("CustomTool", "{}", r#"{"result":"x","extra":1}"#), 80);
        let joined = presentation.result_lines.join("\n");
        assert!(joined.contains("\"result\": \"x\""));
        assert!(joined.contains("\"extra\": 1"));
    }

    #[test]
    fn test_unwrapped_long_lines_still_wrap_by_display_width() {
        let long = "ab".repeat(30); // 60 display cells on one line
        let raw = format!(r#"{{"result":"{long}"}}"#);
        let presentation = present_tool(&tool("CustomTool", "{}", &raw), 20);
        assert!(presentation.result_lines.len() >= 3);
        for line in &presentation.result_lines {
            assert!(UnicodeWidthStr::width(line.as_str()) <= 20);
        }
    }

    #[test]
    fn test_shell_arguments_show_command_only() {
        let presentation = present_tool(
            &tool("ShellExec", r#"{"command":"cargo test","timeout":30}"#, ""),
            80,
        );
        assert_eq!(presentation.argument_lines, vec!["cargo test".to_string()]);
    }

    #[test]
    fn test_shell_arguments_fall_back_to_cmd_key() {
        let presentation = present_tool(&tool("ShellExec", r#"{"cmd":"ls -la"}"#, ""), 80);
        assert_eq!(presentation.argument_lines, vec!["ls -la".to_string()]);
    }

    #[test]
    fn test_shell_result_splits_stdout_and_marks_stderr() {
        let presentation = present_tool(
            &tool(
                "ShellExec",
                r#"{"command":"make"}"#,
                r#"{"stdout":"out one\nout two","stderr":"warn: slow"}"#,
            ),
            80,
        );
        assert_eq!(
            presentation.result_lines,
            vec![
                "out one".to_string(),
                "out two".to_string(),
                "! warn: slow".to_string(),
            ]
        );
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
        assert_eq!(presentation.result_lines, vec!["file.rs".to_string()]);
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
        let joined = presentation.result_lines.join("\n");
        assert!(joined.contains("\"files\": 2"));
    }

    #[test]
    fn test_shell_result_envelope_still_unwraps() {
        let presentation = present_tool(
            &tool("ShellExec", r#"{"command":"ls"}"#, r#"{"result":"a\nb"}"#),
            80,
        );
        assert_eq!(
            presentation.result_lines,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn test_read_arguments_show_path() {
        let presentation = present_tool(&tool("FileRead", r#"{"path":"src/lib.rs"}"#, ""), 80);
        assert_eq!(presentation.argument_lines, vec!["src/lib.rs".to_string()]);
    }

    #[test]
    fn test_edit_arguments_show_path_and_new_string() {
        let presentation = present_tool(
            &tool(
                "FileEdit",
                r#"{"path":"a.rs","old_string":"old","new_string":"new\ntext"}"#,
                "",
            ),
            80,
        );
        assert_eq!(
            presentation.argument_lines,
            vec!["a.rs".to_string(), "new".to_string(), "text".to_string()]
        );
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
            presentation.result_lines,
            vec!["line1".to_string(), "line2".to_string()]
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
            presentation.argument_lines,
            vec!["https://example.com".to_string()]
        );
        assert_eq!(
            presentation.result_lines,
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn test_search_list_task_arguments_extract_primary_field() {
        let search = present_tool(&tool("Grep", r#"{"pattern":"foo","path":"src"}"#, ""), 80);
        assert_eq!(search.argument_lines, vec!["foo".to_string()]);
        let list = present_tool(&tool("DirectoryList", r#"{"directory":"src"}"#, ""), 80);
        assert_eq!(list.argument_lines, vec!["src".to_string()]);
        let task = present_tool(&tool("Task", r#"{"prompt":"do stuff"}"#, ""), 80);
        assert_eq!(task.argument_lines, vec!["do stuff".to_string()]);
    }

    #[test]
    fn test_unparseable_payloads_keep_raw_trim_fallback() {
        let presentation = present_tool(&tool("CustomTool", "  not json  ", "  raw output  "), 80);
        assert_eq!(presentation.argument_lines, vec!["not json".to_string()]);
        assert_eq!(presentation.result_lines, vec!["raw output".to_string()]);
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
        assert_eq!(presentation.result_lines, vec!["file.rs".to_string()]);
    }
}
