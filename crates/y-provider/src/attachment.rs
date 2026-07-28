//! Materialize typed attachment references for provider wire adapters.

use base64::Engine as _;

const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

pub(crate) fn encoded_data(value: &serde_json::Value) -> Option<(String, String)> {
    let mime_type = value.get("mime_type")?.as_str()?.to_string();
    if let Some(data) = value.get("base64_data").and_then(serde_json::Value::as_str) {
        return Some((mime_type, data.to_string()));
    }
    let path = value.get("path")?.as_str()?;
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        tracing::warn!(
            path,
            size = metadata.len(),
            "provider skipped oversized attachment"
        );
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some((
        mime_type,
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

pub(crate) fn data_url(value: &serde_json::Value) -> Option<String> {
    let (mime_type, data) = encoded_data(value)?;
    Some(format!("data:{mime_type};base64,{data}"))
}

pub(crate) fn text_content(value: &serde_json::Value) -> Option<String> {
    let mime_type = value.get("mime_type")?.as_str()?;
    if !(mime_type.starts_with("text/") || mime_type == "application/json") {
        return None;
    }
    let (_, encoded) = encoded_data(value)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let filename = value
        .get("filename")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("attachment");
    Some(format!(
        "<attachment filename=\"{filename}\" mime=\"{mime_type}\">\n{text}\n</attachment>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_reference_is_encoded_only_at_provider_boundary() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"hello").unwrap();
        let value = serde_json::json!({
            "mime_type": "image/png",
            "path": file.path(),
        });

        assert_eq!(
            encoded_data(&value),
            Some(("image/png".into(), "aGVsbG8=".into()))
        );
        assert_eq!(
            data_url(&value).as_deref(),
            Some("data:image/png;base64,aGVsbG8=")
        );
    }

    #[test]
    fn text_attachment_is_materialized_as_bounded_prompt_content() {
        let value = serde_json::json!({
            "filename": "notes.txt",
            "mime_type": "text/plain",
            "base64_data": "aGVsbG8="
        });
        let content = text_content(&value).unwrap();
        assert!(content.contains("notes.txt"));
        assert!(content.contains("hello"));
    }
}
