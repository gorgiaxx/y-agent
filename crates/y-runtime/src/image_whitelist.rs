//! Image whitelist with digest verification.
//!
//! Design reference: runtime-design.md §Image Whitelist
//!
//! Provides a structured whitelist that goes beyond the simple `HashSet<String>`
//! in `RuntimeConfig`. Each entry specifies:
//! - Image name and allowed tags
//! - Expected digest (for tamper detection)
//! - Whether the image can be pulled from a registry

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single entry in the image whitelist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhitelistEntry {
    /// Image name (e.g., `python`, `node`).
    pub image: String,

    /// Allowed tags (e.g., `["3.11", "3.12-slim"]`).
    /// Empty means all tags are allowed.
    #[serde(default)]
    pub allowed_tags: Vec<String>,

    /// Expected image digest (`sha256:...`).
    /// If set, the image digest is verified before execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_digest: Option<String>,

    /// Whether pulling this image from a registry is allowed.
    #[serde(default)]
    pub allow_pull: bool,
}

/// Manages the image whitelist and performs verification.
#[derive(Debug, Clone)]
pub struct ImageWhitelist {
    /// Map from image name to whitelist entry.
    entries: HashMap<String, WhitelistEntry>,
}

/// Result of an image verification check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyResult {
    /// Image is allowed and passes all checks.
    Allowed,
    /// Image is not in the whitelist.
    NotWhitelisted { image: String },
    /// Image tag is not in the allowed tags list.
    TagNotAllowed { image: String, tag: String },
    /// Image digest does not match the expected value.
    DigestMismatch {
        image: String,
        expected: String,
        actual: String,
    },
}

impl ImageWhitelist {
    /// Create an empty whitelist (denies all images).
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Create a whitelist from a list of entries.
    pub fn from_entries(entries: Vec<WhitelistEntry>) -> Self {
        let map = entries.into_iter().map(|e| (e.image.clone(), e)).collect();
        Self { entries: map }
    }

    /// Add an entry to the whitelist.
    pub fn add_entry(&mut self, entry: WhitelistEntry) {
        self.entries.insert(entry.image.clone(), entry);
    }

    /// Check if an image reference (name:tag) is whitelisted.
    ///
    /// Parses the image reference into name and tag components,
    /// then verifies against the whitelist.
    pub fn is_allowed(&self, image_ref: &str) -> bool {
        let (name, tag) = parse_image_ref(image_ref);
        matches!(self.verify(name, tag, None), VerifyResult::Allowed)
    }

    /// Perform full verification of an image.
    ///
    /// Checks:
    /// 1. Image name is in the whitelist
    /// 2. Tag is in the allowed tags (if restriction is set)
    /// 3. Digest matches expected (if digest provided)
    pub fn verify(&self, name: &str, tag: &str, actual_digest: Option<&str>) -> VerifyResult {
        let Some(entry) = self.entries.get(name) else {
            return VerifyResult::NotWhitelisted {
                image: name.to_string(),
            };
        };

        // Check tag restrictions.
        if !entry.allowed_tags.is_empty() && !entry.allowed_tags.iter().any(|t| t == tag) {
            return VerifyResult::TagNotAllowed {
                image: name.to_string(),
                tag: tag.to_string(),
            };
        }

        // Check digest if both expected and actual are available.
        if let (Some(expected), Some(actual)) = (&entry.expected_digest, actual_digest) {
            if expected != actual {
                return VerifyResult::DigestMismatch {
                    image: name.to_string(),
                    expected: expected.clone(),
                    actual: actual.to_string(),
                };
            }
        }

        VerifyResult::Allowed
    }

    /// Check if pulling is allowed for this image.
    pub fn can_pull(&self, image_ref: &str) -> bool {
        let (name, _) = parse_image_ref(image_ref);
        self.entries.get(name).is_some_and(|e| e.allow_pull)
    }

    /// Number of entries in the whitelist.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the whitelist is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ImageWhitelist {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse an image reference into (name, tag).
///
/// Examples:
/// - `"python:3.11"` → `("python", "3.11")`
/// - `"python"` → `("python", "latest")`
/// - `"registry.io/org/image:v1"` → `("registry.io/org/image", "v1")`
fn parse_image_ref(image_ref: &str) -> (&str, &str) {
    // Handle digest references (name@sha256:...)
    if let Some(at_pos) = image_ref.rfind('@') {
        return (&image_ref[..at_pos], "latest");
    }

    // Handle tag references (name:tag)
    // Be careful with registry port numbers (e.g., registry.io:5000/image:tag)
    if let Some(colon_pos) = image_ref.rfind(':') {
        // Check if the colon is part of a tag (after the last /).
        let last_slash = image_ref.rfind('/').unwrap_or(0);
        if colon_pos > last_slash {
            return (&image_ref[..colon_pos], &image_ref[colon_pos + 1..]);
        }
    }

    (image_ref, "latest")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_whitelist() -> ImageWhitelist {
        ImageWhitelist::from_entries(vec![
            WhitelistEntry {
                image: "python".into(),
                allowed_tags: vec!["3.11".into(), "3.12-slim".into()],
                expected_digest: None,
                allow_pull: true,
            },
            WhitelistEntry {
                image: "node".into(),
                allowed_tags: vec![], // all tags
                expected_digest: Some("sha256:abcdef1234567890".into()),
                allow_pull: false,
            },
            WhitelistEntry {
                image: "alpine".into(),
                allowed_tags: vec!["3.19".into()],
                expected_digest: None,
                allow_pull: true,
            },
        ])
    }

    // -----------------------------------------------------------------------
    // parse_image_ref tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_image_ref_with_tag() {
        let (name, tag) = parse_image_ref("python:3.11");
        assert_eq!(name, "python");
        assert_eq!(tag, "3.11");
    }

