//! Text file source connector.
//!
//! Reads plain text files from the local filesystem.

use super::{file_document, file_stem_title, RawDocument, SourceConnector};
use crate::error::KnowledgeError;
use crate::models::SourceType;
use async_trait::async_trait;

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
        let (content, detected_encoding) = super::encoding::read_file_as_utf8(uri).await?;

        if detected_encoding != "UTF-8" {
            tracing::info!(
                uri,
                encoding = detected_encoding,
                "text file was auto-converted from {detected_encoding} to UTF-8"
            );
        }

        Ok(file_document(uri, content, file_stem_title(uri)))
    }

    fn source_type(&self) -> SourceType {
        SourceType::File
    }
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
}
