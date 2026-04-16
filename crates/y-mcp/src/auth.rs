//! MCP authentication: token persistence for remote MCP servers.
//!
//! Provides file-based storage for bearer tokens and OAuth credentials.
//! Tokens are persisted to `mcp-auth.json` in the configured data directory
//! so they survive across restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::McpError;

/// Stored OAuth/bearer tokens for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthTokens {
    /// Bearer access token.
    pub access_token: String,
    /// Optional refresh token for token renewal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix timestamp (seconds) when the access token expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// OAuth scope granted by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl McpAuthTokens {
    /// Check whether the access token has expired.
    ///
    /// Returns `false` if no expiry is set (token assumed valid).
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires_at) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now >= expires_at
            }
            None => false,
        }
    }
}

/// File-based token store for MCP server authentication.
///
/// Tokens are stored as a JSON map keyed by server name at the configured
/// path (typically `~/.config/y-agent/mcp-auth.json`).
pub struct McpAuthStore {
    path: PathBuf,
}

impl McpAuthStore {
    /// Create a new auth store at the given file path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Create a new auth store in the given directory.
    ///
    /// The token file will be `{dir}/mcp-auth.json`.
    pub fn in_directory(dir: &Path) -> Self {
        Self {
            path: dir.join("mcp-auth.json"),
        }
    }

    /// Get the path to the auth store file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load all stored tokens from disk.
    pub fn load_all(&self) -> Result<HashMap<String, McpAuthTokens>, McpError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(&self.path).map_err(|e| McpError::Other {
            message: format!("failed to read auth store at {}: {e}", self.path.display()),
        })?;

        let tokens: HashMap<String, McpAuthTokens> =
            serde_json::from_str(&content).map_err(|e| McpError::Other {
                message: format!("failed to parse auth store: {e}"),
            })?;

        Ok(tokens)
    }

    /// Load tokens for a specific server.
    pub fn load(&self, server_name: &str) -> Result<Option<McpAuthTokens>, McpError> {
        let all = self.load_all()?;
        Ok(all.get(server_name).cloned())
    }

    /// Save tokens for a specific server.
    ///
    /// Merges with existing entries (other servers are preserved).
    pub fn save(&self, server_name: &str, tokens: McpAuthTokens) -> Result<(), McpError> {
        let mut all = self.load_all().unwrap_or_default();
        all.insert(server_name.to_string(), tokens);
        self.write_all(&all)
    }

    /// Remove tokens for a specific server.
    pub fn remove(&self, server_name: &str) -> Result<(), McpError> {
        let mut all = self.load_all().unwrap_or_default();
        all.remove(server_name);
        self.write_all(&all)
    }

    fn write_all(&self, tokens: &HashMap<String, McpAuthTokens>) -> Result<(), McpError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| McpError::Other {
                message: format!(
                    "failed to create auth store directory {}: {e}",
                    parent.display()
                ),
            })?;
        }

        let content = serde_json::to_string_pretty(tokens)?;
        std::fs::write(&self.path, content).map_err(|e| McpError::Other {
            message: format!("failed to write auth store at {}: {e}", self.path.display()),
        })?;

        // Restrict file permissions on Unix (0600 -- owner read/write only).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }

        debug!(path = %self.path.display(), "auth store updated");
        Ok(())
    }
}