    #[test]
    fn test_parse_image_ref_without_tag() {
        let (name, tag) = parse_image_ref("python");
        assert_eq!(name, "python");
        assert_eq!(tag, "latest");
    }

    #[test]
    fn test_parse_image_ref_with_registry() {
        let (name, tag) = parse_image_ref("registry.io/org/image:v1");
        assert_eq!(name, "registry.io/org/image");
        assert_eq!(tag, "v1");
    }

    #[test]
    fn test_parse_image_ref_with_digest() {
        let (name, tag) = parse_image_ref("python@sha256:abc123");
        assert_eq!(name, "python");
        assert_eq!(tag, "latest"); // digest refs default to "latest" tag
    }

    #[test]
    fn test_parse_image_ref_registry_with_port() {
        let (name, tag) = parse_image_ref("registry.io:5000/image:v2");
        assert_eq!(name, "registry.io:5000/image");
        assert_eq!(tag, "v2");
    }

    // -----------------------------------------------------------------------
    // Whitelist tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_allowed_image_and_tag() {
        let wl = sample_whitelist();
        assert!(wl.is_allowed("python:3.11"));
        assert!(wl.is_allowed("python:3.12-slim"));
    }

    #[test]
    fn test_disallowed_tag() {
        let wl = sample_whitelist();
        assert!(!wl.is_allowed("python:3.10")); // tag not in list
    }

    #[test]
    fn test_not_whitelisted() {
        let wl = sample_whitelist();
        assert!(!wl.is_allowed("evil:latest"));
    }

    #[test]
    fn test_all_tags_allowed() {
        let wl = sample_whitelist();
        // node has empty allowed_tags = all tags allowed.
        assert!(wl.is_allowed("node:18"));
        assert!(wl.is_allowed("node:latest"));
        assert!(wl.is_allowed("node:20-slim"));
    }

    #[test]
    fn test_digest_match() {
        let wl = sample_whitelist();
        let result = wl.verify("node", "latest", Some("sha256:abcdef1234567890"));
        assert_eq!(result, VerifyResult::Allowed);
    }

    #[test]
    fn test_digest_mismatch() {
        let wl = sample_whitelist();
        let result = wl.verify("node", "latest", Some("sha256:tampered"));
        assert_eq!(
            result,
            VerifyResult::DigestMismatch {
                image: "node".into(),
                expected: "sha256:abcdef1234567890".into(),
                actual: "sha256:tampered".into(),
            }
        );
    }

    #[test]
    fn test_digest_not_checked_when_none() {
        let wl = sample_whitelist();
        // node has an expected digest, but if no actual digest is provided,
        // we can't verify — so it passes (digest check is optional).
        let result = wl.verify("node", "latest", None);
        assert_eq!(result, VerifyResult::Allowed);
    }

    #[test]
    fn test_can_pull() {
        let wl = sample_whitelist();
        assert!(wl.can_pull("python:3.11")); // allow_pull = true
        assert!(!wl.can_pull("node:18")); // allow_pull = false
        assert!(!wl.can_pull("unknown:latest")); // not in whitelist
    }

    #[test]
    fn test_empty_whitelist_denies_all() {
        let wl = ImageWhitelist::new();
        assert!(!wl.is_allowed("python:3.11"));
        assert!(wl.is_empty());
    }

    #[test]
    fn test_add_entry() {
        let mut wl = ImageWhitelist::new();
        assert!(wl.is_empty());

        wl.add_entry(WhitelistEntry {
            image: "go".into(),
            allowed_tags: vec!["1.22".into()],
            expected_digest: None,
            allow_pull: true,
        });

        assert_eq!(wl.len(), 1);
        assert!(wl.is_allowed("go:1.22"));
        assert!(!wl.is_allowed("go:1.21"));
    }

    #[test]
    fn test_verify_not_whitelisted() {
        let wl = sample_whitelist();
        let result = wl.verify("evil", "latest", None);
        assert_eq!(
            result,
            VerifyResult::NotWhitelisted {
                image: "evil".into()
            }
        );
    }

    #[test]
    fn test_verify_tag_not_allowed() {
        let wl = sample_whitelist();
        let result = wl.verify("python", "3.10", None);
        assert_eq!(
            result,
            VerifyResult::TagNotAllowed {
                image: "python".into(),
                tag: "3.10".into(),
            }
        );
    }

    #[test]
    fn test_whitelist_serialization() {
        let entry = WhitelistEntry {
            image: "python".into(),
            allowed_tags: vec!["3.11".into()],
            expected_digest: Some("sha256:abc".into()),
            allow_pull: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: WhitelistEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.image, "python");
        assert_eq!(deserialized.expected_digest, Some("sha256:abc".into()));
    }
}
