//! Tool call parser for prompt-based tool calling protocol.
//!
//! Extracts tool call blocks from LLM text output using multiple envelope
//! formats (y-agent `<tool_call>`, `DeepSeek` DSML, `MiniMax` M2) and parses
//! the inner payload. This is the core mechanism for provider-agnostic
//! tool calling -- the LLM outputs structured tool calls in its text, and
//! this parser extracts them with tolerant matching.
//!
//! Supported envelope formats:
//! - `<tool_call>...</tool_call>` (y-agent custom, GLM-4, `Qwen3Coder`)
//! - `<longcat_tool_call>...</longcat_tool_call>` (Longcat Flash)
//! - `<function_calls>...</function_calls>` (generic XML tool calling)
//! - `<|DSML|function_calls>...</|DSML|function_calls>` (`DeepSeek` V3.2)
//! - `<minimax:tool_call>...</minimax:tool_call>` (`MiniMax` M2)
//!
//! Design reference: `docs/standards/TOOL_CALL_PROTOCOL.md`

use serde::{Deserialize, Serialize};

/// The default XML-based tool calling syntax prompt for `PromptBased` models.
pub const PROMPT_TOOL_CALL_SYNTAX: &str = r#"When you do need a tool, output a <tool_call> block with <name> and <arguments> tags:

<tool_call>
<name>tool_name</name>
<arguments>{"param1": "value1"}</arguments>
</tool_call>

You may include multiple <tool_call> blocks in a single response. Each will be executed in order.

After each tool call, you will receive the result in a <tool_result> block:

<tool_result name="tool_name" success="true">
{"result_key": "result_value"}
</tool_result>"#;

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

/// An envelope tag pair that wraps tool call content.
struct TagPair {
    open: &'static str,
    close: &'static str,
}

/// All supported envelope formats, checked in order.
const ENVELOPES: &[TagPair] = &[
    TagPair {
        open: "<tool_call>",
        close: "</tool_call>",
    },
    TagPair {
        open: "<longcat_tool_call>",
        close: "</longcat_tool_call>",
    },
    TagPair {
        open: "<function_calls>",
        close: "</function_calls>",
    },
    TagPair {
        open: "<\u{ff5c}DSML\u{ff5c}function_calls>",
        close: "</\u{ff5c}DSML\u{ff5c}function_calls>",
    },
    TagPair {
        open: "<minimax:tool_call>",
        close: "</minimax:tool_call>",
    },
];

/// Patch `MiniMax` 2.5 output that omits the `<minimax:tool_call>` open tag.
///
/// `MiniMax` 2.5 frequently produces malformed output like:
/// ```text
/// <invoke name="Glob"> <parameter name="query">*.rs</parameter> </invoke> </minimax:tool_call>
/// ```
/// where the closing `</minimax:tool_call>` is present but the opening tag
/// is missing. This function scans for each such orphaned close tag and
/// inserts `<minimax:tool_call>` immediately before the `<invoke` that
/// belongs to it. Handles multiple broken tool calls in a single response.
fn patch_minimax_missing_open_tags(raw: &str) -> String {
    const OPEN: &str = "<minimax:tool_call>";
    const CLOSE: &str = "</minimax:tool_call>";

    // Fast path: nothing to patch if the close tag is absent.
    if !raw.contains(CLOSE) {
        return raw.to_string();
    }

    let mut result = String::with_capacity(raw.len() + OPEN.len() * 4);
    let mut cursor = 0;

    while cursor < raw.len() {
        // Find the next close tag from the current cursor.
        let Some(close_offset) = raw[cursor..].find(CLOSE) else {
            // No more close tags -- append the rest and finish.
            result.push_str(&raw[cursor..]);
            break;
        };
        let close_abs = cursor + close_offset;

        // The region between `cursor` and `close_abs` is where the open tag
        // and invoke content should live.
        let region = &raw[cursor..close_abs];

        if region.contains(OPEN) {
            // The open tag is present -- no patching needed for this block.
            result.push_str(&raw[cursor..close_abs + CLOSE.len()]);
        } else {
            // Missing open tag. Find the `<invoke` that starts this tool call
            // block by scanning backwards from the close tag position.
            if let Some(invoke_offset) = region.rfind("<invoke") {
                // Append text before the invoke tag, then insert the open tag.
                result.push_str(&region[..invoke_offset]);
                result.push_str(OPEN);
                result.push_str(&region[invoke_offset..]);
                result.push_str(CLOSE);
            } else {
                // No `<invoke` found -- cannot patch; emit the region as-is.
                result.push_str(&raw[cursor..close_abs + CLOSE.len()]);
            }
        }

        cursor = close_abs + CLOSE.len();
    }

    result
}

/// Find the earliest occurring envelope open tag from `cursor` onward.
///
/// Returns `(tag_pair_index, absolute_offset_of_open_tag)` for the earliest
/// match, or `None` if no envelope tag is found.
fn find_earliest_envelope(raw: &str, cursor: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (idx, pair) in ENVELOPES.iter().enumerate() {
        if let Some(offset) = raw[cursor..].find(pair.open) {
            let abs = cursor + offset;
            if best.is_none_or(|(_, prev)| abs < prev) {
                best = Some((idx, abs));
            }
        }
    }
    best
}

