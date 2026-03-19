//! Security policy for browser automation.
//!
//! Provides domain allowlist filtering and SSRF protection.

use url::Url;

/// Security policy controlling which URLs the browser can navigate to.
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Normalized allowed domains. `["*"]` means all public domains.
    allowed_domains: Vec<String>,
    /// Whether to block private/local network addresses.
    block_private_networks: bool,
}

impl SecurityPolicy {
    /// Create a new security policy.
    pub fn new(allowed_domains: Vec<String>, block_private_networks: bool) -> Self {
        let normalized = normalize_domains(allowed_domains);
        Self {
            allowed_domains: normalized,
            block_private_networks,
        }
    }

    /// Validate a URL against the security policy.
    pub fn validate_url(&self, url: &str) -> Result<(), SecurityError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(SecurityError::EmptyUrl);
        }

        // Block file:// URLs — can exfiltrate local files.
        if url.starts_with("file://") {
            return Err(SecurityError::BlockedScheme("file".into()));
        }

        // Only allow http(s).
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(SecurityError::BlockedScheme(
                url.split("://").next().unwrap_or("unknown").into(),
            ));
        }

        let parsed = Url::parse(url).map_err(|e| SecurityError::InvalidUrl(e.to_string()))?;

        let host = parsed
            .host_str()
            .ok_or_else(|| SecurityError::InvalidUrl("missing host".into()))?;

        // Block private networks if configured.
        if self.block_private_networks && is_private_host(host) {
            return Err(SecurityError::PrivateNetwork(host.into()));
        }

        // Check domain allowlist.
        if !self.is_domain_allowed(host) {
            return Err(SecurityError::DomainNotAllowed(host.into()));
        }

        Ok(())
    }

    fn is_domain_allowed(&self, host: &str) -> bool {
        if self.allowed_domains.is_empty() {
            return false;
        }
        if self.allowed_domains.iter().any(|d| d == "*") {
            return true;
        }
        let host_lower = host.to_lowercase();
        self.allowed_domains.iter().any(|domain| {
            host_lower == *domain
                || host_lower
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    }
}

/// Security policy errors.
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("URL cannot be empty")]
    EmptyUrl,

    #[error("blocked URL scheme: {0}")]
    BlockedScheme(String),

    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    #[error("blocked private/local network host: {0}")]
    PrivateNetwork(String),

    #[error("domain '{0}' not in allowed_domains")]
    DomainNotAllowed(String),
}

/// Normalize domain entries: lowercase, strip scheme/path/port.
fn normalize_domains(domains: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = domains
        .into_iter()
        .filter_map(|d| {
            let mut d = d.trim().to_lowercase();
            if d.is_empty() {
                return None;
            }
            // Special wildcard.
            if d == "*" {
                return Some(d);
            }
            // Strip scheme.
            if let Some(rest) = d.strip_prefix("https://") {
                d = rest.to_string();
            } else if let Some(rest) = d.strip_prefix("http://") {
                d = rest.to_string();
            }
            // Strip path.
            if let Some((host, _)) = d.split_once('/') {
                d = host.to_string();
            }
            // Strip port.
            if let Some((host, _)) = d.split_once(':') {
                d = host.to_string();
            }
            d = d.trim_start_matches('.').trim_end_matches('.').to_string();
            if d.is_empty() {
                return None;
            }
            Some(d)
        })
        .collect();
    result.sort_unstable();
    result.dedup();
    result
}

/// Check if a hostname resolves to a private/local address.
fn is_private_host(host: &str) -> bool {
    let h = host.to_lowercase();

    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if h == "localhost" || h.ends_with(".localhost") || h == "::1" || h.ends_with(".local") {
        return true;
    }

    // Check IPv4 private ranges.
    if let Some(octets) = parse_ipv4(&h) {
        let [a, b, _, _] = octets;
        return a == 0
            || a == 10
            || a == 127
            || (a == 169 && b == 254)
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || (a == 100 && (64..=127).contains(&b));
    }

    false
}

fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (i, part) in parts.iter().enumerate() {
        octets[i] = part.parse::<u8>().ok()?;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(domains: &[&str]) -> SecurityPolicy {
        SecurityPolicy::new(domains.iter().map(|s| s.to_string()).collect(), true)
    }

    #[test]
    fn allows_wildcard() {
        let p = policy(&["*"]);
        assert!(p.validate_url("https://example.com").is_ok());
    }

    #[test]
    fn blocks_private_with_wildcard() {
        let p = policy(&["*"]);
        assert!(p.validate_url("https://localhost:8080").is_err());
        assert!(p.validate_url("https://192.168.1.1").is_err());
        assert!(p.validate_url("https://10.0.0.1").is_err());
    }

    #[test]
    fn blocks_file_scheme() {
        let p = policy(&["*"]);
        assert!(p.validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn allows_exact_domain() {
        let p = policy(&["example.com"]);
        assert!(p.validate_url("https://example.com/path").is_ok());
    }

    #[test]
    fn allows_subdomain() {
        let p = policy(&["example.com"]);
        assert!(p.validate_url("https://api.example.com/v1").is_ok());
    }

    #[test]
    fn blocks_unlisted_domain() {
        let p = policy(&["example.com"]);
        assert!(p.validate_url("https://google.com").is_err());
    }

    #[test]
    fn blocks_empty_allowlist() {
        let p = policy(&[]);
        assert!(p.validate_url("https://example.com").is_err());
    }

    #[test]
    fn blocks_empty_url() {
        let p = policy(&["*"]);
        assert!(p.validate_url("").is_err());
    }
}
