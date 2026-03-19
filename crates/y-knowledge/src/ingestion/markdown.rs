//! Markdown file source connector.
//!
//! Reads Markdown files and extracts heading structure for title detection.

use super::{RawDocument, SourceConnector};
use crate::error::KnowledgeError;
use crate::models::SourceType;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// Connector for Markdown files.
#[derive(Debug, Default)]
pub struct MarkdownConnector;

impl MarkdownConnector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SourceConnector for MarkdownConnector {
    async fn fetch(&self, uri: &str) -> Result<RawDocument, KnowledgeError> {
        let (content, detected_encoding) = super::encoding::read_file_as_utf8(uri).await?;

        if detected_encoding != "UTF-8" {
            tracing::info!(
                uri,
                encoding = detected_encoding,
                "markdown file was auto-converted from {detected_encoding} to UTF-8"
            );
        }

        let content_hash = hex_sha256(&content);

        // Extract title from first H1 heading, falling back to filename.
        let title = extract_markdown_title(&content).unwrap_or_else(|| {
            std::path::Path::new(uri)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

        Ok(RawDocument {
            content,
            uri: uri.to_string(),
            title,
            content_hash,
            source_type: SourceType::File,
        })
    }

    fn source_type(&self) -> SourceType {
        SourceType::File
    }
}

/// Extract the title from a Markdown document.
///
/// Looks for the first `# Heading` line and returns the heading text.
fn extract_markdown_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            let title = heading.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

/// Compute hex-encoded SHA-256 hash.
fn hex_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_markdown_connector_reads_file() {
        let dir = std::env::temp_dir().join("y-knowledge-test-md");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let file_path = dir.join("guide.md");
        tokio::fs::write(
            &file_path,
            "# Getting Started\n\nThis is a guide to Rust.\n\n## Installation\n\nInstall rustup.",
        )
        .await
        .unwrap();

        let connector = MarkdownConnector::new();
        let doc = connector
            .fetch(file_path.to_str().unwrap())
            .await
            .expect("should read markdown");

        assert_eq!(doc.title, "Getting Started");
        assert!(doc.content.contains("## Installation"));
        assert!(!doc.content_hash.is_empty());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_markdown_connector_no_heading() {
        let dir = std::env::temp_dir().join("y-knowledge-test-md-nh");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let file_path = dir.join("notes.md");
        tokio::fs::write(&file_path, "Just some text without headings.")
            .await
            .unwrap();

        let connector = MarkdownConnector::new();
        let doc = connector
            .fetch(file_path.to_str().unwrap())
            .await
            .expect("should read markdown");

        // Falls back to filename.
        assert_eq!(doc.title, "notes");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn test_extract_markdown_title() {
        assert_eq!(
            extract_markdown_title("# My Title\n\nContent"),
            Some("My Title".to_string())
        );
        assert_eq!(extract_markdown_title("## Not H1\n\nContent"), None);
        assert_eq!(extract_markdown_title("No headings at all"), None);
        assert_eq!(
            extract_markdown_title("  # Indented Title  \n\nContent"),
            Some("Indented Title".to_string())
        );
    }

    #[tokio::test]
    async fn test_markdown_connector_missing_file() {
        let connector = MarkdownConnector::new();
        let result = connector.fetch("/nonexistent/doc.md").await;
        assert!(result.is_err());
    }
}