/// Resolve a bearer token for an MCP server from environment or auth store.
///
/// Checks in order:
/// 1. Explicit `bearer_token` parameter (from config)
/// 2. Environment variable named `{SERVER_NAME}_TOKEN` (uppercased, hyphens to underscores)
/// 3. Tokens persisted in the auth store
///
/// Returns `None` if no token is found anywhere.
pub fn resolve_bearer_token(
    server_name: &str,
    explicit_token: Option<&str>,
    auth_store: Option<&McpAuthStore>,
) -> Option<String> {
    // 1. Explicit token from config.
    if let Some(token) = explicit_token {
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }

    // 2. Environment variable: {SERVER_NAME}_TOKEN.
    let env_key = format!(
        "{}_TOKEN",
        server_name.to_uppercase().replace(['-', '.'], "_")
    );
    if let Ok(token) = std::env::var(&env_key) {
        if !token.is_empty() {
            debug!(server = %server_name, env_var = %env_key, "using bearer token from env");
            return Some(token);
        }
    }

    // 3. Auth store.
    if let Some(store) = auth_store {
        match store.load(server_name) {
            Ok(Some(tokens)) => {
                if tokens.is_expired() {
                    warn!(
                        server = %server_name,
                        "stored token has expired; skipping"
                    );
                    return None;
                }
                return Some(tokens.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(
                    server = %server_name,
                    error = %e,
                    "failed to load tokens from auth store"
                );
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens_not_expired_when_no_expiry() {
        let tokens = McpAuthTokens {
            access_token: "test-token".into(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_tokens_expired() {
        let tokens = McpAuthTokens {
            access_token: "test-token".into(),
            refresh_token: None,
            expires_at: Some(1_000_000), // far in the past
            scope: None,
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn test_tokens_not_expired_future() {
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let tokens = McpAuthTokens {
            access_token: "test-token".into(),
            refresh_token: None,
            expires_at: Some(future),
            scope: None,
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_auth_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpAuthStore::in_directory(dir.path());

        // Initially empty.
        assert!(store.load("test-server").unwrap().is_none());

        // Save and load.
        let tokens = McpAuthTokens {
            access_token: "abc123".into(),
            refresh_token: Some("refresh-xyz".into()),
            expires_at: Some(9_999_999_999),
            scope: Some("read write".into()),
        };
        store.save("test-server", tokens.clone()).unwrap();

        let loaded = store.load("test-server").unwrap().unwrap();
        assert_eq!(loaded.access_token, "abc123");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-xyz"));
        assert_eq!(loaded.expires_at, Some(9_999_999_999));

        // Another server does not exist.
        assert!(store.load("other-server").unwrap().is_none());

        // Remove.
        store.remove("test-server").unwrap();
        assert!(store.load("test-server").unwrap().is_none());
    }

    #[test]
    fn test_auth_store_preserves_other_servers() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpAuthStore::in_directory(dir.path());

        let t1 = McpAuthTokens {
            access_token: "token-a".into(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        };
        let t2 = McpAuthTokens {
            access_token: "token-b".into(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        };

        store.save("server-a", t1).unwrap();
        store.save("server-b", t2).unwrap();

        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all["server-a"].access_token, "token-a");
        assert_eq!(all["server-b"].access_token, "token-b");
    }

    #[test]
    fn test_resolve_bearer_token_explicit() {
        let token = resolve_bearer_token("server", Some("explicit-token"), None);
        assert_eq!(token, Some("explicit-token".to_string()));
    }

    #[test]
    fn test_resolve_bearer_token_empty_explicit_skips() {
        let token = resolve_bearer_token("server", Some(""), None);
        assert!(token.is_none());
    }

    #[test]
    fn test_resolve_bearer_token_env_var() {
        let env_key = "TEST_MCP_AUTH_SERVER_TOKEN";
        std::env::set_var(env_key, "env-token-123");
        let token = resolve_bearer_token("test-mcp-auth-server", None, None);
        assert_eq!(token, Some("env-token-123".to_string()));
        std::env::remove_var(env_key);
    }

    #[test]
    fn test_resolve_bearer_token_from_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpAuthStore::in_directory(dir.path());
        store
            .save(
                "my-server",
                McpAuthTokens {
                    access_token: "stored-token".into(),
                    refresh_token: None,
                    expires_at: None,
                    scope: None,
                },
            )
            .unwrap();

        let token = resolve_bearer_token("my-server", None, Some(&store));
        assert_eq!(token, Some("stored-token".to_string()));
    }

    #[test]
    fn test_resolve_bearer_token_expired_store_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = McpAuthStore::in_directory(dir.path());
        store
            .save(
                "expired-server",
                McpAuthTokens {
                    access_token: "old-token".into(),
                    refresh_token: None,
                    expires_at: Some(1_000_000), // expired
                    scope: None,
                },
            )
            .unwrap();

        let token = resolve_bearer_token("expired-server", None, Some(&store));
        assert!(token.is_none());
    }

    #[test]
    fn test_tokens_serialization_roundtrip() {
        let tokens = McpAuthTokens {
            access_token: "abc".into(),
            refresh_token: Some("def".into()),
            expires_at: Some(12345),
            scope: Some("read".into()),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: McpAuthTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "abc");
        assert_eq!(parsed.refresh_token.as_deref(), Some("def"));
    }
}
