//! Ingestion pipeline: multi-format input processing.
//!
//! Converts various input formats (TOML, Markdown, YAML, JSON, `PlainText`)
//! into `SkillManifest`. Includes format detection by extension and content
//! heuristics.

use std::path::Path;

use crate::config::SkillConfig;
use crate::error::SkillModuleError;
use crate::manifest::ManifestParser;
use y_core::skill::SkillManifest;

/// Supported ingestion formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionFormat {
    /// TOML skill manifest.
    Toml,
    /// Markdown document.
    Markdown,
    /// YAML document.
    Yaml,
    /// JSON document.
    Json,
    /// Plain text.
    PlainText,
    /// Directory containing skill files.
    Directory,
}

impl std::fmt::Display for IngestionFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Toml => "toml",
            Self::Markdown => "markdown",
            Self::Yaml => "yaml",
            Self::Json => "json",
            Self::PlainText => "plaintext",
            Self::Directory => "directory",
        };
        f.write_str(s)
    }
}

/// Detects the format of a skill source by extension and content heuristics.
#[derive(Debug)]
pub struct FormatDetector;

impl FormatDetector {
    /// Detect format from a file path (extension-based).
    pub fn from_path(path: &Path) -> IngestionFormat {
        if path.is_dir() {
            return IngestionFormat::Directory;
        }

        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "toml" => IngestionFormat::Toml,
            "md" | "markdown" => IngestionFormat::Markdown,
            "yaml" | "yml" => IngestionFormat::Yaml,
            "json" => IngestionFormat::Json,
            _ => IngestionFormat::PlainText,
        }
    }

    /// Detect format from content heuristics.
    pub fn from_content(content: &str) -> IngestionFormat {
        let trimmed = content.trim();

        // TOML: has key = value patterns or [section] headers
        if (trimmed.contains(" = ") || trimmed.contains(" = \""))
            && (trimmed.contains('[') && trimmed.contains(']'))
        {
            return IngestionFormat::Toml;
        }

        // JSON: starts with { or [
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
        {
            return IngestionFormat::Json;
        }

        // Markdown: has heading patterns
        if trimmed.starts_with("# ") || trimmed.contains("\n# ") || trimmed.contains("\n## ") {
            return IngestionFormat::Markdown;
        }

        // YAML: has key: value patterns
        if trimmed.contains(": ") && !trimmed.contains(" = ") {
            return IngestionFormat::Yaml;
        }

        IngestionFormat::PlainText
    }
}

impl Default for FormatDetector {
    fn default() -> Self {
        Self
    }
}

/// Processes various input formats into `SkillManifest`.
#[derive(Debug)]
pub struct IngestionPipeline {
    parser: ManifestParser,
}

impl IngestionPipeline {
    /// Create a new ingestion pipeline.
    pub fn new(config: SkillConfig) -> Self {
        Self {
            parser: ManifestParser::new(config),
        }
    }

    /// Ingest content in the specified format.
    pub fn ingest(
        &self,
        content: &str,
        format: IngestionFormat,
    ) -> Result<SkillManifest, SkillModuleError> {
        match format {
            IngestionFormat::Toml => self.parser.parse(content),
            IngestionFormat::Markdown
            | IngestionFormat::Yaml
            | IngestionFormat::Json
            | IngestionFormat::PlainText => Err(SkillModuleError::IngestionError {
                message: format!(
                    "format '{format}' requires LLM-assisted transformation (not yet wired)"
                ),
            }),
            IngestionFormat::Directory => Err(SkillModuleError::IngestionError {
                message: "directory ingestion requires filesystem scanning".to_string(),
            }),
        }
    }

    /// Auto-detect format and ingest.
    pub fn ingest_auto(&self, content: &str) -> Result<SkillManifest, SkillModuleError> {
        let format = FormatDetector::from_content(content);
        self.ingest(content, format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S4-01: Format detector identifies .md, .yaml, .json, .toml, .txt, dir.
    #[test]
    fn test_format_detector_by_extension() {
        assert_eq!(
            FormatDetector::from_path(Path::new("skill.toml")),
            IngestionFormat::Toml
        );
        assert_eq!(
            FormatDetector::from_path(Path::new("README.md")),
            IngestionFormat::Markdown
        );
        assert_eq!(
            FormatDetector::from_path(Path::new("skill.yaml")),
            IngestionFormat::Yaml
        );
        assert_eq!(
            FormatDetector::from_path(Path::new("skill.yml")),
            IngestionFormat::Yaml
        );
        assert_eq!(
            FormatDetector::from_path(Path::new("manifest.json")),
            IngestionFormat::Json
        );
        assert_eq!(
            FormatDetector::from_path(Path::new("notes.txt")),
            IngestionFormat::PlainText
        );
    }

    /// Content-based detection works.
    #[test]
    fn test_format_detector_by_content() {
        // Markdown
        assert_eq!(
            FormatDetector::from_content("# Heading\nSome content"),
            IngestionFormat::Markdown
        );

        // JSON
        assert_eq!(
            FormatDetector::from_content("{\"name\": \"test\"}"),
            IngestionFormat::Json
        );

        // TOML
        assert_eq!(
            FormatDetector::from_content("[skill]\nname = \"test\""),
            IngestionFormat::Toml
        );

        // PlainText fallback
        assert_eq!(
            FormatDetector::from_content("Just some plain text here"),
            IngestionFormat::PlainText
        );
    }

    /// Directory detection.
    #[test]
    fn test_format_detector_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            FormatDetector::from_path(tmp.path()),
            IngestionFormat::Directory
        );
    }
}
