use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Service configuration for optional Language Server Protocol support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_max_message_bytes")]
    pub max_message_bytes: usize,
    #[serde(default = "default_max_restarts")]
    pub max_restarts: u32,
    #[serde(default = "default_restart_base_delay_ms")]
    pub restart_base_delay_ms: u64,
    #[serde(default = "default_servers")]
    pub servers: Vec<LspServerConfig>,
}

/// One configured language-server command and its project matching rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub language_id: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub initialization_options: serde_json::Value,
}

impl LspConfig {
    /// Select the server with the longest matching file extension.
    pub fn server_for_path(&self, path: &Path) -> Option<&LspServerConfig> {
        if !self.enabled {
            return None;
        }
        let file_name = path.file_name()?.to_string_lossy().to_lowercase();
        self.servers
            .iter()
            .filter_map(|server| {
                server
                    .extensions
                    .iter()
                    .filter_map(|extension| {
                        let normalized = extension.trim_start_matches('.').to_lowercase();
                        file_name
                            .ends_with(&format!(".{normalized}"))
                            .then_some(normalized.len())
                    })
                    .max()
                    .map(|length| (length, server))
            })
            .max_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| right.1.id.cmp(&left.1.id))
            })
            .map(|(_, server)| server)
    }
}

impl LspServerConfig {
    /// Resolve the nearest ancestor containing one of this server's markers.
    pub fn project_root(&self, path: &Path) -> Option<PathBuf> {
        let start = if path.is_dir() { path } else { path.parent()? };
        for directory in start.ancestors() {
            if self
                .root_markers
                .iter()
                .any(|marker| directory.join(marker).exists())
            {
                return Some(directory.to_path_buf());
            }
        }
        Some(start.to_path_buf())
    }

    /// Resolve the nearest project marker without walking above a trusted root.
    pub fn project_root_within(&self, path: &Path, trusted_root: &Path) -> Option<PathBuf> {
        let start = if path.is_dir() { path } else { path.parent()? };
        if !start.starts_with(trusted_root) {
            return None;
        }
        for directory in start.ancestors() {
            if !directory.starts_with(trusted_root) {
                break;
            }
            if self
                .root_markers
                .iter()
                .any(|marker| directory.join(marker).exists())
            {
                return Some(directory.to_path_buf());
            }
        }
        Some(trusted_root.to_path_buf())
    }
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            request_timeout_ms: default_request_timeout_ms(),
            max_message_bytes: default_max_message_bytes(),
            max_restarts: default_max_restarts(),
            restart_base_delay_ms: default_restart_base_delay_ms(),
            servers: default_servers(),
        }
    }
}

impl Default for LspServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            command: String::new(),
            args: Vec::new(),
            language_id: String::new(),
            extensions: Vec::new(),
            root_markers: Vec::new(),
            initialization_options: serde_json::Value::Null,
        }
    }
}

fn default_request_timeout_ms() -> u64 {
    15_000
}

fn default_max_message_bytes() -> usize {
    8 * 1024 * 1024
}

fn default_max_restarts() -> u32 {
    3
}

fn default_restart_base_delay_ms() -> u64 {
    250
}

fn default_servers() -> Vec<LspServerConfig> {
    vec![
        LspServerConfig {
            id: "rust".to_string(),
            command: "rust-analyzer".to_string(),
            args: Vec::new(),
            language_id: "rust".to_string(),
            extensions: vec!["rs".to_string()],
            root_markers: vec!["Cargo.toml".to_string(), "rust-project.json".to_string()],
            initialization_options: serde_json::Value::Null,
        },
        LspServerConfig {
            id: "typescript".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            language_id: "typescript".to_string(),
            extensions: vec![
                "ts".to_string(),
                "tsx".to_string(),
                "js".to_string(),
                "jsx".to_string(),
                "d.ts".to_string(),
            ],
            root_markers: vec![
                "tsconfig.json".to_string(),
                "jsconfig.json".to_string(),
                "package.json".to_string(),
            ],
            initialization_options: serde_json::Value::Null,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LspConfig, LspServerConfig};

    #[test]
    fn defaults_are_disabled_with_rust_and_typescript_templates() {
        let config = LspConfig::default();

        assert!(!config.enabled);
        assert!(config.servers.iter().any(|server| server.id == "rust"));
        assert!(config
            .servers
            .iter()
            .any(|server| server.id == "typescript"));
    }

    #[test]
    fn server_selection_uses_longest_matching_extension() {
        let config = LspConfig {
            enabled: true,
            servers: vec![
                LspServerConfig {
                    id: "javascript".to_string(),
                    extensions: vec!["js".to_string()],
                    ..LspServerConfig::default()
                },
                LspServerConfig {
                    id: "typescript".to_string(),
                    extensions: vec!["ts".to_string(), "d.ts".to_string()],
                    ..LspServerConfig::default()
                },
            ],
            ..LspConfig::default()
        };

        let selected = config
            .server_for_path(Path::new("/repo/src/generated.d.ts"))
            .expect("typescript server");

        assert_eq!(selected.id, "typescript");
    }

    #[test]
    fn project_root_uses_the_nearest_configured_marker() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let package = workspace.join("packages/app");
        let source = package.join("src");
        std::fs::create_dir_all(&source).expect("source dirs");
        std::fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("workspace marker");
        std::fs::write(package.join("Cargo.toml"), "[package]").expect("package marker");
        let file = source.join("lib.rs");
        std::fs::write(&file, "pub fn value() -> usize { 1 }").expect("source file");
        let server = LspServerConfig {
            root_markers: vec!["Cargo.toml".to_string()],
            ..LspServerConfig::default()
        };

        let root = server.project_root(&file).expect("project root");

        assert_eq!(root, package);
    }

    #[test]
    fn project_root_does_not_escape_the_trusted_workspace() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let parent = temp.path();
        let workspace = parent.join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).expect("source dirs");
        std::fs::write(parent.join("Cargo.toml"), "[workspace]").expect("parent marker");
        let file = source.join("lib.rs");
        std::fs::write(&file, "pub fn value() -> usize { 1 }").expect("source file");
        let server = LspServerConfig {
            root_markers: vec!["Cargo.toml".to_string()],
            ..LspServerConfig::default()
        };

        let root = server
            .project_root_within(&file, &workspace)
            .expect("bounded project root");

        assert_eq!(root, workspace);
    }
}
