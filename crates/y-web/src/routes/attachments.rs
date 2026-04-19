//! Attachment endpoints -- file reading and upload for chat attachments.
//!
//! Mirrors the GUI `attachment_read_files` command and adds multipart upload.

use std::path::{Path, PathBuf};

use axum::extract::Multipart;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum file size: 20 MB.
const MAX_FILE_SIZE: u64 = 20 * 1024 * 1024;

/// Allowed image extensions.
const ALLOWED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReadFilesRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AttachmentData {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub base64_data: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub path: String,
    pub filename: String,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mime_from_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/attachments/read`
async fn read_files(Json(body): Json<ReadFilesRequest>) -> Result<impl IntoResponse, ApiError> {
    let mut results = Vec::with_capacity(body.paths.len());

    for file_path in &body.paths {
        let path = Path::new(file_path);

        if !path.exists() {
            return Err(ApiError::NotFound(format!("File not found: {file_path}")));
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();

        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "Unsupported file type: .{ext}"
            )));
        }

        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to read metadata: {e}")))?;

        if metadata.len() > MAX_FILE_SIZE {
            return Err(ApiError::BadRequest(format!(
                "File exceeds 20 MB limit: {file_path}"
            )));
        }

        let data = tokio::fs::read(path)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to read file: {e}")))?;

        let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        results.push(AttachmentData {
            id: uuid::Uuid::new_v4().to_string(),
            filename,
            mime_type: mime_from_ext(&ext).to_string(),
            base64_data,
            size: metadata.len(),
        });
    }

    Ok(Json(results))
}

/// `POST /api/v1/attachments/upload` -- multipart file upload.
///
/// Accepts multipart form data with one or more file fields.
/// Saves files to a temporary directory and returns server-side paths.
async fn upload_files(mut multipart: Multipart) -> Result<impl IntoResponse, ApiError> {
    let mut results = Vec::new();

    let temp_dir = std::env::temp_dir().join("y-agent-uploads");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create upload dir: {e}")))?;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("Multipart error: {e}")))?
    {
        let filename = field
            .file_name()
            .map(String::from)
            .unwrap_or_else(|| format!("upload-{}", uuid::Uuid::new_v4()));

        let ext = Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();

        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "Unsupported file type: .{ext}"
            )));
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::BadRequest(format!("Failed to read field: {e}")))?;

        if data.len() as u64 > MAX_FILE_SIZE {
            return Err(ApiError::BadRequest(format!(
                "File exceeds 20 MB limit: {filename}"
            )));
        }

        let file_path = temp_dir.join(&filename);
        tokio::fs::write(&file_path, &data)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to write file: {e}")))?;

        results.push(UploadResponse {
            path: file_path.to_string_lossy().to_string(),
            filename,
            size: data.len() as u64,
        });
    }

    Ok(Json(results))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Attachments route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/attachments/read", post(read_files))
        .route("/api/v1/attachments/upload", post(upload_files))
}
