//! Attachment command handlers -- read files and return base64-encoded data.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use uuid::Uuid;

/// Supported image MIME types and their file extensions.
const IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];

/// Maximum file size allowed (20 MB).
const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Attachment data returned to the frontend.
#[derive(Debug, Serialize, Clone)]
pub struct AttachmentData {
    /// Unique identifier.
    pub id: String,
    /// Original filename.
    pub filename: String,
    /// MIME type (e.g. `image/png`).
    pub mime_type: String,
    /// Base64-encoded file contents.
    pub base64_data: String,
    /// File size in bytes.
    pub size: u64,
}

/// Resolve MIME type from a file extension.
fn mime_from_extension(ext: &str) -> Option<&'static str> {
    let lower = ext.to_ascii_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .find(|(e, _)| *e == lower)
        .map(|(_, mime)| *mime)
}

/// Read image files from disk and return base64-encoded attachment data.
///
/// Validates that each file exists, is an image, and is within the size limit.
#[tauri::command]
pub async fn attachment_read_files(paths: Vec<String>) -> Result<Vec<AttachmentData>, String> {
    let mut results = Vec::with_capacity(paths.len());

    for path_str in &paths {
        let path = std::path::Path::new(path_str);

        // Validate existence.
        if !path.exists() {
            return Err(format!("File not found: {path_str}"));
        }

        // Extract and validate extension.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let mime = mime_from_extension(ext).ok_or_else(|| {
            format!("Unsupported file type: .{ext}. Supported: png, jpg, jpeg, gif, webp")
        })?;

        // Validate file size.
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("Failed to read file metadata: {e}"))?;

        if metadata.len() > MAX_FILE_SIZE {
            return Err(format!(
                "File too large: {} ({} bytes, max {} bytes)",
                path_str,
                metadata.len(),
                MAX_FILE_SIZE,
            ));
        }

        // Read and encode.
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("Failed to read file: {e}"))?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        results.push(AttachmentData {
            id: Uuid::new_v4().to_string(),
            filename,
            mime_type: mime.to_string(),
            base64_data: STANDARD.encode(&bytes),
            size: metadata.len(),
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn read_valid_image_file() {
        // Create a temp PNG file (minimal valid PNG header).
        let mut tmp = tempfile::Builder::new()
            .suffix(".png")
            .tempfile()
            .expect("create temp file");
        let png_header: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        tmp.write_all(png_header).expect("write png");
        tmp.flush().expect("flush");

        let path = tmp.path().to_str().unwrap().to_string();
        let result = attachment_read_files(vec![path]).await;

        assert!(result.is_ok());
        let attachments = result.unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime_type, "image/png");
        assert_eq!(attachments[0].size, png_header.len() as u64);
        assert!(!attachments[0].base64_data.is_empty());
    }

    #[tokio::test]
    async fn reject_unsupported_extension() {
        let tmp = tempfile::Builder::new()
            .suffix(".txt")
            .tempfile()
            .expect("create temp file");
        let path = tmp.path().to_str().unwrap().to_string();
        let result = attachment_read_files(vec![path]).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported file type"));
    }

    #[tokio::test]
    async fn reject_nonexistent_file() {
        let result = attachment_read_files(vec!["/nonexistent/file.png".to_string()]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("File not found"));
    }
}