/// Parse tool call blocks from LLM text output.
///
/// Recognises multiple envelope formats (y-agent `<tool_call>`, `DeepSeek`
/// DSML, `MiniMax`) and dispatches inner content to format-specific parsers.
/// Returns the remaining text (with tool call blocks removed) and the
/// extracted tool calls. Malformed blocks are treated as regular text
/// and a warning is emitted.
pub fn parse_tool_calls(raw: &str) -> ParseResult {
    // Pre-process: patch MiniMax 2.5 output with missing open tags.
    let patched = patch_minimax_missing_open_tags(raw);
    let raw = &patched;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut warnings = Vec::new();
    let mut cursor = 0;

    while cursor < raw.len() {
        if let Some((env_idx, tag_start)) = find_earliest_envelope(raw, cursor) {
            let pair = &ENVELOPES[env_idx];
            let content_start = tag_start + pair.open.len();

            // Find the matching close tag.
            if let Some(end_offset) = raw[content_start..].find(pair.close) {
                let content_end = content_start + end_offset;

                // Append text before this tool call block.
                text.push_str(&raw[cursor..tag_start]);

                // Extract and parse the inner content.
                let inner_raw = raw[content_start..content_end].trim();
                let inner = &sanitize_json_newlines(inner_raw);

                if inner.is_empty() {
                    warnings.push(format!("empty {} block skipped", pair.open));
                } else {
                    parse_inner_content(
                        inner,
                        raw,
                        tag_start,
                        content_end,
                        pair,
                        &mut tool_calls,
                        &mut warnings,
                        &mut text,
                    );
                }

                cursor = content_end + pair.close.len();
            } else {
                // Unclosed tag -- treat everything from here as text.
                text.push_str(&raw[cursor..]);
                cursor = raw.len();
            }
        } else {
            // No more envelope tags -- append remaining text.
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

/// Dispatch inner content of a matched envelope to format-specific parsers.
///
/// Tries parsers in priority order:
/// 1. XML-nested (`<name>...</name><arguments>...</arguments>`)
/// 2. Function-attribute (`<function=NAME><parameter=KEY>VALUE</parameter></function>`)
/// 3. DSML invoke (`<|DSML|invoke name="...">...`)
/// 4. `MiniMax` invoke (`<invoke name="...">...`)
/// 5. GLM `arg_key/arg_value` (`func_name\n<arg_key>k</arg_key><arg_value>v</arg_value>`)
/// 6. JSON object (`{"name": "...", "arguments": {...}}`)
fn parse_inner_content(
    inner: &str,
    raw: &str,
    tag_start: usize,
    content_end: usize,
    pair: &TagPair,
    tool_calls: &mut Vec<ParsedToolCall>,
    warnings: &mut Vec<String>,
    text: &mut String,
) {
    // 1. XML-nested format
    if let Ok(tc) = try_parse_xml_tool_call(inner) {
        tool_calls.push(tc);
        return;
    }

    // 2. Function-attribute format (Llama/Qwen)
    if let Ok(tc) = try_parse_function_format(inner) {
        tool_calls.push(tc);
        return;
    }

    // 3. DSML invoke format (DeepSeek V3.2) -- can yield multiple tool calls
    if let Ok(tcs) = try_parse_dsml_invokes(inner) {
        tool_calls.extend(tcs);
        return;
    }

    // 4. MiniMax invoke format -- can yield multiple tool calls
    if let Ok(tcs) = try_parse_minimax_invokes(inner) {
        tool_calls.extend(tcs);
        return;
    }

    // 5. GLM arg_key/arg_value format
    if let Ok(tc) = try_parse_glm_arg_format(inner) {
        tool_calls.push(tc);
        return;
    }

    // 6. JSON fallback
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(inner) {
        match extract_tool_call(&json) {
            Ok(tc) => {
                tool_calls.push(tc);
                return;
            }
            Err(msg) => {
                warnings.push(msg);
                text.push_str(&raw[tag_start..content_end + pair.close.len()]);
                return;
            }
        }
    }

    // Nothing matched -- malformed block.
    warnings.push(format!(
        "invalid content in {}: not a recognized tool call format",
        pair.open
    ));
    text.push_str(&raw[tag_start..content_end + pair.close.len()]);
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
            if arguments.is_array() {
                "array"
            } else {
                "non-object"
            }
        ));
    }

    Ok(ParsedToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// Try to parse a tool call from function-attribute format.
///
/// Handles formats commonly produced by Llama/Qwen-family models:
/// ```xml
/// <function=Browser>
/// <parameter=url>https://example.com</parameter>
/// <parameter=query>hello</parameter>
/// </function>
/// ```
///
/// Also handles `<action>` tags and bare text inside the `<function>` block.
fn try_parse_function_format(inner: &str) -> Result<ParsedToolCall, String> {
    // Match <function=NAME> ... </function>
    let func_prefix = "<function=";
    let func_start = inner
        .find(func_prefix)
        .ok_or_else(|| "no <function=...> tag found".to_string())?;
    let after_prefix = func_start + func_prefix.len();

    // Find the closing `>` of the opening tag.
    let tag_close = inner[after_prefix..]
        .find('>')
        .ok_or_else(|| "unclosed <function= tag".to_string())?;
    let name = inner[after_prefix..after_prefix + tag_close].trim();
    if name.is_empty() {
        return Err("empty function name in <function=...> tag".into());
    }

    // Extract body between `<function=NAME>` and `</function>`.
    let body_start = after_prefix + tag_close + 1;
    let body = if let Some(end_offset) = inner[body_start..].find("</function>") {
        inner[body_start..body_start + end_offset].trim()
    } else {
        // No closing </function> -- use everything after the opening tag.
        inner[body_start..].trim()
    };

    // Collect parameters from <parameter=KEY>VALUE</parameter> tags.
    let mut args = serde_json::Map::new();
    let param_prefix = "<parameter=";
    let mut cursor = 0;
    while cursor < body.len() {
        if let Some(p_start) = body[cursor..].find(param_prefix) {
            let abs_start = cursor + p_start + param_prefix.len();
            if let Some(key_end) = body[abs_start..].find('>') {
                let key = body[abs_start..abs_start + key_end].trim();
                let val_start = abs_start + key_end + 1;
                let close_tag = "</parameter>";
                let val_end = body[val_start..]
                    .find(close_tag)
                    .map_or(body.len(), |i| val_start + i);
                let value = body[val_start..val_end].trim();
                if !key.is_empty() {
                    args.insert(
                        key.to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
                cursor = if val_end < body.len() {
                    val_end + close_tag.len()
                } else {
                    body.len()
                };
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Also extract <action>VALUE</action> tags (variant seen in some models).
    if let Some(action_val) = extract_xml_tag(body, "action") {
        let action = action_val.trim();
        if !action.is_empty() {
            args.insert(
                "action".to_string(),
                serde_json::Value::String(action.to_string()),
            );
        }
    }

    // If the body looks like a JSON object, try parsing it directly.
    if args.is_empty() && body.starts_with('{') {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            if json.is_object() {
                return Ok(ParsedToolCall {
                    name: name.to_string(),
                    arguments: json,
                });
            }
        }
    }

    Ok(ParsedToolCall {
        name: name.to_string(),
        arguments: serde_json::Value::Object(args),
    })
}

/// Try to parse `DeepSeek` V3.2 DSML invoke blocks from inner content.
///
/// Handles:
/// ```xml
/// <|DSML|invoke name="GetWeather">
/// <|DSML|parameter name="location" string="true">value</|DSML|parameter>
/// </|DSML|invoke>
/// ```
///
/// A single DSML envelope can contain multiple `<|DSML|invoke>` blocks,
/// so this returns `Vec<ParsedToolCall>`.
fn try_parse_dsml_invokes(inner: &str) -> Result<Vec<ParsedToolCall>, String> {
    // Fullwidth vertical bar used by DeepSeek DSML tags.
    const INVOKE_OPEN: &str = "<\u{ff5c}DSML\u{ff5c}invoke ";
    const INVOKE_CLOSE: &str = "</\u{ff5c}DSML\u{ff5c}invoke>";
    const PARAM_OPEN: &str = "<\u{ff5c}DSML\u{ff5c}parameter ";
    const PARAM_CLOSE: &str = "</\u{ff5c}DSML\u{ff5c}parameter>";

    if !inner.contains(INVOKE_OPEN) {
        return Err("no DSML invoke tag found".into());
    }

    let mut results = Vec::new();
    let mut cursor = 0;

    while cursor < inner.len() {
        let Some(inv_start) = inner[cursor..].find(INVOKE_OPEN) else {
            break;
        };
        let inv_abs = cursor + inv_start;
        let after_invoke_prefix = inv_abs + INVOKE_OPEN.len();

        // Extract function name from `name="..."`.
        let name = extract_quoted_attr(&inner[after_invoke_prefix..], "name")
            .ok_or_else(|| "missing name attribute in DSML invoke".to_string())?;
        if name.is_empty() {
            return Err("empty name in DSML invoke".into());
        }

        // Find closing tag for this invoke block.
        let invoke_body_start = inner[after_invoke_prefix..]
            .find('>')
            .map_or(after_invoke_prefix, |i| after_invoke_prefix + i + 1);

        let invoke_end = inner[invoke_body_start..]
            .find(INVOKE_CLOSE)
            .map_or(inner.len(), |i| invoke_body_start + i);

        let body = &inner[invoke_body_start..invoke_end];

        // Extract parameters.
        let mut args = serde_json::Map::new();
        let mut pcursor = 0;
        while pcursor < body.len() {
            let Some(p_start) = body[pcursor..].find(PARAM_OPEN) else {
                break;
            };
            let p_abs = pcursor + p_start;
            let after_param_prefix = p_abs + PARAM_OPEN.len();

            let param_name =
                extract_quoted_attr(&body[after_param_prefix..], "name").unwrap_or_default();

            let param_body_start = body[after_param_prefix..]
                .find('>')
                .map_or(after_param_prefix, |i| after_param_prefix + i + 1);

            let param_end = body[param_body_start..]
                .find(PARAM_CLOSE)
                .map_or(body.len(), |i| param_body_start + i);

            let value = body[param_body_start..param_end].trim();

            if !param_name.is_empty() {
                args.insert(
                    param_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            }

            pcursor = if param_end < body.len() {
                param_end + PARAM_CLOSE.len()
            } else {
                body.len()
            };
        }

        results.push(ParsedToolCall {
            name: name.to_string(),
            arguments: serde_json::Value::Object(args),
        });

        cursor = if invoke_end < inner.len() {
            invoke_end + INVOKE_CLOSE.len()
        } else {
            inner.len()
        };
    }

    if results.is_empty() {
        return Err("no complete DSML invoke blocks found".into());
    }
    Ok(results)
}

/// Try to parse `MiniMax` M2 invoke blocks from inner content.
///
/// Handles:
/// ```xml
/// <invoke name="func_name">
/// <parameter name="key">value</parameter>
/// </invoke>
/// ```
///
/// A single `MiniMax` envelope can contain multiple `<invoke>` blocks.
fn try_parse_minimax_invokes(inner: &str) -> Result<Vec<ParsedToolCall>, String> {
    const INVOKE_OPEN: &str = "<invoke name=";
    const INVOKE_CLOSE: &str = "</invoke>";
    const PARAM_OPEN: &str = "<parameter name=";
    const PARAM_CLOSE: &str = "</parameter>";

    if !inner.contains(INVOKE_OPEN) {
        return Err("no MiniMax invoke tag found".into());
    }

    let mut results = Vec::new();
    let mut cursor = 0;

    while cursor < inner.len() {
        let Some(inv_start) = inner[cursor..].find(INVOKE_OPEN) else {
            break;
        };
        let inv_abs = cursor + inv_start;
        let after_prefix = inv_abs + INVOKE_OPEN.len();

        // Extract function name from the segment after `name=`. Since
        // INVOKE_OPEN already consumed `name=`, the segment starts with
        // the quoted value (e.g. `"FileRead">`).
        let tag_rest = &inner[after_prefix..];
        let tag_close = tag_rest.find('>').ok_or("unclosed invoke tag")?;
        let name_segment = tag_rest[..tag_close].trim();
        let name = extract_first_quoted(name_segment).unwrap_or_else(|| strip_quotes(name_segment));
        if name.is_empty() {
            return Err("empty function name in MiniMax invoke".into());
        }

        let body_start = after_prefix + tag_close + 1;
        let body_end = inner[body_start..]
            .find(INVOKE_CLOSE)
            .map_or(inner.len(), |i| body_start + i);

        let body = &inner[body_start..body_end];

        // Extract parameters.
        let mut args = serde_json::Map::new();
        let mut pcursor = 0;
        while pcursor < body.len() {
            let Some(p_start) = body[pcursor..].find(PARAM_OPEN) else {
                break;
            };
            let p_abs = pcursor + p_start;
            let after_param = p_abs + PARAM_OPEN.len();

            let param_rest = &body[after_param..];
            let Some(param_close) = param_rest.find('>') else {
                break;
            };
            let param_segment = param_rest[..param_close].trim();
            // PARAM_OPEN already consumed `name=`, so the segment starts with
            // the quoted value (e.g. `"path" string="true"`). Extract just the
            // first quoted string as the parameter name.
            let param_name =
                extract_first_quoted(param_segment).unwrap_or_else(|| strip_quotes(param_segment));

            let val_start = after_param + param_close + 1;
            let val_end = body[val_start..]
                .find(PARAM_CLOSE)
                .map_or(body.len(), |i| val_start + i);

            let value = body[val_start..val_end].trim();
            if !param_name.is_empty() {
                args.insert(
                    param_name.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            }

            pcursor = if val_end < body.len() {
                val_end + PARAM_CLOSE.len()
            } else {
                body.len()
            };
        }

        results.push(ParsedToolCall {
            name: name.to_string(),
            arguments: serde_json::Value::Object(args),
        });

        cursor = if body_end < inner.len() {
            body_end + INVOKE_CLOSE.len()
        } else {
            inner.len()
        };
    }

    if results.is_empty() {
        return Err("no complete MiniMax invoke blocks found".into());
    }
    Ok(results)
}

/// Try to parse GLM-4 / GLM-4.7 `arg_key/arg_value` format.
///
/// Handles:
/// ```text
/// func_name
/// <arg_key>location</arg_key>
/// <arg_value>Beijing</arg_value>
/// ```
///
/// Also handles zero-arg calls: `func_name` with no `<arg_key>` tags.
fn try_parse_glm_arg_format(inner: &str) -> Result<ParsedToolCall, String> {
    const ARG_KEY_OPEN: &str = "<arg_key>";
    const ARG_KEY_CLOSE: &str = "</arg_key>";
    const ARG_VAL_OPEN: &str = "<arg_value>";
    const ARG_VAL_CLOSE: &str = "</arg_value>";

    // GLM format requires either `<arg_key>` tags or a bare function name
    // without any other XML tags in the content. We use the presence of
    // `<arg_key>` or a simple identifier-like string to identify this format.
    let has_arg_keys = inner.contains(ARG_KEY_OPEN);

    // If no arg_key tags, check if this looks like a bare function name
    // (single identifier, no XML-like content other than what we already tried).
    if !has_arg_keys {
        let trimmed = inner.trim();
        // A bare function name: no spaces, no '<', no '{', no '"', looks like an identifier
        let is_identifier = !trimmed.is_empty()
            && !trimmed.contains('<')
            && !trimmed.contains('{')
            && !trimmed.contains('"')
            && !trimmed.contains(' ')
            && !trimmed.contains('\n');
        if is_identifier {
            return Ok(ParsedToolCall {
                name: trimmed.to_string(),
                arguments: serde_json::Value::Object(serde_json::Map::new()),
            });
        }
        return Err("no GLM arg_key tags found and not a bare function name".into());
    }

    // Extract function name: everything before the first `<arg_key>` or `<`.
    let first_tag = inner.find(ARG_KEY_OPEN).unwrap_or(inner.len());
    // Function name might be on its own line or directly before the first tag.
    let name_part = inner[..first_tag].trim();
    // The function name could have a newline separator.
    let name = name_part.lines().next().unwrap_or("").trim();
    if name.is_empty() {
        return Err("empty function name in GLM arg format".into());
    }

    // Extract key-value pairs.
    let mut args = serde_json::Map::new();
    let mut cursor = 0;
    while cursor < inner.len() {
        let Some(key_start) = inner[cursor..].find(ARG_KEY_OPEN) else {
            break;
        };
        let key_abs = cursor + key_start + ARG_KEY_OPEN.len();

        let Some(key_end_off) = inner[key_abs..].find(ARG_KEY_CLOSE) else {
            break;
        };
        let key = inner[key_abs..key_abs + key_end_off].trim();

        let after_key_close = key_abs + key_end_off + ARG_KEY_CLOSE.len();

        // Find matching <arg_value>...</arg_value>.
        let Some(val_start_off) = inner[after_key_close..].find(ARG_VAL_OPEN) else {
            break;
        };
        let val_abs = after_key_close + val_start_off + ARG_VAL_OPEN.len();

        let Some(val_end_off) = inner[val_abs..].find(ARG_VAL_CLOSE) else {
            break;
        };
        let value = inner[val_abs..val_abs + val_end_off].trim();

        if !key.is_empty() {
            // Try to parse as JSON first (for objects/arrays/numbers/booleans),
            // fall back to string.
            let json_val = serde_json::from_str::<serde_json::Value>(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
            args.insert(key.to_string(), json_val);
        }

        cursor = val_abs + val_end_off + ARG_VAL_CLOSE.len();
    }

    Ok(ParsedToolCall {
        name: name.to_string(),
        arguments: serde_json::Value::Object(args),
    })
}

/// Extract a quoted attribute value from a tag fragment.
///
/// e.g. `extract_quoted_attr("name=\"GetWeather\" string=\"true\">", "name")`
/// returns `Some("GetWeather")`.
fn extract_quoted_attr<'a>(tag_fragment: &'a str, attr: &str) -> Option<&'a str> {
    let prefix = format!("{attr}=\"");
    let start = tag_fragment.find(&prefix)? + prefix.len();
    let end = tag_fragment[start..].find('"')? + start;
    Some(&tag_fragment[start..end])
}

/// Strip surrounding single or double quotes from a string.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Extract the first double-quoted substring from text.
///
/// e.g. `extract_first_quoted("\"path\" string=\"true\"")` -> `Some("path")`
///
/// Useful when a tag prefix already consumed `name=` and the remaining text
/// starts with the quoted value, possibly followed by extra attributes.
fn extract_first_quoted(s: &str) -> Option<&str> {
    let start = s.find('"')? + 1;
    let end = s[start..].find('"')? + start;
    Some(&s[start..end])
}

/// Sanitize raw newline characters inside JSON string literals.
///
/// LLMs sometimes produce JSON arguments with unescaped newlines inside string
/// values (e.g. multi-line git commit messages). JSON requires newlines to be
/// escaped as `\n` within strings, so `serde_json::from_str` rejects them.
///
/// This function walks through the input and, whenever it is inside a
/// JSON string literal (between unescaped double quotes), replaces raw `\n`
/// and `\r` with their JSON escape sequences `\\n` and `\\r`.
fn sanitize_json_newlines(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_string = false;
    let mut prev_backslash = false;

    for ch in input.chars() {
        if in_string {
            match ch {
                '"' if !prev_backslash => {
                    in_string = false;
                    result.push(ch);
                }
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                _ => result.push(ch),
            }
            prev_backslash = ch == '\\' && !prev_backslash;
        } else {
            if ch == '"' {
                in_string = true;
            }
            result.push(ch);
            prev_backslash = false;
        }
    }

    result
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
{"name": "FileRead", "arguments": {"path": "/src/main.rs"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert!(result.text.contains("I need to read that file."));
        assert!(!result.text.contains("tool_call"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let input = r#"Let me check both files.

<tool_call>
{"name": "FileRead", "arguments": {"path": "/src/lib.rs"}}
</tool_call>

<tool_call>
{"name": "FileRead", "arguments": {"path": "/src/main.rs"}}
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
{"name": "FileRead", "arguments": {"path": "/a.rs"}}
</tool_call>

Middle text.

<tool_call>
{"name": "ShellExec", "arguments": {"command": "ls"}}
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
{"name": "ToolSearch"}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ToolSearch");
        assert!(result.tool_calls[0].arguments.is_object());
        assert!(result.tool_calls[0]
            .arguments
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_parse_arguments_not_object() {
        let input = r#"<tool_call>
{"name": "Test", "arguments": [1, 2, 3]}
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
{"name": "ShellExec", "arguments": {"command": "echo '<div>hello</div>'"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        assert_eq!(
            result.tool_calls[0].arguments["command"],
            "echo '<div>hello</div>'"
        );
    }

    #[test]
    fn test_parse_whitespace_around_json() {
        let input = "<tool_call>   \n  {\"name\": \"Test\", \"arguments\": {}}  \n  </tool_call>";
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Test");
    }

    #[test]
    fn test_format_tool_result_success() {
        let content = serde_json::json!({"data": "hello"});
        let formatted = format_tool_result("FileRead", true, &content);
        assert!(formatted.starts_with("<tool_result name=\"FileRead\" success=\"true\">"));
        assert!(formatted.ends_with("</tool_result>"));
        assert!(formatted.contains("hello"));
    }

    #[test]
    fn test_format_tool_result_error() {
        let content = serde_json::json!({"error": "file not found"});
        let formatted = format_tool_result("FileRead", false, &content);
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
        let input = r#"<tool_call>{"name": "Test", "arguments": {"key": "value"}}</tool_call>"#;
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].arguments["key"], "value");
    }

    #[test]
    fn test_parse_text_cleanup_no_extra_whitespace() {
        let input =
            "Before.\n\n<tool_call>\n{\"name\": \"t\", \"arguments\": {}}\n</tool_call>\n\nAfter.";
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
<name>FileRead</name>
<arguments>{"path": "/src/main.rs"}</arguments>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert!(result.text.contains("I need to read that file."));
        assert!(!result.text.contains("tool_call"));
    }

    #[test]
    fn test_parse_xml_multiple_tool_calls() {
        let input = r#"Let me search for tools.

<tool_call>
<name>ToolSearch</name>
<arguments>{"query": "list directory"}</arguments>
</tool_call>

<tool_call>
<name>ToolSearch</name>
<arguments>{"category": "file"}</arguments>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "ToolSearch");
        assert_eq!(result.tool_calls[0].arguments["query"], "list directory");
        assert_eq!(result.tool_calls[1].arguments["category"], "file");
    }

    #[test]
    fn test_parse_xml_without_arguments() {
        let input = r"<tool_call>
<name>ToolSearch</name>
</tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ToolSearch");
        assert!(result.tool_calls[0].arguments.is_object());
        assert!(result.tool_calls[0]
            .arguments
            .as_object()
            .unwrap()
            .is_empty());
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
        let input = "<tool_call>\n  <name>  FileRead  </name>\n  <arguments>  {\"path\": \"/a\"}  </arguments>\n</tool_call>";
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[0].arguments["path"], "/a");
    }

    #[test]
    fn test_parse_mixed_xml_and_json_formats() {
        let input = r#"<tool_call>
<name>FileRead</name>
<arguments>{"path": "/a.rs"}</arguments>
</tool_call>

<tool_call>
{"name": "ShellExec", "arguments": {"command": "ls"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[1].name, "ShellExec");
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

    // -----------------------------------------------------------------------
    // Function-attribute format tests (Llama/Qwen compatibility)
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_function_format_with_parameters() {
        let input = r#"<tool_call>
<function=Browser>
<action>navigate</action>
<parameter=url>https://www.google.com/search?q=weather</parameter>
</function>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Browser");
        assert_eq!(
            result.tool_calls[0].arguments["url"],
            "https://www.google.com/search?q=weather"
        );
        assert_eq!(result.tool_calls[0].arguments["action"], "navigate");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_function_format_multiple_params() {
        let input = r#"<tool_call>
<function=FileWrite>
<parameter=path>/src/main.rs</parameter>
<parameter=content>fn main() {}</parameter>
</function>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileWrite");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert_eq!(result.tool_calls[0].arguments["content"], "fn main() {}");
    }

    #[test]
    fn test_parse_function_format_with_json_body() {
        let input = r#"<tool_call>
<function=ShellExec>{"command": "ls -la"}</function>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        assert_eq!(result.tool_calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn test_parse_function_format_no_closing_function_tag() {
        let input = r#"<tool_call>
<function=Browser>
<parameter=url>https://example.com</parameter>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Browser");
        assert_eq!(result.tool_calls[0].arguments["url"], "https://example.com");
    }

    #[test]
    fn test_parse_function_format_empty_name_fails() {
        let input = r#"<tool_call>
<function=>
<parameter=url>https://example.com</parameter>
</function>
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_parse_mixed_all_three_formats() {
        let input = r#"<tool_call>
<name>FileRead</name>
<arguments>{"path": "/a.rs"}</arguments>
</tool_call>

<tool_call>
<function=Browser>
<parameter=url>https://example.com</parameter>
</function>
</tool_call>

<tool_call>
{"name": "ShellExec", "arguments": {"command": "ls"}}
</tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 3);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[1].name, "Browser");
        assert_eq!(result.tool_calls[2].name, "ShellExec");
        assert!(result.warnings.is_empty());
    }

    // -----------------------------------------------------------------------
    // Multiline JSON / raw newline tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_xml_multiline_git_commit_message() {
        // Reproduces: LLM returns a multiline git commit message inside
        // <arguments> JSON, with raw newline characters inside the string.
        let input = "<tool_call>\n<name>ShellExec</name>\n<arguments>{\"command\": \"git commit -m \\\"feat(gui): copy button\n\n- Add copyContent prop\n- Update ActionBar\\\"\"}</arguments>\n</tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(
            result.tool_calls.len(),
            1,
            "warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        let cmd = result.tool_calls[0].arguments["command"]
            .as_str()
            .expect("command should be a string");
        assert!(cmd.contains("feat(gui): copy button"));
        assert!(cmd.contains("Add copyContent prop"));
    }

    #[test]
    fn test_parse_json_multiline_git_commit_message() {
        // Same scenario but using JSON format inside <tool_call>.
        let input = "<tool_call>\n{\"name\": \"ShellExec\", \"arguments\": {\"command\": \"git commit -m \\\"fix: improve handling\n\n- Track composition end time\n- Replace keyCode check\\\"\"}}\n</tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(
            result.tool_calls.len(),
            1,
            "warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        let cmd = result.tool_calls[0].arguments["command"]
            .as_str()
            .expect("command should be a string");
        assert!(cmd.contains("fix: improve handling"));
    }

    #[test]
    fn test_sanitize_json_newlines_basic() {
        let input = r#"{"key": "line1
line2"}"#;
        let sanitized = sanitize_json_newlines(input);
        assert!(serde_json::from_str::<serde_json::Value>(&sanitized).is_ok());
        let val: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(val["key"].as_str().unwrap(), "line1\nline2");
    }

    #[test]
    fn test_sanitize_json_newlines_preserves_escaped() {
        // Already-escaped \n should remain as-is.
        let input = r#"{"key": "line1\nline2"}"#;
        let sanitized = sanitize_json_newlines(input);
        assert_eq!(sanitized, input);
        let val: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(val["key"].as_str().unwrap(), "line1\nline2");
    }

    #[test]
    fn test_sanitize_json_newlines_outside_strings_preserved() {
        // Newlines outside JSON strings (structural whitespace) should be kept.
        let input = "{\n  \"key\": \"value\"\n}";
        let sanitized = sanitize_json_newlines(input);
        assert_eq!(sanitized, input);
    }

    // -----------------------------------------------------------------------
    // DeepSeek V3.2 DSML format tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_dsml_single_invoke() {
        let input = "Let me check the weather.\n\n\
            <\u{ff5c}DSML\u{ff5c}function_calls>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"GetWeather\">\n\
            <\u{ff5c}DSML\u{ff5c}parameter name=\"location\" string=\"true\">\
            Beijing</\u{ff5c}DSML\u{ff5c}parameter>\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            </\u{ff5c}DSML\u{ff5c}function_calls>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "GetWeather");
        assert_eq!(result.tool_calls[0].arguments["location"], "Beijing");
        assert!(result.text.contains("Let me check the weather."));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_dsml_multiple_invokes() {
        let input = "\
            <\u{ff5c}DSML\u{ff5c}function_calls>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"GetWeather\">\n\
            <\u{ff5c}DSML\u{ff5c}parameter name=\"location\" string=\"true\">\
            Hangzhou</\u{ff5c}DSML\u{ff5c}parameter>\n\
            <\u{ff5c}DSML\u{ff5c}parameter name=\"date\" string=\"true\">\
            2024-01-16</\u{ff5c}DSML\u{ff5c}parameter>\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"GetWeather\">\n\
            <\u{ff5c}DSML\u{ff5c}parameter name=\"location\" string=\"true\">\
            Beijing</\u{ff5c}DSML\u{ff5c}parameter>\n\
            <\u{ff5c}DSML\u{ff5c}parameter name=\"date\" string=\"true\">\
            2024-01-16</\u{ff5c}DSML\u{ff5c}parameter>\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            </\u{ff5c}DSML\u{ff5c}function_calls>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].arguments["location"], "Hangzhou");
        assert_eq!(result.tool_calls[1].arguments["location"], "Beijing");
        assert_eq!(result.tool_calls[0].arguments["date"], "2024-01-16");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_dsml_no_params() {
        let input = "\
            <\u{ff5c}DSML\u{ff5c}function_calls>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"FileList\">\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            </\u{ff5c}DSML\u{ff5c}function_calls>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileList");
        assert!(result.tool_calls[0]
            .arguments
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_parse_dsml_unclosed_envelope() {
        // Missing close tag -- treated as text.
        let input = "text <\u{ff5c}DSML\u{ff5c}function_calls> some content here";
        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(result.text.contains("some content here"));
    }

    #[test]
    fn test_parse_dsml_with_surrounding_text() {
        // Reproduces the user's example: LLM outputs Chinese text then DSML.
        let input = "\
            I will create an agent for you. Let me check existing files.\n\n\
            <\u{ff5c}DSML\u{ff5c}function_calls>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"FileList\">\n\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            </\u{ff5c}DSML\u{ff5c}function_calls>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "FileList");
        assert!(result.text.contains("I will create an agent for you."));
    }

    // -----------------------------------------------------------------------
    // MiniMax M2 format tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_minimax_single_invoke() {
        let input = "Searching for info.\n\n\
            <minimax:tool_call>\n\
            <invoke name=\"WebSearch\">\n\
            <parameter name=\"query\">rust programming</parameter>\n\
            </invoke>\n\
            </minimax:tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "WebSearch");
        assert_eq!(result.tool_calls[0].arguments["query"], "rust programming");
        assert!(result.text.contains("Searching for info."));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_minimax_multiple_invokes() {
        let input = "\
            <minimax:tool_call>\n\
            <invoke name=\"FileRead\">\n\
            <parameter name=\"path\">/src/main.rs</parameter>\n\
            </invoke>\n\
            <invoke name=\"FileRead\">\n\
            <parameter name=\"path\">/src/lib.rs</parameter>\n\
            </invoke>\n\
            </minimax:tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert_eq!(result.tool_calls[1].arguments["path"], "/src/lib.rs");
    }

    #[test]
    fn test_parse_minimax_quoted_names() {
        let input = "\
            <minimax:tool_call>\n\
            <invoke name=\"ShellExec\">\n\
            <parameter name=\"command\">ls -la</parameter>\n\
            <parameter name=\"timeout\">30</parameter>\n\
            </invoke>\n\
            </minimax:tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        assert_eq!(result.tool_calls[0].arguments["command"], "ls -la");
        assert_eq!(result.tool_calls[0].arguments["timeout"], "30");
    }

    // -----------------------------------------------------------------------
    // GLM-4 / GLM-4.7 arg_key/arg_value format tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_glm_arg_format_with_args() {
        let input = "\
            <tool_call>\n\
            GetWeather\n\
            <arg_key>location</arg_key>\n\
            <arg_value>Beijing</arg_value>\n\
            <arg_key>unit</arg_key>\n\
            <arg_value>celsius</arg_value>\n\
            </tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "GetWeather");
        assert_eq!(result.tool_calls[0].arguments["location"], "Beijing");
        assert_eq!(result.tool_calls[0].arguments["unit"], "celsius");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_glm_arg_format_zero_args() {
        // GLM-4.7 zero-arg format: bare function name inside <tool_call>.
        let input = "<tool_call>ListFiles</tool_call>";
        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ListFiles");
        assert!(result.tool_calls[0]
            .arguments
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_parse_glm_arg_format_json_value() {
        // GLM format with a JSON-parseable value (number).
        let input = "\
            <tool_call>\n\
            Calculator\n\
            <arg_key>expression</arg_key>\n\
            <arg_value>42</arg_value>\n\
            </tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Calculator");
        // 42 is parsed as a JSON number.
        assert_eq!(result.tool_calls[0].arguments["expression"], 42);
    }

    #[test]
    fn test_parse_glm_inline_arg_key() {
        // GLM-4.7 style: function name directly before <arg_key> without newline.
        let input = "\
            <tool_call>GetWeather<arg_key>city</arg_key>\
            <arg_value>Tokyo</arg_value></tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "GetWeather");
        assert_eq!(result.tool_calls[0].arguments["city"], "Tokyo");
    }

    // -----------------------------------------------------------------------
    // Mixed envelope tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_mixed_envelopes() {
        // Different envelope types in the same output.
        let input = "\
            <tool_call>\n\
            {\"name\": \"FileRead\", \"arguments\": {\"path\": \"/a.rs\"}}\n\
            </tool_call>\n\n\
            <minimax:tool_call>\n\
            <invoke name=\"WebSearch\">\n\
            <parameter name=\"query\">test</parameter>\n\
            </invoke>\n\
            </minimax:tool_call>";

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[1].name, "WebSearch");
    }

    #[test]
    fn test_strip_tool_call_blocks_dsml() {
        let input = "Hello\n\
            <\u{ff5c}DSML\u{ff5c}function_calls>\n\
            <\u{ff5c}DSML\u{ff5c}invoke name=\"Test\">\n\
            </\u{ff5c}DSML\u{ff5c}invoke>\n\
            </\u{ff5c}DSML\u{ff5c}function_calls>\n\
            World";

        let stripped = strip_tool_call_blocks(input);
        assert!(!stripped.contains("DSML"));
        assert!(stripped.contains("Hello"));
        assert!(stripped.contains("World"));
    }

    #[test]
    fn test_strip_tool_call_blocks_minimax() {
        let input = "Before\n\
            <minimax:tool_call>\n\
            <invoke name=\"Test\">\n\
            <parameter name=\"key\">val</parameter>\n\
            </invoke>\n\
            </minimax:tool_call>\n\
            After";

        let stripped = strip_tool_call_blocks(input);
        assert!(!stripped.contains("minimax"));
        assert!(stripped.contains("Before"));
        assert!(stripped.contains("After"));
    }

    #[test]
    fn test_parse_function_calls_envelope() {
        let input = r#"Here is the result.
<function_calls>
<invoke name="FileRead">
<parameter name="path" string="true">.</parameter>
</invoke>
</function_calls>"#;

        let result = parse_tool_calls(input);
        assert_eq!(
            result.tool_calls.len(),
            1,
            "warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[0].arguments["path"], ".");
        assert!(result.text.contains("Here is the result."));
        assert!(!result.text.contains("function_calls"));
    }

    #[test]
    fn test_parse_function_calls_multiple_invokes() {
        let input = r#"<function_calls>
<invoke name="FileRead">
<parameter name="path">/src/main.rs</parameter>
</invoke>
<invoke name="ShellExec">
<parameter name="command">ls -la</parameter>
</invoke>
</function_calls>"#;

        let result = parse_tool_calls(input);
        assert_eq!(
            result.tool_calls.len(),
            2,
            "warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.tool_calls[0].name, "FileRead");
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/main.rs");
        assert_eq!(result.tool_calls[1].name, "ShellExec");
        assert_eq!(result.tool_calls[1].arguments["command"], "ls -la");
    }

    #[test]
    fn test_strip_tool_call_blocks_function_calls() {
        let input = "Before\n\
            <function_calls>\n\
            <invoke name=\"Test\">\n\
            <parameter name=\"key\">val</parameter>\n\
            </invoke>\n\
            </function_calls>\n\
            After";

        let stripped = strip_tool_call_blocks(input);
        assert!(!stripped.contains("function_calls"));
        assert!(stripped.contains("Before"));
        assert!(stripped.contains("After"));
    }

    // -----------------------------------------------------------------------
    // Longcat Flash envelope tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_longcat_single_tool_call() {
        let input = r#"Let me search for that.

<longcat_tool_call>
{"name": "WebSearch", "arguments": {"query": "Rust async patterns"}}
</longcat_tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "WebSearch");
        assert_eq!(
            result.tool_calls[0].arguments["query"],
            "Rust async patterns"
        );
        assert!(result.text.contains("Let me search for that."));
        assert!(!result.text.contains("longcat_tool_call"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_longcat_multiple_tool_calls() {
        let input = r#"I'll read both files.

<longcat_tool_call>
{"name": "FileRead", "arguments": {"path": "/src/lib.rs"}}
</longcat_tool_call>

<longcat_tool_call>
{"name": "FileRead", "arguments": {"path": "/src/main.rs"}}
</longcat_tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].arguments["path"], "/src/lib.rs");
        assert_eq!(result.tool_calls[1].arguments["path"], "/src/main.rs");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_longcat_xml_nested_format() {
        let input = r#"<longcat_tool_call>
<name>ShellExec</name>
<arguments>{"command": "cargo build"}</arguments>
</longcat_tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "ShellExec");
        assert_eq!(result.tool_calls[0].arguments["command"], "cargo build");
    }

    #[test]
    fn test_strip_tool_call_blocks_longcat() {
        let input = "Before\n\
            <longcat_tool_call>\n\
            {\"name\": \"Test\", \"arguments\": {}}\n\
            </longcat_tool_call>\n\
            After";

        let stripped = strip_tool_call_blocks(input);
        assert!(!stripped.contains("longcat_tool_call"));
        assert!(stripped.contains("Before"));
        assert!(stripped.contains("After"));
    }

    #[test]
    fn test_parse_longcat_mixed_with_standard() {
        let input = r#"<tool_call>
{"name": "First", "arguments": {}}
</tool_call>

<longcat_tool_call>
{"name": "Second", "arguments": {"key": "val"}}
</longcat_tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "First");
        assert_eq!(result.tool_calls[1].name, "Second");
        assert_eq!(result.tool_calls[1].arguments["key"], "val");
    }

    // -----------------------------------------------------------------------
    // MiniMax 2.5 missing open tag patch tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_patch_minimax_single_missing_open_tag() {
        let input = r#"<think>thinking...</think>
<invoke name="Glob"> <parameter name="query">**/glob.rs</parameter> </invoke> </minimax:tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Glob");
        assert_eq!(result.tool_calls[0].arguments["query"], "**/glob.rs");
    }

    #[test]
    fn test_patch_minimax_multiple_missing_open_tags() {
        let input = r#"<think>thinking...</think>
<invoke name="Glob"> <parameter name="query">**/glob.rs</parameter> </invoke> </minimax:tool_call>

Some text in between.

<invoke name="FileRead"> <parameter name="path">/src/main.rs</parameter> </invoke> </minimax:tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "Glob");
        assert_eq!(result.tool_calls[0].arguments["query"], "**/glob.rs");
        assert_eq!(result.tool_calls[1].name, "FileRead");
        assert_eq!(result.tool_calls[1].arguments["path"], "/src/main.rs");
        assert!(result.text.contains("Some text in between."));
    }

    #[test]
    fn test_patch_minimax_mixed_present_and_missing() {
        // First tool call has proper tags, second is missing the open tag.
        let input = r#"<minimax:tool_call>
<invoke name="Glob"> <parameter name="query">*.rs</parameter> </invoke>
</minimax:tool_call>

<invoke name="FileRead"> <parameter name="path">/a.rs</parameter> </invoke> </minimax:tool_call>"#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].name, "Glob");
        assert_eq!(result.tool_calls[1].name, "FileRead");
    }

    #[test]
    fn test_patch_minimax_no_close_tag_no_patch() {
        // No close tag at all -- should not be affected.
        let input = "<invoke name=\"Glob\"> <parameter name=\"query\">*.rs</parameter> </invoke>";
        let result = parse_tool_calls(input);
        assert!(result.tool_calls.is_empty());
        assert!(result.text.contains("<invoke"));
    }

    #[test]
    fn test_patch_minimax_preserves_surrounding_text() {
        let input = r#"Before text.

<invoke name="Glob"> <parameter name="query">*.rs</parameter> </invoke> </minimax:tool_call>

After text."#;

        let result = parse_tool_calls(input);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Glob");
        assert!(result.text.contains("Before text."));
        assert!(result.text.contains("After text."));
    }
}
