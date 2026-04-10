//! Shared hashing helpers for journal capture and conflict detection.

use sha2::{Digest, Sha256};

/// Compute a SHA-256 digest and return it as lowercase hex.
pub(crate) fn compute_sha256_hex(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::compute_sha256_hex;

    #[test]
    fn test_compute_sha256_hex_matches_known_digest() {
        let digest = compute_sha256_hex(b"hello world");
        assert_eq!(
            digest,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
