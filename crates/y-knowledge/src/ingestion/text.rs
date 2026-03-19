//! Text file source connector.
//!
//! Reads plain text files from the local filesystem.

use super::{RawDocument, SourceConnector};
use crate::error::KnowledgeError;
use crate::models::SourceType;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// Connector for plain text files.
#[derive(Debug, Default)]
pub struct TextConnector;

impl TextConnector {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SourceConnector for TextConnector {
    async fn fetch(&self, uri: &str) -> Result<RawDocument, KnowledgeError> {
        let (content, detected_encoding) =
            super::encoding::read_file_as_utf8(uri).await?;

        if detected_encoding != "UTF-8" {
            tracing::info!(
                uri,
                encoding = detected_encoding,
                "text file was auto-converted from {detected_encoding} to UTF-8"
            );
        }

        let content_hash = hex_sha256(&content);

        // Extract title from filename.
        let title = std::path::Path::new(uri)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

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

/// Compute hex-encoded SHA-256 hash of a string.
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
    async fn test_text_connector_reads_file() {
        // Create a temp file.
        let dir = std::env::temp_dir().join("y-knowledge-test-text");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let file_path = dir.join("sample.txt");
        tokio::fs::write(&file_path, "Hello, knowledge base!")
            .await
            .unwrap();

        let connector = TextConnector::new();
        let doc = connector
            .fetch(file_path.to_str().unwrap())
            .await
            .expect("should read file");

        assert_eq!(doc.content, "Hello, knowledge base!");
        assert_eq!(doc.title, "sample");
        assert!(!doc.content_hash.is_empty());
        assert_eq!(doc.source_type, SourceType::File);

        // Cleanup.
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_text_connector_missing_file() {
        let connector = TextConnector::new();
        let result = connector.fetch("/nonexistent/file.txt").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_hex_sha256_deterministic() {
        let hash1 = hex_sha256("hello");
        let hash2 = hex_sha256("hello");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars

        let hash3 = hex_sha256("world");
        assert_ne!(hash1, hash3);
    }
}
