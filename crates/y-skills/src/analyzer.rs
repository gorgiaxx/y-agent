//! Content analyzer: LLM-assisted skill content analysis.
//!
//! Uses a single LLM call per skill with structured JSON output
//! to analyze purpose, capabilities, embedded tools, and security flags.

use serde::{Deserialize, Serialize};

/// Analysis report produced by the content analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// Detected purpose of the skill.
    pub purpose: String,
    /// Classification hint (used by the classifier).
    pub classification_hint: String,
    /// List of capabilities the skill provides.
    pub capabilities: Vec<String>,
    /// Embedded tool references found in the content.
    pub embedded_tools: Vec<EmbeddedTool>,
    /// Embedded script blocks found in the content.
    pub embedded_scripts: Vec<EmbeddedScript>,
    /// Quality issues detected.
    pub quality_issues: Vec<String>,
    /// Estimated token count for the content.
    pub token_estimate: u32,
    /// Security flags detected during analysis.
    pub security_flags: SecurityFlags,
}

/// An embedded tool found in skill content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedTool {
    /// Tool name or identifier.
    pub name: String,
    /// Type of tool (e.g., `api_endpoint`, `cli_command`, `function`).
    pub tool_type: String,
    /// Description of what this tool does.
    pub description: String,
}

/// An embedded script block found in skill content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedScript {
    /// Language of the script block.
    pub language: String,
    /// The script content.
    pub content: String,
    /// Line number where the script starts.
    pub line_start: usize,
}

/// Security flags detected during content analysis.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityFlags {
    /// Content contains patterns suggesting external API calls.
    #[serde(default)]
    pub has_external_calls: bool,
    /// Content contains file system operation patterns.
    #[serde(default)]
    pub has_file_operations: bool,
    /// Content contains executable code blocks.
    #[serde(default)]
    pub has_code_execution: bool,
    /// Content contains patterns suggesting data exfiltration.
    #[serde(default)]
    pub has_data_exfiltration: bool,
    /// Content contains delegation to other agents/skills.
    #[serde(default)]
    pub has_delegation: bool,
}

/// Content analyzer using pattern matching (deterministic).
///
/// In production, this would be enhanced with LLM calls via `y-provider`.
/// For now, it uses rule-based heuristics for deterministic behavior.
#[derive(Debug)]
pub struct ContentAnalyzer;

#[allow(clippy::unused_self)]
impl ContentAnalyzer {
    /// Create a new content analyzer.
    pub fn new() -> Self {
        Self
    }

    /// Analyze skill content and produce an analysis report.
    pub fn analyze(&self, content: &str) -> AnalysisReport {
        let embedded_tools = self.detect_tools(content);
        let embedded_scripts = self.detect_scripts(content);
        let security_flags = self.detect_security_flags(content);
        let quality_issues = self.detect_quality_issues(content);
        let classification_hint = self.classify_hint(content, &embedded_tools, &embedded_scripts);
        let token_estimate = crate::manifest::estimate_tokens(content);

        AnalysisReport {
            purpose: self.extract_purpose(content),
            classification_hint,
            capabilities: self.extract_capabilities(content),
            embedded_tools,
            embedded_scripts,
            quality_issues,
            token_estimate,
            security_flags,
        }
    }

