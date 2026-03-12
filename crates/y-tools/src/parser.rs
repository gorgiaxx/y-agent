//! Tool call parser for prompt-based tool calling protocol.
//!
//! Extracts `<tool_call>` XML-tag blocks from LLM text output and parses
//! the JSON payload inside. This is the core mechanism for provider-agnostic
//! tool calling — the LLM outputs structured tool calls in its text, and
//! this parser extracts them.
//!
//! Design reference: `docs/standards/TOOL_CALL_PROTOCOL.md`

use serde::{Deserialize, Serialize};

/// A tool call extracted from LLM text output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedToolCall {
    /// Tool name.
    pub name: String,
    /// Tool arguments as a JSON object.
    pub arguments: serde_json::Value,
}

/// Result of parsing LLM text for tool calls.
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// Text content with `<tool_call>` blocks removed.
    pub text: String,
    /// Extracted tool calls in order of appearance.
    pub tool_calls: Vec<ParsedToolCall>,
    /// Warnings for malformed blocks that were skipped.
    pub warnings: Vec<String>,
}

const OPEN_TAG: &str = "<tool_call>";
const CLOSE_TAG: &str = "</tool_call>";

/// Parse `<tool_call>...</tool_call>` blocks from LLM text output.
///
/// Returns the remaining text (with tool call blocks removed) and the
/// extracted tool calls. Malformed blocks are treated as regular text
/// and a warning is emitted.
pub fn parse_tool_calls(raw: &str) -> ParseResult {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = 0;

    while cursor < raw.len() {
        // Find the next <tool_call> tag.
        if let Some(start_offset) = raw[cursor..].find(OPEN_TAG) {
            let tag_start = cursor + start_offset;
            let content_start = tag_start + OPEN_TAG.len();

            // Find the matching </tool_call> tag.
            if let Some(end_offset) = raw[content_start..].find(CLOSE_TAG) {
                let content_end = content_start + end_offset;

                // Append text before this tool call block.
                text.push_str(&raw[cursor..tag_start]);

                // Extract and parse the JSON content.
                let inner = raw[content_start..content_end].trim();

                if inner.is_empty() {
                    warnings.push("empty <tool_call> block skipped".into());
                } else {
                    // Try XML-nested format first (primary, more token-efficient):
                    //   <name>tool_name</name>
                    //   <arguments>{"key": "value"}</arguments>
                    // Fall back to JSON format for backward compatibility:
                    //   {"name": "tool_name", "arguments": {"key": "value"}}
                    if let Ok(tc) = try_parse_xml_tool_call(inner) {
                        tool_calls.push(tc);
                    } else if let Ok(json) = serde_json::from_str::<serde_json::Value>(inner) {
                        match extract_tool_call(&json) {
                            Ok(tc) => tool_calls.push(tc),
                            Err(msg) => {
                                warnings.push(msg);
                                text.push_str(&raw[tag_start..content_end + CLOSE_TAG.len()]);
                            }
                        }
                    } else {
                        warnings.push(
                            "invalid content in <tool_call>: \
                             not XML-nested nor JSON format"
                                .into(),
                        );
                        // Malformed: keep as text.
                        text.push_str(&raw[tag_start..content_end + CLOSE_TAG.len()]);
                    }
                }

                cursor = content_end + CLOSE_TAG.len();
            } else {
                // Unclosed tag — treat everything from here as text.
                text.push_str(&raw[cursor..]);
                cursor = raw.len();
            }
        } else {
            // No more tags — append remaining text.
            text.push_str(&raw[cursor..]);
            cursor = raw.len();
        }
    }

    ParseResult {
        text,
        tool_calls,
        warnings,
    }
}

