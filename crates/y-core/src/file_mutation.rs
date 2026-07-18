//! Shared contracts for capability-declared file mutations.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::SessionId;

/// Hash value used when a mutation expects a path not to exist yet.
pub const ABSENT_CONTENT_HASH: &str = "absent";

/// Semantic operation declared by a file-mutating tool or recorded after execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileMutationOperation {
    /// The tool may create a missing file or modify an existing file.
    CreateOrModify,
    Create,
    Modify,
    Delete,
    Move,
}

/// Filesystem mutation metadata declared by a tool definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMutationCapability {
    pub operation: FileMutationOperation,
    pub path_argument: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_path_argument: Option<String>,
}

impl FileMutationCapability {
    pub fn new(operation: FileMutationOperation, path_argument: impl Into<String>) -> Self {
        Self {
            operation,
            path_argument: path_argument.into(),
            destination_path_argument: None,
        }
    }

    #[must_use]
    pub fn with_destination_argument(mut self, argument: impl Into<String>) -> Self {
        self.destination_path_argument = Some(argument.into());
        self
    }
}

/// Auditable result of one successful, capability-declared filesystem mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMutationEvent {
    pub tool_call_id: String,
    pub session_id: SessionId,
    pub agent_id: String,
    pub operation: FileMutationOperation,
    pub absolute_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_content_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content_ref: Option<String>,
    pub is_new_file: bool,
}

/// Incremental SHA-256 content hasher shared by tools and service capture.
pub struct ContentHasher(Sha256);

impl ContentHasher {
    pub fn new() -> Self {
        Self(Sha256::new())
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> String {
        format_hash(self.0.finalize())
    }
}

impl Default for ContentHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Return a stable SHA-256 identifier for raw file bytes.
pub fn content_hash(content: &[u8]) -> String {
    format_hash(Sha256::digest(content))
}

/// Convert a content hash into a content-addressed reference without embedding data.
pub fn content_ref(hash: &str) -> String {
    hash.strip_prefix("sha256:").map_or_else(
        || format!("cas:{hash}"),
        |digest| format!("cas:sha256:{digest}"),
    )
}

fn format_hash(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut output = String::with_capacity("sha256:".len() + digest.len() * 2);
    output.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_and_prefixed() {
        assert_eq!(
            content_hash(b"hello world"),
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn incremental_hash_matches_one_shot_hash() {
        let mut hasher = ContentHasher::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        assert_eq!(hasher.finish(), content_hash(b"hello world"));
    }
}
