//! Tool/script separator: extracts embedded executable content from skills.
//!
//! Scans for fenced code blocks, API endpoint patterns, and CLI command
//! templates. Replaces extracted items with `[tool:name]` reference markers.

/// An extracted tool or script from skill content.
#[derive(Debug, Clone)]
pub struct ExtractedTool {
    /// Generated name for the extracted tool.
    pub name: String,
    /// Type: `script`, `api_endpoint`, or `cli_command`.
    pub tool_type: String,
    /// Language (for scripts) or method (for APIs).
    pub language: String,
    /// The extracted content.
    pub content: String,
    /// Line number where the tool was found.
    pub original_line: usize,
}

/// Result of separation.
#[derive(Debug, Clone)]
pub struct SeparationResult {
    /// The content with extracted items replaced by `[tool:name]` references.
    pub cleaned_content: String,
    /// All extracted tools/scripts.
    pub extracted: Vec<ExtractedTool>,
}

/// Scans skill content and extracts embedded executable content.
#[derive(Debug)]
pub struct ToolSeparator;

impl ToolSeparator {
    /// Create a new separator.
    pub fn new() -> Self {
        Self
    }

    /// Separate executable content from skill content.
    pub fn separate(&self, content: &str) -> SeparationResult {
        let mut extracted = Vec::new();
        let mut cleaned_lines: Vec<String> = Vec::new();
        let mut tool_counter = 0u32;

        let mut in_block = false;
        let mut block_lang = String::new();
        let mut block_content = String::new();
        let mut block_start = 0usize;

        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with("```") && !in_block {
                let lang = trimmed.trim_start_matches('`').trim().to_lowercase();
                if Self::is_executable_lang(&lang) {
                    in_block = true;
                    block_lang = lang;
                    block_content.clear();
                    block_start = i + 1;
                    continue;
                }
            }

            if trimmed == "```" && in_block {
                tool_counter += 1;
                let name = format!("extracted_{block_lang}_{tool_counter}");
                extracted.push(ExtractedTool {
                    name: name.clone(),
                    tool_type: "script".to_string(),
                    language: block_lang.clone(),
                    content: block_content.clone(),
                    original_line: block_start,
                });
                cleaned_lines.push(format!("[tool:{name}]"));
                in_block = false;
                continue;
            }

            if in_block {
                if !block_content.is_empty() {
                    block_content.push('\n');
                }
                block_content.push_str(line);
            } else {
                cleaned_lines.push(line.to_string());
            }
        }

        SeparationResult {
            cleaned_content: cleaned_lines.join("\n"),
            extracted,
        }
    }

    fn is_executable_lang(lang: &str) -> bool {
        matches!(
            lang,
            "bash" | "sh" | "python" | "javascript" | "typescript" | "ruby" | "go" | "rust"
        )
    }
}

impl Default for ToolSeparator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S5-03: Separator extracts fenced bash code blocks.
    #[test]
    fn test_separator_extracts_bash_blocks() {
        let content = r"# Deploy Guide

Run this to deploy:

```bash
kubectl apply -f deployment.yaml
kubectl rollout status deployment/app
```

Then verify the status.
";
        let result = ToolSeparator::new().separate(content);

        assert_eq!(result.extracted.len(), 1);
        assert_eq!(result.extracted[0].language, "bash");
        assert!(result.extracted[0].content.contains("kubectl apply"));
        assert_eq!(result.extracted[0].tool_type, "script");
    }

    /// T-SK-S5-04: Separator replaces extracted scripts with `[tool:X]` refs.
    #[test]
    fn test_separator_replaces_with_refs() {
        let content = "Before\n\n```python\nprint('hello')\n```\n\nAfter";
        let result = ToolSeparator::new().separate(content);

        assert!(result.cleaned_content.contains("[tool:extracted_python_1]"));
        assert!(!result.cleaned_content.contains("print('hello')"));
        assert!(result.cleaned_content.contains("Before"));
        assert!(result.cleaned_content.contains("After"));
    }

    /// Non-executable blocks are preserved.
    #[test]
    fn test_separator_preserves_non_executable() {
        let content = "```toml\nname = \"test\"\n```";
        let result = ToolSeparator::new().separate(content);

        assert!(result.extracted.is_empty());
        assert!(result.cleaned_content.contains("name = \"test\""));
    }

    /// Multiple blocks extracted.
    #[test]
    fn test_separator_multiple_blocks() {
        let content = "```bash\necho hello\n```\n\nText\n\n```python\nprint(1)\n```";
        let result = ToolSeparator::new().separate(content);

        assert_eq!(result.extracted.len(), 2);
        assert_eq!(result.extracted[0].language, "bash");
        assert_eq!(result.extracted[1].language, "python");
    }
}