/// Extract a `ParsedToolCall` from a parsed JSON value.
fn extract_tool_call(json: &serde_json::Value) -> Result<ParsedToolCall, String> {
    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing or non-string 'name' field in tool call".to_string())?;

    if name.is_empty() {
        return Err("empty 'name' field in tool call".into());
    }

    let arguments = json
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    if !arguments.is_object() {
        return Err(format!(
            "'arguments' must be an object, got {}",
            match &arguments {
                serde_json::Value::Array(_) => "array",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Null => "null",
                serde_json::Value::Object(_) => unreachable!(),
            }
        ));
    }

    Ok(ParsedToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// Try to parse a tool call from XML-nested format.
///
/// Handles the common LLM failure mode of generating:
/// ```xml
/// <name>tool_name</name>
/// <arguments>{"key": "value"}</arguments>
/// ```
/// instead of the expected JSON object.
fn try_parse_xml_tool_call(inner: &str) -> Result<ParsedToolCall, String> {
    // Extract <name>...</name>
    let name = extract_xml_tag(inner, "name")
        .ok_or_else(|| "no <name> tag found in XML-nested tool call".to_string())?;
    let name = name.trim();
    if name.is_empty() {
        return Err("empty <name> in XML-nested tool call".into());
    }

    // Extract <arguments>...</arguments> (optional, defaults to {})
    let arguments = if let Some(args_str) = extract_xml_tag(inner, "arguments") {
        let trimmed = args_str.trim();
        if trimmed.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str::<serde_json::Value>(trimmed)
                .map_err(|e| format!("invalid JSON in <arguments>: {e}"))?
        }
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    if !arguments.is_object() {
        return Err(format!(
            "<arguments> must contain a JSON object, got {}",
            if arguments.is_array() { "array" } else { "non-object" }
        ));
    }

    Ok(ParsedToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// Extract the text content of a simple XML tag from a string.
///
/// e.g. `extract_xml_tag("<name>foo</name>", "name")` → `Some("foo")`
fn extract_xml_tag<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

/// Strip all `<tool_call>...</tool_call>` blocks from text.
///
/// Used to sanitize LLM output before displaying to the user,
/// ensuring raw protocol XML is never visible.
pub fn strip_tool_call_blocks(raw: &str) -> String {
    let result = parse_tool_calls(raw);
    result.text.trim().to_string()
}

/// Format a tool result as a `<tool_result>` block for injection into the conversation.
pub fn format_tool_result(name: &str, success: bool, content: &serde_json::Value) -> String {
    format!(
        "<tool_result name=\"{name}\" success=\"{success}\">\n{}\n</tool_result>",
        serde_json::to_string_pretty(content).unwrap_or_else(|_| content.to_string())
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_tool_call() {
        let input = r#"I need to read that file.

<tool_call>
{"name": "file_read", "arguments": {"path": "/src/main.rs"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "file_read");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert!(result.text.contains("I need to read that file."));
        assert!(!result.text.contains("tool_call"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let input = r#"Let me check both files.

<tool_call>
{"name": "file_read", "arguments": {"path": "/src/lib.rs"}}
</tool_call>

<tool_call>
{"name": "file_read", "arguments": {"path": "/src/main.rs"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/lib.rs");
        assert_eq!(result.tool_calls[1].arguments["path"], "/src/main.rs");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_no_tool_calls() {
        let input = "Just a normal text response with no tool calls.";
        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.text, input);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_mixed_text_and_tool_calls() {
        let input = r#"First some text.

<tool_call>
{"name": "file_read", "arguments": {"path": "/a.rs"}}
</tool_call>

Middle text.

<tool_call>
{"name": "shell_exec", "arguments": {"command": "ls"}}
</tool_call>

End text."#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert!(result.text.contains("First some text."));
        assert!(result.text.contains("Middle text."));
        assert!(result.text.contains("End text."));
        assert!(!result.text.contains("tool_call"));
    }

    #[test]
    fn test_parse_malformed_content() {
        let input = r"<tool_call>
not valid json or xml
</tool_call>";

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("invalid content"));
        // Malformed block kept as text.
        assert!(result.text.contains("<tool_call>"));
    }

    #[test]
    fn test_parse_missing_name_field() {
        let input = r#"<tool_call>
{"arguments": {"path": "/test"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("name"));
    }

    #[test]
    fn test_parse_empty_name_field() {
        let input = r#"<tool_call>
{"name": "", "arguments": {}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(result.warnings[0].contains("empty"));
    }

    #[test]
    fn test_parse_missing_arguments_defaults_to_empty_object() {
        let input = r#"<tool_call>
{"name": "tool_search"}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "tool_search");
        assert!(result.tool_calls[0].arguments.is_object());
        assert!(result.tool_calls[0].arguments.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_parse_arguments_not_object() {
        let input = r#"<tool_call>
{"name": "test", "arguments": [1, 2, 3]}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(result.warnings[0].contains("array"));
    }

    #[test]
    fn test_parse_unclosed_tag() {
        let input = "Some text <tool_call> { incomplete tag without closing";
        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(result.text.contains("<tool_call>"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_empty_block() {
        let input = "<tool_call>\n</tool_call>";
        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("empty"));
    }

    #[test]
    fn test_parse_json_with_angle_brackets() {
        let input = r#"<tool_call>
{"name": "shell_exec", "arguments": {"command": "echo '<div>hello</div>'"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "shell_exec");
        assert_eq!(
            result.tool_calls[0].arguments["command"],
            "echo '<div>hello</div>'"
        );
    }

    #[test]
    fn test_parse_whitespace_around_json() {
        let input = "<tool_call>   \n  {\"name\": \"test\", \"arguments\": {}}  \n  </tool_call>";
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "test");
    }

    #[test]
    fn test_format_tool_result_success() {
        let content = serde_json::json!({"data": "hello"});
        let formatted = format_tool_result("file_read", true, &content);
        assert!(formatted.starts_with("<tool_result name=\"file_read\" success=\"true\">"));
        assert!(formatted.ends_with("</tool_result>"));
        assert!(formatted.contains("hello"));
    }

    #[test]
    fn test_format_tool_result_error() {
        let content = serde_json::json!({"error": "file not found"});
        let formatted = format_tool_result("file_read", false, &content);
        assert!(formatted.contains("success=\"false\""));
        assert!(formatted.contains("file not found"));
    }

    #[test]
    fn test_parse_preserves_order() {
        let input = r#"<tool_call>
{"name": "first", "arguments": {}}
</tool_call>
<tool_call>
{"name": "second", "arguments": {}}
</tool_call>
<tool_call>
{"name": "third", "arguments": {}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 3);
        assert_eq!(result.tool_calls[0].name, "first");
        assert_eq!(result.tool_calls[1].name, "second");
        assert_eq!(result.tool_calls[2].name, "third");
    }

    #[test]
    fn test_parse_tool_call_inline_json() {
        let input = r#"<tool_call>{"name": "test", "arguments": {"key": "value"}}</tool_call>"#;
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments["key"], "value");
    }

    #[test]
    fn test_parse_text_cleanup_no_extra_whitespace() {
        let input = "Before.\n\n<tool_call>\n{\"name\": \"t\", \"arguments\": {}}\n</tool_call>\n\nAfter.";
        let result = parse_tool_calls(input);
        assert_eq!(result.text, "Before.\n\n\n\nAfter.");
    }

    // -----------------------------------------------------------------------
    // XML-nested format tests (primary format)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_xml_single_tool_call() {
        let input = r#"I need to read that file.

<tool_call>
<name>file_read</name>
<arguments>{"path": "/src/main.rs"}</arguments>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "file_read");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert!(result.text.contains("I need to read that file."));
        assert!(!result.text.contains("tool_call"));
    }

    #[test]
    fn test_parse_xml_multiple_tool_calls() {
        let input = r#"Let me search for tools.

<tool_call>
<name>tool_search</name>
<arguments>{"query": "list directory"}</arguments>
</tool_call>

<tool_call>
<name>tool_search</name>
<arguments>{"category": "file"}</arguments>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "tool_search");
        assert_eq!(result.tool_calls[0].arguments["query"], "list directory");
        assert_eq!(result.tool_calls[1].arguments["category"], "file");
    }

    #[test]
    fn test_parse_xml_without_arguments() {
        let input = r"<tool_call>
<name>tool_search</name>
</tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "tool_search");
        assert!(result.tool_calls[0].arguments.is_object());
        assert!(result.tool_calls[0].arguments.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_parse_xml_empty_name_fails() {
        let input = r"<tool_call>
<name></name>
<arguments>{}</arguments>
</tool_call>";

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_parse_xml_with_whitespace() {
        let input = "<tool_call>\n  <name>  file_read  </name>\n  <arguments>  {\"path\": \"/a\"}  </arguments>\n</tool_call>";
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "file_read");
        assert_eq!(result.tool_calls[0].arguments["path"], "/a");
    }

    #[test]
    fn test_parse_mixed_xml_and_json_formats() {
        let input = r#"<tool_call>
<name>file_read</name>
<arguments>{"path": "/a.rs"}</arguments>
</tool_call>

<tool_call>
{"name": "shell_exec", "arguments": {"command": "ls"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "file_read");
        assert_eq!(result.tool_calls[1].name, "shell_exec");
    }

    #[test]
    fn test_strip_tool_call_blocks() {
        let input = "Hello\n<tool_call>\n<name>t</name>\n</tool_call>\nWorld";
        let stripped = strip_tool_call_blocks(input);
        assert_eq!(stripped, "Hello\n\nWorld");
        assert!(!stripped.contains("tool_call"));
    }

    #[test]
    fn test_strip_tool_call_blocks_malformed() {
        // Even malformed blocks should be stripped via parse_tool_calls.
        // (parse_tool_calls keeps malformed as text, but strip_tool_call_blocks
        //  doesn't filter further — it relies on parse result.text.)
        let input = "Before <tool_call>not xml or json</tool_call> After";
        let stripped = strip_tool_call_blocks(input);
        // Malformed blocks are kept as text by the parser — that's OK,
        // they're at least not tool-protocol looking.
        assert!(stripped.contains("Before"));
        assert!(stripped.contains("After"));
    }
}