    fn extract_purpose(&self, content: &str) -> String {
        // Extract from first heading or first paragraph
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                return trimmed.trim_start_matches("# ").to_string();
            }
        }
        // First non-empty line as fallback
        content
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Unknown purpose")
            .trim()
            .to_string()
    }

    fn extract_capabilities(&self, content: &str) -> Vec<String> {
        let mut caps = Vec::new();
        let mut in_list = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().contains("capabilit")
                || trimmed.to_lowercase().contains("feature")
            {
                in_list = true;
                continue;
            }
            if in_list && trimmed.starts_with("- ") {
                caps.push(trimmed.trim_start_matches("- ").to_string());
            } else if in_list && !trimmed.is_empty() && !trimmed.starts_with("- ") {
                in_list = false;
            }
        }
        caps
    }

    fn detect_tools(&self, content: &str) -> Vec<EmbeddedTool> {
        let mut tools = Vec::new();
        let patterns = [
            ("curl ", "api_endpoint", "HTTP API call"),
            ("wget ", "api_endpoint", "HTTP download"),
            ("POST ", "api_endpoint", "HTTP POST endpoint"),
            ("GET ", "api_endpoint", "HTTP GET endpoint"),
        ];
        for line in content.lines() {
            let trimmed = line.trim();
            for (pattern, tool_type, desc) in &patterns {
                if trimmed.contains(pattern) {
                    tools.push(EmbeddedTool {
                        name: format!("detected_{tool_type}"),
                        tool_type: (*tool_type).to_string(),
                        description: (*desc).to_string(),
                    });
                    break;
                }
            }
        }
        tools
    }

    fn detect_scripts(&self, content: &str) -> Vec<EmbeddedScript> {
        let mut scripts = Vec::new();
        let mut in_block = false;
        let mut block_lang = String::new();
        let mut block_content = String::new();
        let mut block_start = 0;

        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") && !in_block {
                let lang = trimmed.trim_start_matches('`').trim().to_string();
                if matches!(
                    lang.as_str(),
                    "bash" | "sh" | "python" | "javascript" | "typescript" | "ruby" | "go"
                ) {
                    in_block = true;
                    block_lang = lang;
                    block_content.clear();
                    block_start = i + 1;
                }
            } else if trimmed == "```" && in_block {
                scripts.push(EmbeddedScript {
                    language: block_lang.clone(),
                    content: block_content.clone(),
                    line_start: block_start,
                });
                in_block = false;
            } else if in_block {
                if !block_content.is_empty() {
                    block_content.push('\n');
                }
                block_content.push_str(line);
            }
        }
        scripts
    }

    fn detect_security_flags(&self, content: &str) -> SecurityFlags {
        let lower = content.to_lowercase();
        SecurityFlags {
            has_external_calls: lower.contains("http://")
                || lower.contains("https://")
                || lower.contains("curl ")
                || lower.contains("api call"),
            has_file_operations: lower.contains("read file")
                || lower.contains("write file")
                || lower.contains("fs.")
                || lower.contains("open("),
            has_code_execution: lower.contains("exec(")
                || lower.contains("eval(")
                || lower.contains("subprocess")
                || lower.contains("system("),
            has_data_exfiltration: lower.contains("send data")
                || lower.contains("upload")
                || lower.contains("exfiltrat"),
            has_delegation: lower.contains("delegate")
                || lower.contains("sub-agent")
                || lower.contains("invoke agent"),
        }
    }

    fn detect_quality_issues(&self, content: &str) -> Vec<String> {
        let mut issues = Vec::new();
        let token_est = crate::manifest::estimate_tokens(content);

        if token_est > 5000 {
            issues.push(format!(
                "Content is very large ({token_est} estimated tokens), consider splitting"
            ));
        }
        if content.lines().count() < 5 {
            issues.push("Content is very short, may lack sufficient detail".to_string());
        }
        if !content.contains('#') {
            issues.push("No headings found, content may lack structure".to_string());
        }
        issues
    }

    fn classify_hint(
        &self,
        content: &str,
        tools: &[EmbeddedTool],
        scripts: &[EmbeddedScript],
    ) -> String {
        if !tools.is_empty() || !scripts.is_empty() {
            if content.to_lowercase().contains("reasoning")
                || content.to_lowercase().contains("think step by step")
            {
                return "hybrid".to_string();
            }
            if !tools.is_empty() {
                return "api_call".to_string();
            }
            return "tool_wrapper".to_string();
        }
        if content.to_lowercase().contains("agent") && content.to_lowercase().contains("delegat") {
            return "agent_behavior".to_string();
        }
        "llm_reasoning".to_string()
    }
}

impl Default for ContentAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S4-02: Content analyzer produces structured `AnalysisReport`.
    #[test]
    fn test_analyzer_produces_report() {
        let analyzer = ContentAnalyzer::new();
        let content = r"# Essay Writing Helper

## Capabilities
- Generate outlines
- Improve clarity
- Fix grammar

Write essays with clear structure and reasoning.
Think step by step about the argument.
";
        let report = analyzer.analyze(content);

        assert_eq!(report.purpose, "Essay Writing Helper");
        assert!(!report.capabilities.is_empty());
        assert!(report.token_estimate > 0);
        assert_eq!(report.classification_hint, "llm_reasoning");
    }

    /// Analyzer detects embedded scripts.
    #[test]
    fn test_analyzer_detects_scripts() {
        let analyzer = ContentAnalyzer::new();
        let content = r"# Deploy Helper
Use this script:
```bash
curl -X POST https://api.example.com/deploy
```
";
        let report = analyzer.analyze(content);

        assert!(!report.embedded_scripts.is_empty());
        assert_eq!(report.embedded_scripts[0].language, "bash");
        assert!(report.security_flags.has_external_calls);
    }

    /// Analyzer detects API tools.
    #[test]
    fn test_analyzer_detects_api_tools() {
        let analyzer = ContentAnalyzer::new();
        let content = "Use curl https://api.example.com to fetch data";
        let report = analyzer.analyze(content);

        assert!(!report.embedded_tools.is_empty());
        assert_eq!(report.classification_hint, "api_call");
    }
}
