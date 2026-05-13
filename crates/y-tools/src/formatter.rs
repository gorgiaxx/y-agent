//! Tool result formatter for LLM consumption.
//!
//! Transforms raw tool execution results into structured, token-efficient
//! formats suitable for LLM context. Supports multiple output formats
//! and content truncation for large outputs.
//!
//! Design reference: tools-design.md §Result Processing

use serde_json::Value;

/// Result format options for formatting tool output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultFormat {
    /// Raw text output (pass-through).
    Raw,
    /// JSON-formatted output with pretty-printing.
    Json,
    /// Markdown-formatted output with code blocks.
    Markdown,
    /// Compact single-line format for minimal token usage.
    Compact,
}

/// Configuration for the result formatter.
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    /// Maximum character length for tool output before truncation.
    pub max_output_chars: usize,
    /// Truncation suffix appended when output is cut.
    pub truncation_suffix: String,
    /// Default format when none is specified.
    pub default_format: ResultFormat,
    /// Whether to include metadata (execution time, etc.) in output.
    pub include_metadata: bool,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            max_output_chars: 8000,
            truncation_suffix: "\n... [output truncated]".to_string(),
            default_format: ResultFormat::Raw,
            include_metadata: false,
        }
    }
}

/// Result of formatting a tool output.
#[derive(Debug, Clone)]
pub struct FormattedResult {
    /// The formatted output text.
    pub content: String,
    /// Whether the output was truncated.
    pub truncated: bool,
    /// Original character count before formatting.
    pub original_chars: usize,
    /// Final character count after formatting.
    pub final_chars: usize,
}

/// Tool result formatter.
#[derive(Debug, Clone)]
pub struct ResultFormatter {
    config: FormatterConfig,
}

impl ResultFormatter {
    /// Create a new formatter with the given config.
    pub fn new(config: FormatterConfig) -> Self {
        Self { config }
    }

    /// Create a formatter with default settings.
    pub fn with_defaults() -> Self {
        Self::new(FormatterConfig::default())
    }

    /// Format a tool result string.
    pub fn format(&self, output: &str, format: Option<&ResultFormat>) -> FormattedResult {
        let format = format.unwrap_or(&self.config.default_format);
        let original_chars = output.len();

        let formatted = match format {
            ResultFormat::Raw => output.to_string(),
            ResultFormat::Json => Self::format_json(output),
            ResultFormat::Markdown => Self::format_markdown(output),
            ResultFormat::Compact => Self::format_compact(output),
        };

        let (content, truncated) = self.truncate_if_needed(&formatted);

        FormattedResult {
            final_chars: content.len(),
            content,
            truncated,
            original_chars,
        }
    }

    /// Format output as pretty-printed JSON.
    fn format_json(output: &str) -> String {
        match serde_json::from_str::<Value>(output) {
            Ok(value) => {
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| output.to_string())
            }
            Err(_) => output.to_string(), // Not valid JSON, return as-is.
        }
    }

    /// Format output as a Markdown code block.
    fn format_markdown(output: &str) -> String {
        // Detect if the output is JSON, and use appropriate language tag.
        let lang = if serde_json::from_str::<Value>(output).is_ok() {
            "json"
        } else {
            ""
        };
        format!("```{lang}\n{output}\n```")
    }

    /// Format output in compact form: collapse whitespace, single line.
    fn format_compact(output: &str) -> String {
        output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Truncate output if it exceeds `max_output_chars`.
    fn truncate_if_needed(&self, output: &str) -> (String, bool) {
        let char_count = output.chars().count();
        if char_count <= self.config.max_output_chars {
            (output.to_string(), false)
        } else {
            let target =
                self.config.max_output_chars - self.config.truncation_suffix.chars().count();
            let byte_offset = output
                .char_indices()
                .nth(target)
                .map_or(output.len(), |(i, _)| i);
            let mut result = output[..byte_offset].to_string();
            result.push_str(&self.config.truncation_suffix);
            (result, true)
        }
    }
}

impl Default for ResultFormatter {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_raw() {
        let formatter = ResultFormatter::with_defaults();
        let result = formatter.format("hello world", Some(&ResultFormat::Raw));
        assert_eq!(result.content, "hello world");
        assert!(!result.truncated);
    }

    #[test]
    fn test_format_json_valid() {
        let formatter = ResultFormatter::with_defaults();
        let input = r#"{"name":"Alice","age":30}"#;
        let result = formatter.format(input, Some(&ResultFormat::Json));
        assert!(result.content.contains("\"name\": \"Alice\""));
        assert!(result.content.contains('\n')); // Pretty-printed.
    }

    #[test]
    fn test_format_json_invalid() {
        let formatter = ResultFormatter::with_defaults();
        let input = "not json at all";
        let result = formatter.format(input, Some(&ResultFormat::Json));
        assert_eq!(result.content, "not json at all"); // Pass-through.
    }

    #[test]
    fn test_format_markdown() {
        let formatter = ResultFormatter::with_defaults();
        let result = formatter.format("some output", Some(&ResultFormat::Markdown));
        assert!(result.content.starts_with("```"));
        assert!(result.content.ends_with("```"));
        assert!(result.content.contains("some output"));
    }

    #[test]
    fn test_format_markdown_json() {
        let formatter = ResultFormatter::with_defaults();
        let input = r#"{"key": "value"}"#;
        let result = formatter.format(input, Some(&ResultFormat::Markdown));
        assert!(result.content.starts_with("```json"));
    }

    #[test]
    fn test_format_compact() {
        let formatter = ResultFormatter::with_defaults();
        let input = "  line one  \n  line two  \n\n  line three  ";
        let result = formatter.format(input, Some(&ResultFormat::Compact));
        assert_eq!(result.content, "line one line two line three");
    }

    #[test]
    fn test_truncation() {
        let config = FormatterConfig {
            max_output_chars: 20,
            truncation_suffix: "...".to_string(),
            ..Default::default()
        };
        let formatter = ResultFormatter::new(config);

        let input = "a".repeat(50);
        let result = formatter.format(&input, Some(&ResultFormat::Raw));
        assert!(result.truncated);
        assert_eq!(result.final_chars, 20);
        assert!(result.content.ends_with("..."));
    }

    #[test]
    fn test_no_truncation_within_limit() {
        let config = FormatterConfig {
            max_output_chars: 100,
            ..Default::default()
        };
        let formatter = ResultFormatter::new(config);

        let result = formatter.format("short output", Some(&ResultFormat::Raw));
        assert!(!result.truncated);
        assert_eq!(result.original_chars, 12);
        assert_eq!(result.final_chars, 12);
    }

    #[test]
    fn test_default_format() {
        let config = FormatterConfig {
            default_format: ResultFormat::Compact,
            ..Default::default()
        };
        let formatter = ResultFormatter::new(config);

        let input = "  line one  \n  line two  ";
        let result = formatter.format(input, None); // Uses default.
        assert_eq!(result.content, "line one line two");
    }

    #[test]
    fn test_empty_input() {
        let formatter = ResultFormatter::with_defaults();
        let result = formatter.format("", Some(&ResultFormat::Raw));
        assert_eq!(result.content, "");
        assert!(!result.truncated);
    }
}
