//! Helpers for provider-level custom HTTP headers.

use std::collections::HashMap;
use std::hash::BuildHasher;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::RequestBuilder;

/// Build a validated header map from user-provided provider configuration.
pub fn custom_header_map<S: BuildHasher>(
    headers: &HashMap<String, String, S>,
) -> Result<HeaderMap, String> {
    let mut header_map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid custom header name '{name}'"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|_| format!("invalid value for custom header '{name}'"))?;
        header_map.insert(header_name, header_value);
    }
    Ok(header_map)
}

/// Apply custom headers to a reqwest request builder.
pub fn apply_custom_headers(
    mut request_builder: RequestBuilder,
    headers: &HeaderMap,
) -> RequestBuilder {
    for (name, value) in headers {
        request_builder = request_builder.header(name, value);
    }
    request_builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_custom_header_map_accepts_valid_headers() {
        let headers = HashMap::from([("X-LLM-Tenant".to_string(), "workspace-a".to_string())]);

        let header_map = custom_header_map(&headers).expect("valid headers");

        assert_eq!(
            header_map
                .get("x-llm-tenant")
                .and_then(|value| value.to_str().ok()),
            Some("workspace-a")
        );
    }

    #[test]
    fn test_custom_header_map_rejects_invalid_header_names() {
        let headers = HashMap::from([("Bad Header".to_string(), "value".to_string())]);

        let err = custom_header_map(&headers).expect_err("invalid header should fail");

        assert!(err.contains("Bad Header"));
    }
}
