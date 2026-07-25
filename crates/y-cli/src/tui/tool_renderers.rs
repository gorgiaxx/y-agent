use serde_json::Value;

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
    let input = parse_json(&tool.input_preview);
    let summary = tool_summary(kind, input.as_ref());
    let argument_lines = preview_lines(&tool.input_preview, width);
    let mut result_lines = preview_lines(&tool.result_preview, width);
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

fn preview_lines(raw: &str, width: usize) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let normalized = parse_json(raw).map_or_else(
        || raw.trim().to_string(),
        |value| match value {
            Value::String(text) => text,
            other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| raw.to_string()),
        },
    );
    normalized
        .lines()
        .flat_map(|line| wrap_line(line, width.max(20)))
        .collect()
}

fn parse_json(raw: &str) -> Option<Value> {
    serde_json::from_str(raw.trim()).ok()
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if line.chars().count() <= width {
        return vec![line.to_string()];
    }
    let chars: Vec<char> = line.chars().collect();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
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
}
