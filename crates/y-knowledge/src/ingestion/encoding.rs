//! Encoding detection and conversion utilities.
//!
//! Reads raw bytes from a file, detects the character encoding using
//! `chardetng`, and converts to UTF-8 using `encoding_rs` if needed.

use crate::error::KnowledgeError;
use chardetng::EncodingDetector;
use encoding_rs::Encoding;

/// Read a file and return its content as a UTF-8 string.
///
/// If the file is already valid UTF-8, it is returned directly.
/// Otherwise, the encoding is detected (supports Big5, GBK/GB18030,
/// `Shift_JIS`, EUC-KR, ISO-8859-*, Windows-1252, and more) and the
/// content is converted to UTF-8.
///
/// Returns the UTF-8 content and the name of the detected encoding.
pub async fn read_file_as_utf8(path: &str) -> Result<(String, &'static str), KnowledgeError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| KnowledgeError::IngestionError {
            message: format!("failed to read file '{path}': {e}"),
        })?;

    decode_bytes_as_utf8(&bytes, path)
}

/// Decode raw bytes into a UTF-8 string with automatic encoding detection.
///
/// `source_hint` is used only for error messages to identify the source.
pub fn decode_bytes_as_utf8(
    bytes: &[u8],
    source_hint: &str,
) -> Result<(String, &'static str), KnowledgeError> {
    // Fast path: if it's already valid UTF-8, return directly.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok((s.to_string(), "UTF-8"));
    }

    // Detect encoding using chardetng.
    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let encoding = detector.guess(None, true);

    Ok(decode_with_encoding(bytes, encoding, source_hint))
}

/// Decode bytes using a specific encoding, returning UTF-8 string.
fn decode_with_encoding(
    bytes: &[u8],
    encoding: &'static Encoding,
    source_hint: &str,
) -> (String, &'static str) {
    let (cow, actual_encoding, had_errors) = encoding.decode(bytes);

    if had_errors {
        tracing::warn!(
            source = source_hint,
            encoding = actual_encoding.name(),
            "encoding conversion had replacement characters"
        );
    }

    tracing::info!(
        source = source_hint,
        encoding = actual_encoding.name(),
        "converted file from {} to UTF-8",
        actual_encoding.name()
    );

    (cow.into_owned(), actual_encoding.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_passthrough() {
        let content = "Hello, world! 你好世界";
        let bytes = content.as_bytes();
        let (result, encoding) = decode_bytes_as_utf8(bytes, "test").unwrap();
        assert_eq!(result, content);
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn test_big5_detection() {
        // "你好世界" in Big5 encoding
        let big5 = encoding_rs::BIG5;
        let (encoded, _, _) = big5.encode("你好世界，這是一段繁體中文測試文字。");
        let (result, encoding) = decode_bytes_as_utf8(&encoded, "test.txt").unwrap();
        assert!(
            result.contains("你好世界"),
            "should contain decoded Chinese, got: {result}"
        );
        // chardetng may report Big5 or a related encoding name
        assert!(
            encoding == "Big5" || encoding == "big5",
            "expected Big5 encoding, got: {encoding}"
        );
    }

    #[test]
    fn test_gbk_detection() {
        // "你好世界" in GBK encoding (encoding_rs uses gb18030 which is a superset of GBK/GB2312)
        let gbk = encoding_rs::GBK;
        let (encoded, _, _) = gbk.encode("你好世界，这是一段简体中文测试文字。");
        let (result, encoding) = decode_bytes_as_utf8(&encoded, "test.txt").unwrap();
        assert!(
            result.contains("你好世界"),
            "should contain decoded Chinese, got: {result}"
        );
        // chardetng reports GBK-family as "GBK" or "gb18030"
        assert!(
            encoding == "GBK" || encoding == "gb18030" || encoding == "GB18030",
            "expected GBK/GB18030 encoding, got: {encoding}"
        );
    }

    #[test]
    fn test_gb2312_detection() {
        // GB2312 is a subset of GBK, encoding_rs handles it via GBK
        let gbk = encoding_rs::GBK;
        let (encoded, _, _) = gbk.encode("这是一段中文内容，用于测试编码检测功能。");
        let (result, _encoding) = decode_bytes_as_utf8(&encoded, "test.txt").unwrap();
        assert!(
            result.contains("这是一段中文内容"),
            "should contain decoded Chinese, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_read_file_utf8() {
        let dir = std::env::temp_dir().join("y-knowledge-test-encoding");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("utf8.txt");
        tokio::fs::write(&path, "Hello, UTF-8! 你好").await.unwrap();

        let (content, encoding) = read_file_as_utf8(path.to_str().unwrap()).await.unwrap();
        assert_eq!(content, "Hello, UTF-8! 你好");
        assert_eq!(encoding, "UTF-8");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_read_file_big5() {
        let dir = std::env::temp_dir().join("y-knowledge-test-encoding-big5");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("big5.txt");

        // Write Big5-encoded content
        let big5 = encoding_rs::BIG5;
        let (encoded, _, _) =
            big5.encode("這是繁體中文測試文件的內容。包含多個句子用於測試編碼偵測。");
        tokio::fs::write(&path, &*encoded).await.unwrap();

        let (content, encoding) = read_file_as_utf8(path.to_str().unwrap()).await.unwrap();
        assert!(content.contains("繁體中文"), "got: {content}");
        assert!(
            encoding == "Big5" || encoding == "big5",
            "expected Big5, got: {encoding}"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_read_file_missing() {
        let result = read_file_as_utf8("/nonexistent/file.txt").await;
        assert!(result.is_err());
    }
}
